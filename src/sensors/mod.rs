mod adc;
mod builder;
mod hardware;

pub use hardware::SensorPeripherals;

use log::info;

use crate::domain::SensorData;

/// Read all sensors once: initialize the hardware, collect one averaged set of
/// samples and return it. Called once per wake cycle, in parallel with the
/// WiFi connection.
pub async fn read_sensors(p: SensorPeripherals) -> SensorData {
    info!("Initializing sensor hardware");
    let mut hw = hardware::initialize_hardware(p).await;
    builder::collect_all_sensor_data(&mut hw).await
}
