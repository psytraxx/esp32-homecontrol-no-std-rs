#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use alloc::format;
use config::{
    AWAKE_DURATION_SECONDS, DEEP_SLEEP_DURATION_SECONDS, LOW_BATTERY_CUTOFF_MV,
    WIFI_CONNECT_TIMEOUT_SECONDS,
};
use display::{Display, DisplayPeripherals};
use domain::Sensor;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_net::Stack;
use embassy_time::{Delay, Duration, Instant, Timer, with_deadline, with_timeout};
use esp_alloc::{heap_allocator, psram_allocator};
use esp_backtrace as _;
use esp_hal::{
    Config,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig, Pin},
    peripherals::WIFI,
    rng::Rng,
    rtc_cntl::{SocResetReason, wakeup_cause},
    system::{SleepSource, reset_reason},
    timer::timg::TimerGroup,
};
use esp_println::logger::init_logger;
use esp_radio::wifi::WifiError;
use esp_rtos::main;
use log::{error, info, warn};
use pump::run_pump;
use rtc_memory::{boot_count as read_boot_count, set_boot_count};
use sensors::SensorPeripherals;
use sleep::enter_deep;
use wifi::{WIFI_SIGNAL, connect_to_wifi};

extern crate alloc;

mod config;
mod display;
mod domain;
mod mqtt;
mod pump;
mod rtc_memory;
mod sensors;
mod sleep;
mod wifi;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
async fn main(spawner: Spawner) {
    init_logger(log::LevelFilter::Info);

    // Why the last reset happened. A SysBrownOut / watchdog / CoreSw reason
    // (rather than CoreDeepSleep) means a previous cycle crashed instead of
    // sleeping cleanly — the key signal for diagnosing the brownout/reset loop
    // without a serial cable (also surfaced on the display on button wake).
    let reset_reason = reset_reason();
    info!("Reset reason: {:?}", reset_reason);

    let boot_count = read_boot_count();
    info!("Current boot count = {}", boot_count);
    set_boot_count(boot_count + 1);

    let peripherals = esp_hal::init(Config::default().with_cpu_clock(CpuClock::_80MHz));

    heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 73744);

    psram_allocator!(peripherals.PSRAM, esp_hal::psram);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // GPIO15 must be HIGH for the display to receive power (even when display is unused).
    let mut power_pin = Output::new(peripherals.GPIO15, Level::Low, OutputConfig::default());
    power_pin.set_high();

    let mut pump_pin = Output::new(peripherals.GPIO13, Level::Low, OutputConfig::default());

    let display_peripherals = DisplayPeripherals {
        backlight: peripherals.GPIO38.degrade(),
        cs: peripherals.GPIO6.degrade(),
        dc: peripherals.GPIO7.degrade(),
        rst: peripherals.GPIO5.degrade(),
        wr: peripherals.GPIO8.degrade(),
        rd: peripherals.GPIO9.degrade(),
        d0: peripherals.GPIO39.degrade(),
        d1: peripherals.GPIO40.degrade(),
        d2: peripherals.GPIO41.degrade(),
        d3: peripherals.GPIO42.degrade(),
        d4: peripherals.GPIO45.degrade(),
        d5: peripherals.GPIO46.degrade(),
        d6: peripherals.GPIO47.degrade(),
        d7: peripherals.GPIO48.degrade(),
    };

    // see https://github.com/Xinyuan-LilyGO/T-Display-S3/blob/main/image/T-DISPLAY-S3.jpg
    let sensor_peripherals = SensorPeripherals {
        dht11_digital_pin: peripherals.GPIO1,
        battery_pin: peripherals.GPIO4,
        moisture_analog_pin: peripherals.GPIO2,
        moisture_power_pin: peripherals.GPIO16,
        water_level_analog_pin: peripherals.GPIO3,
        water_level_power_pin: peripherals.GPIO21,
        adc1: peripherals.ADC1,
    };

    // The wake cycle is fallible, but the device always goes back to sleep:
    // a failed cycle (router down, broker unreachable) retries in an hour
    // instead of boot-looping with the radio on.
    if let Err(error) = run_cycle(
        spawner,
        peripherals.WIFI,
        display_peripherals,
        sensor_peripherals,
        &mut pump_pin,
        boot_count,
        reset_reason,
    )
    .await
    {
        error!("Error while running wake cycle: {error:?}");
    }

    info!("Request to disconnect wifi");
    WIFI_SIGNAL.signal(());

    power_pin.set_low();

    let deep_sleep_duration = Duration::from_secs(DEEP_SLEEP_DURATION_SECONDS);
    info!("Enter deep sleep for {}s", DEEP_SLEEP_DURATION_SECONDS);
    // Give the USB CDC logger time to flush pending output before powering down
    Timer::after(Duration::from_millis(100)).await;
    let mut wake_up_btn_pin = peripherals.GPIO14;
    enter_deep(&mut wake_up_btn_pin, peripherals.LPWR, deep_sleep_duration);
}

/// One linear wake cycle: connect WiFi while sampling sensors, show the
/// readings, publish to MQTT, then listen for pump commands until the awake
/// window closes.
async fn run_cycle(
    spawner: Spawner,
    wifi: WIFI<'static>,
    display_peripherals: DisplayPeripherals,
    sensor_peripherals: SensorPeripherals,
    pump_pin: &mut Output<'static>,
    boot_count: u32,
    reset_reason: Option<SocResetReason>,
) -> Result<(), Error> {
    // Everything in the cycle works against one deadline: whatever time WiFi,
    // sensors and publishing don't use remains as the MQTT command window.
    let deadline = Instant::now() + Duration::from_secs(AWAKE_DURATION_SECONDS);

    // Timer wakes are unattended; button wakes and any other wake (e.g. cold
    // boot / USB reset, which is neither Ext0 nor Timer) have someone present.
    let display_enabled = !matches!(wakeup_cause(), SleepSource::Timer);

    // Pre-radio phase: read the timing-sensitive DHT11 and an early battery
    // sample *before* the WiFi radio is powered on, so radio interrupts can't
    // corrupt the bit-banged DHT11 read and we know the battery state before
    // committing to WiFi/pump current draw.
    let readout = sensors::begin_read(sensor_peripherals).await;

    // Low-battery guard: a weak LiPo browns out under radio/pump current spikes,
    // causing a reset loop that drains it further. Skip WiFi and pump, and sleep.
    // Only pay for the display (backlight + panel wake) on an attended wake —
    // a timer wake has nobody watching.
    if let Some(battery_mv) = readout.battery_mv
        && battery_mv < LOW_BATTERY_CUTOFF_MV
    {
        warn!(
            "Battery {}mV below cutoff {}mV — skipping WiFi/pump this cycle",
            battery_mv, LOW_BATTERY_CUTOFF_MV
        );
        let sensor_data = sensors::finish_read(readout).await;
        if display_enabled {
            let mut display = Display::new(display_peripherals, Delay)?;
            display.write_multiline(&format!("LOW BATTERY {battery_mv}mV\n{sensor_data}"))?;
            display.enable_powersave()?;
        }
        return Ok(());
    }

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Overlap the slow WiFi/DHCP handshake with the ADC sampling (moisture,
    // water level, battery) — these are not timing-sensitive to radio activity.
    let (stack, mut sensor_data) = join(
        with_timeout(
            Duration::from_secs(WIFI_CONNECT_TIMEOUT_SECONDS),
            connect_to_wifi(wifi, seed, spawner),
        ),
        sensors::finish_read(readout),
    )
    .await;
    let stack = stack.map_err(|_| Error::WifiTimeout)??;

    // Boot count is published every cycle so a gap in HA (timer wake fired but
    // publish failed vs. timer never fired) is distinguishable remotely.
    if sensor_data
        .data
        .push(Sensor::BootCount(boot_count))
        .is_err()
    {
        error!("Failed to push BootCount to sensor_data");
    }

    // Overflow state is established before MQTT ever connects, so a retained
    // ON command can never race the interlock. Fail closed: a confirmed dry
    // reading is required to allow the pump, so a failed/missing water-level
    // sample (sensor fault, ADC read failure) blocks the pump instead of
    // silently allowing it.
    let pump_allowed = sensor_data
        .data
        .iter()
        .any(|e| matches!(e, Sensor::OverflowDetected(false)));
    if !pump_allowed
        && !sensor_data
            .data
            .iter()
            .any(|e| matches!(e, Sensor::OverflowDetected(_)))
    {
        warn!("No overflow reading this cycle — pump blocked as a precaution");
    }

    // Only build the display on an attended wake — nobody is watching a
    // timer wake, so skip the backlight/panel current draw entirely for it.
    let mut display = if display_enabled {
        let mut display = Display::new(display_peripherals, Delay)?;
        let status = if let Some(stack_config) = stack.config_v4() {
            format!(
                "Reset: {:?}\nClient IP: {}\nBoot count: {}\n{}",
                reset_reason, stack_config.address, boot_count, sensor_data
            )
        } else {
            error!("Failed to get stack config");
            format!("{sensor_data}")
        };
        display.write_multiline(&status)?;
        Some(display)
    } else {
        None
    };

    // mqtt::connect (DNS, TCP connect, broker handshake) has no internal
    // timeout, so a stalled broker/network could otherwise keep the radio on
    // past the awake window indefinitely. Bound the whole session by the same
    // deadline the pump command loop already uses.
    let result = match with_deadline(
        deadline,
        run_mqtt_session(stack, &sensor_data, pump_allowed, deadline, pump_pin),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => {
            error!("MQTT session exceeded awake deadline");
            Err(Error::MqttDeadline)
        }
    };

    if let Some(display) = &mut display {
        display.enable_powersave()?;
    }

    result
}

/// Connect to MQTT, publish sensor state, then poll for pump commands until
/// the awake deadline. Split out so the caller can always power down the
/// display afterward regardless of how this returns.
async fn run_mqtt_session(
    stack: Stack<'static>,
    sensor_data: &domain::SensorData,
    pump_allowed: bool,
    deadline: Instant,
    pump_pin: &mut Output<'static>,
) -> Result<(), Error> {
    let mut session = mqtt::connect(stack).await?;
    session.publish(sensor_data).await?;
    session.subscribe_to_pump_commands().await?;

    // Keep listening until the deadline so a switch flipped while the device
    // is awake still works; the retained ON from the sleep period arrives
    // right after subscribing.
    loop {
        match session.wait_for_pump_command(pump_allowed, deadline).await {
            Ok(true) => run_pump(pump_pin).await,
            Ok(false) => break, // awake window over
            Err(error) => {
                // No reconnect: the next wake is in an hour anyway.
                error!("MQTT error during command window: {error}");
                break;
            }
        }
    }

    Ok(())
}

#[derive(Debug)]
enum Error {
    Wifi(WifiError),
    WifiTimeout,
    Display(display::Error),
    Mqtt(mqtt::Error),
    MqttDeadline,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Wifi(error) => write!(f, "Wifi error: {error:?}"),
            Error::WifiTimeout => write!(f, "Wifi connection timed out"),
            Error::Display(error) => write!(f, "Display error: {error}"),
            Error::Mqtt(error) => write!(f, "MQTT error: {error}"),
            Error::MqttDeadline => write!(f, "MQTT session exceeded awake deadline"),
        }
    }
}

impl From<WifiError> for Error {
    fn from(error: WifiError) -> Self {
        Self::Wifi(error)
    }
}

impl From<display::Error> for Error {
    fn from(error: display::Error) -> Self {
        Self::Display(error)
    }
}

impl From<mqtt::Error> for Error {
    fn from(error: mqtt::Error) -> Self {
        Self::Mqtt(error)
    }
}
