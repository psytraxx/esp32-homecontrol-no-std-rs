#![no_std]
#![no_main]
#![feature(async_closure)]

use alloc::format;
use clock::Clock;
use config::{DEEP_SLEEP_DURATION, MEASUREMENTS_NEEDED, MEASUREMENT_INTERVAL_SECONDS};
use defmt::{error, info, Debug2Format};
use display::{Display, DisplayPeripherals, DisplayTrait};
use domain::SensorData;
use embassy_executor::Spawner;
use embassy_sync::{
    blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex},
    channel::Channel,
    mutex::Mutex,
    signal::Signal,
};
use embassy_time::{Duration, Timer};
use esp_alloc::heap_allocator;
use esp_hal::{prelude::*, rng::Rng, timer::timg::TimerGroup};
use esp_wifi::wifi::WifiError;
use relay_task::relay_task;
use sensors_task::{sensor_task, SensorPeripherals};
use sleep::enter_deep;
use static_cell::StaticCell;
use update_task::update_task;
use wifi::{connect_to_wifi, STOP_WIFI_SIGNAL};
use {defmt_rtt as _, esp_backtrace as _};

extern crate alloc;

mod clock;
mod config;
mod display;
mod domain;
mod ntp;
mod relay_task;
mod sensors_task;
mod sleep;
mod update_task;
mod wifi;

/// A channel between sensor sampler and display updater
static CHANNEL: StaticCell<Channel<NoopRawMutex, SensorData, 3>> = StaticCell::new();
static ENABLE_PUMP: Signal<CriticalSectionRawMutex, bool> = Signal::new();

/// Stored boot count between deep sleep cycles
///
/// This is a statically allocated variable and it is placed in the RTC Fast
/// memory, which survives deep sleep.
#[ram(rtc_fast)]
static BOOT_COUNT: Mutex<CriticalSectionRawMutex, u64> = Mutex::new(0);

#[main]
async fn main(spawner: Spawner) {
    let mut count = BOOT_COUNT.lock().await;
    info!("Current boot count = {}", *count);
    *count += 1;

    if let Err(error) = main_fallible(spawner).await {
        error!("Error while running firmware: {:?}", error);
    }
}

async fn main_fallible(spawner: Spawner) -> Result<(), Error> {
    let peripherals = esp_hal::init({
        let mut config = esp_hal::Config::default();
        config.cpu_clock = CpuClock::max();
        config
    });

    heap_allocator!(72 * 1024);

    let rng = Rng::new(peripherals.RNG);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timg1 = TimerGroup::new(peripherals.TIMG1);

    esp_hal_embassy::init(timg0.timer0);

    let stack = connect_to_wifi(
        peripherals.WIFI,
        timg1.timer0,
        peripherals.RADIO_CLK,
        rng,
        spawner,
    )
    .await?;

    info!("Synchronize clock from server");
    let unix_time = ntp::get_unix_time(stack).await?;

    let clock = Clock::new(unix_time as u64);

    if let Some(time) = clock.now() {
        info!("Now is {:?}", Debug2Format(&time));
    }

    let display_peripherals = DisplayPeripherals {
        backlight: peripherals.GPIO38,
        cs: peripherals.GPIO6,
        dc: peripherals.GPIO7,
        rst: peripherals.GPIO5,
        wr: peripherals.GPIO8,
        rd: peripherals.GPIO9,
        d0: peripherals.GPIO39,
        d1: peripherals.GPIO40,
        d2: peripherals.GPIO41,
        d3: peripherals.GPIO42,
        d4: peripherals.GPIO45,
        d5: peripherals.GPIO46,
        d6: peripherals.GPIO47,
        d7: peripherals.GPIO48,
    };

    let mut display = Display::new(display_peripherals)?;

    if let Some(stack_config) = stack.config_v4() {
        display.write(format!("Booting... {}", stack_config.address).as_str())?;
    } else {
        error!("Failed to get stack config");
    }

    info!("Create channel");
    let channel: &'static mut _ = CHANNEL.init(Channel::new());
    let receiver = channel.receiver();
    let sender = channel.sender();

    spawner
        .spawn(update_task(stack, display, receiver, clock.clone(), true))
        .ok();

    // see https://github.com/Xinyuan-LilyGO/T-Display-S3/blob/main/image/T-DISPLAY-S3.jpg
    let sensor_peripherals = SensorPeripherals {
        dht11_pin: peripherals.GPIO1,
        battery_pin: peripherals.GPIO4,
        moisture_pin: peripherals.GPIO11,
        water_level_pin: peripherals.GPIO12,
        adc1: peripherals.ADC1,
        adc2: peripherals.ADC2,
    };

    spawner
        .spawn(sensor_task(sender, clock.clone(), sensor_peripherals))
        .ok();

    spawner.spawn(relay_task(peripherals.GPIO2)).ok();

    let awake_duration = Duration::from_secs(MEASUREMENT_INTERVAL_SECONDS * MEASUREMENTS_NEEDED);

    info!("Stay awake for {}s", awake_duration);
    Timer::after(awake_duration).await;
    info!("Request to disconnect wifi");
    STOP_WIFI_SIGNAL.signal(());
    let deep_sleep_duration = Duration::from_secs(DEEP_SLEEP_DURATION);
    info!("Enter deep sleep for {}s", DEEP_SLEEP_DURATION);
    enter_deep(peripherals.GPIO14, peripherals.LPWR, deep_sleep_duration);
}

#[derive(Debug, defmt::Format)]
enum Error {
    Wifi(WifiError),
    Clock(ntp::Error),
    Display(display::Error),
}

impl From<WifiError> for Error {
    fn from(error: WifiError) -> Self {
        Self::Wifi(error)
    }
}

impl From<ntp::Error> for Error {
    fn from(error: ntp::Error) -> Self {
        Self::Clock(error)
    }
}

impl From<display::Error> for Error {
    fn from(error: display::Error) -> Self {
        Self::Display(error)
    }
}
