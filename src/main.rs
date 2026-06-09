#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use alloc::format;
use config::{AWAKE_DURATION_SECONDS, DEEP_SLEEP_DURATION_SECONDS};
use display::{Display, DisplayPeripherals, DisplayTrait};
use domain::SensorData;
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    channel::Channel,
    signal::Signal,
};
use embassy_time::{Delay, Duration, Timer};
use esp_alloc::{heap_allocator, psram_allocator};
use esp_backtrace as _;
use esp_hal::{
    Config,
    clock::CpuClock,
    gpio::{Level, Output, OutputConfig, Pin},
    ram,
    rng::Rng,
    rtc_cntl::wakeup_cause,
    system::{SleepSource, software_reset},
    timer::timg::TimerGroup,
};
use esp_println::logger::init_logger;
use esp_radio::wifi::WifiError;
use esp_rtos::main;
use log::{error, info};
use relay_task::relay_task;
use rtc_memory::RtcCell;
use sensors::{SensorPeripherals, sensor_task};
use sleep::enter_deep;
use static_cell::StaticCell;
use update_task::update_task;
use wifi::{WIFI_SIGNAL, connect_to_wifi};

extern crate alloc;

mod config;
mod display;
mod domain;
mod relay_task;
mod rtc_memory;
mod sensors;
mod sleep;
mod update_task;
mod wifi;

/// A channel between sensor sampler and display updater
static CHANNEL: StaticCell<Channel<NoopRawMutex, SensorData, 3>> = StaticCell::new();
/// Fired by update_task to start a timed pump run (HA command only).
static ENABLE_PUMP: Signal<CriticalSectionRawMutex, ()> = Signal::new();
/// Relay task signals pump on/off so update_task can publish state to HA.
static PUMP_STATE: Signal<CriticalSectionRawMutex, bool> = Signal::new();
/// Fired by main before deep sleep so update_task puts the display to sleep.
static DISPLAY_SLEEP: Signal<CriticalSectionRawMutex, ()> = Signal::new();

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

    if let Err(error) = main_fallible(spawner, boot_count).await {
        error!("Error while running firmware: {:?}", error);
        software_reset()
    }
}

async fn main_fallible(spawner: Spawner, boot_count: u32) -> Result<(), Error> {
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

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    let stack = connect_to_wifi(peripherals.WIFI, seed, spawner).await?;

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

    // Skip display init on timer wakes — nobody is watching.
    let button_wake = matches!(wakeup_cause(), SleepSource::Ext0);
    let mut display = Display::new(display_peripherals, Delay, button_wake)?;

    if button_wake {
        if let Some(stack_config) = stack.config_v4() {
            display.write_multiline(
                format!(
                    "Client IP: {}\nBoot count: {}",
                    stack_config.address, boot_count
                )
                .as_str(),
            )?;
        } else {
            error!("Failed to get stack config");
        }
    }

    info!("Create channel");
    let sensordata_channel: &'static mut _ = CHANNEL.init(Channel::new());
    let sensordata_receiver = sensordata_channel.receiver();
    let sensordata_sender = sensordata_channel.sender();

    spawner.spawn(
        update_task(stack, display, sensordata_receiver).expect("Unable to start update task"),
    );

    // see https://github.com/Xinyuan-LilyGO/T-Display-S3/blob/main/image/T-DISPLAY-S3.jpg
    let sensor_peripherals = SensorPeripherals {
        dht11_digital_pin: peripherals.GPIO1,
        battery_pin: peripherals.GPIO4,
        moisture_analog_pin: peripherals.GPIO11,
        moisture_power_pin: peripherals.GPIO16,
        water_level_analog_pin: peripherals.GPIO12,
        water_level_power_pin: peripherals.GPIO21,
        adc1: peripherals.ADC1,
        adc2: peripherals.ADC2,
    };

    spawner.spawn(
        sensor_task(sensordata_sender, sensor_peripherals).expect("Unable to start sensor task"),
    );

    spawner.spawn(relay_task(peripherals.GPIO2.degrade()).expect("Unable to start relay task"));

    let awake_duration = Duration::from_secs(AWAKE_DURATION_SECONDS);

    info!("Stay awake for {}s", awake_duration.as_secs());
    Timer::after(awake_duration).await;
    info!("Request to disconnect wifi");
    WIFI_SIGNAL.signal(());
    info!("Request to put display to sleep");
    DISPLAY_SLEEP.signal(());

    // set power pin to low to save power
    power_pin.set_low();

    let deep_sleep_duration = Duration::from_secs(DEEP_SLEEP_DURATION_SECONDS);
    info!("Enter deep sleep for {}s", DEEP_SLEEP_DURATION_SECONDS);
    // Give the USB CDC logger time to flush pending output before powering down
    Timer::after(Duration::from_millis(100)).await;
    let mut wake_up_btn_pin = peripherals.GPIO14;
    enter_deep(&mut wake_up_btn_pin, peripherals.LPWR, deep_sleep_duration);
}

#[derive(Debug)]
enum Error {
    Wifi(WifiError),
    Display(display::Error),
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Wifi(error) => write!(f, "Wifi error: {error:?}"),
            Error::Display(error) => write!(f, "Display error: {error}"),
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
