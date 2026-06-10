use embassy_time::{Duration, Timer};
use esp_hal::gpio::Output;
use log::info;

const PUMP_RUN_DURATION: Duration = Duration::from_secs(10);

/// Run the pump for the fixed watering duration. Awaited inline by the wake
/// cycle so deep sleep can never cut a run short.
pub async fn run_pump(relay_pin: &mut Output<'_>) {
    info!("Turning on pump");
    relay_pin.set_high();
    Timer::after(PUMP_RUN_DURATION).await;
    relay_pin.set_low();
    info!("Pump off after {} s", PUMP_RUN_DURATION.as_secs());
}
