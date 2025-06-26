use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    peripherals::GPIO2,
};
use esp_println::println;

use crate::ENABLE_PUMP;

const PUMP_INTERVAL: Duration = Duration::from_secs(10);

#[embassy_executor::task]
pub async fn relay_task(pin: GPIO2<'static>) {
    println!("Created a relay task");
    // Configure GPIO pin for relay (using GPIO2)
    let mut dht_pin = Output::new(pin, Level::Low, OutputConfig::default());

    loop {
        let start_pump = ENABLE_PUMP.wait().await;
        if start_pump {
            println!("Turning on pump");
            dht_pin.set_high();
            Timer::after(PUMP_INTERVAL).await;
            println!("Turning off");
            dht_pin.set_low();
        }
    }
}
