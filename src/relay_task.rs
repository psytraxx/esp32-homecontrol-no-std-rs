use embassy_time::{Duration, Timer};
use esp_hal::gpio::{AnyPin, Level, Output, OutputConfig};
use log::info;

use crate::{ENABLE_PUMP, PUMP_STATE};

const PUMP_RUN_DURATION: Duration = Duration::from_secs(10);

#[embassy_executor::task]
pub async fn relay_task(pin: AnyPin<'static>) {
    info!("Created a relay task");
    let mut relay_pin = Output::new(pin, Level::Low, OutputConfig::default());

    loop {
        ENABLE_PUMP.wait().await;

        info!("Turning on pump");
        relay_pin.set_high();
        PUMP_STATE.signal(true);

        Timer::after(PUMP_RUN_DURATION).await;

        relay_pin.set_low();
        PUMP_STATE.signal(false);
        info!("Pump off after 10 s");
    }
}
