#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use alloc::format;
use config::{AWAKE_DURATION_SECONDS, DEEP_SLEEP_DURATION_SECONDS, WIFI_CONNECT_TIMEOUT_SECONDS};
use display::{Display, DisplayPeripherals, DisplayTrait};
use domain::Sensor;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Delay, Duration, Instant, Timer, with_timeout};
use esp_alloc::{heap_allocator, psram_allocator};
use esp_backtrace as _;
use esp_hal::{
    Config,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig, Pin},
    peripherals::WIFI,
    ram,
    rng::Rng,
    rtc_cntl::wakeup_cause,
    system::SleepSource,
    timer::timg::TimerGroup,
};
use esp_println::logger::init_logger;
use esp_radio::wifi::WifiError;
use esp_rtos::main;
use log::{error, info};
use pump::run_pump;
use rtc_memory::RtcCell;
use sensors::{SensorPeripherals, read_sensors};
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

/// Stored boot count between deep sleep cycles
///
/// This is a statically allocated variable and it is placed in the RTC Fast
/// memory, which survives deep sleep. Uses RtcCell for safe interior mutability.
#[ram(unstable(rtc_fast))]
pub(crate) static BOOT_COUNT: RtcCell<u32> = RtcCell::new(0);

/// Tracks whether MQTT discovery messages have been sent
///
/// Placed in RTC Fast memory to prevent re-sending on every wake.
/// Uses RtcCell for safe interior mutability.
#[ram(unstable(rtc_fast))]
pub(crate) static DISCOVERY_MESSAGES_SENT: RtcCell<bool> = RtcCell::new(false);

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
async fn main(spawner: Spawner) {
    init_logger(log::LevelFilter::Info);

    let boot_count = BOOT_COUNT.get();
    info!("Current boot count = {}", boot_count);
    BOOT_COUNT.set(boot_count + 1);

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

    let mut pump_pin = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

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

    // V2 hardware: all sensors on I2C bus (GPIO3=SDA, GPIO10=SCL)
    // Water level ADC (GPIO12) kept — binary overflow detector only
    let sensor_peripherals = SensorPeripherals {
        i2c: peripherals.I2C0,
        sda: peripherals.GPIO3,
        scl: peripherals.GPIO10,
        water_level_analog_pin: peripherals.GPIO12,
        water_level_power_pin: peripherals.GPIO21,
        adc2: peripherals.ADC2,
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
    )
    .await
    {
        error!("Error while running wake cycle: {error:?}");
    }

    info!("Request to disconnect wifi");
    WIFI_SIGNAL.signal(());

    // set power pin to low to save power
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
) -> Result<(), Error> {
    // Everything in the cycle works against one deadline: whatever time WiFi,
    // sensors and publishing don't use remains as the MQTT command window.
    let deadline = Instant::now() + Duration::from_secs(AWAKE_DURATION_SECONDS);

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Overlap the slow WiFi/DHCP handshake with the DHT11 warmup and ADC sampling.
    let (stack, sensor_data) = join(
        with_timeout(
            Duration::from_secs(WIFI_CONNECT_TIMEOUT_SECONDS),
            connect_to_wifi(wifi, seed, spawner),
        ),
        read_sensors(sensor_peripherals),
    )
    .await;
    let stack = stack.map_err(|_| Error::WifiTimeout)??;

    // Overflow state is established before MQTT ever connects, so a retained
    // ON command can never race the interlock.
    let pump_allowed = !sensor_data
        .data
        .iter()
        .any(|e| matches!(e, Sensor::OverflowDetected(true)));

    let button_wake = matches!(wakeup_cause(), SleepSource::Ext0);
    let mut display = Display::new(display_peripherals, Delay, button_wake)?;

    let mut status = format!("{sensor_data}");
    if button_wake {
        if let Some(stack_config) = stack.config_v4() {
            status = format!(
                "Client IP: {}\nBoot count: {}\n{}",
                stack_config.address, boot_count, status
            );
        } else {
            error!("Failed to get stack config");
        }
    }
    display.write_multiline(&status)?;

    let mut session = mqtt::connect(stack).await?;
    session.publish(&sensor_data).await?;
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

    display.enable_powersave()?;
    Ok(())
}

#[derive(Debug)]
enum Error {
    Wifi(WifiError),
    WifiTimeout,
    Display(display::Error),
    Mqtt(mqtt::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Wifi(error) => write!(f, "Wifi error: {error:?}"),
            Error::WifiTimeout => write!(f, "Wifi connection timed out"),
            Error::Display(error) => write!(f, "Display error: {error}"),
            Error::Mqtt(error) => write!(f, "MQTT error: {error}"),
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
