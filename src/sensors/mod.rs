mod adc;
mod builder;
mod hardware;

pub use hardware::SensorPeripherals;

use log::info;

use crate::domain::SensorData;
use adc::read_battery_voltage;
use builder::{collect_adc_sensor_data, read_dht11_with_retries};
use dht_sensor::dht11::Reading;
use hardware::SensorHardware;

/// Sensor reading in progress. Created with [`begin_read`] (which reads the
/// timing-sensitive DHT11 and an early battery sample *before* the WiFi radio
/// starts), then completed with [`finish_read`] (the ADC loop, which can safely
/// overlap WiFi via `join`).
pub struct SensorReadout {
    hw: SensorHardware<'static>,
    dht11: Option<Reading>,
    /// Early single-shot battery reading (mV) used for the low-battery guard.
    /// `None` while charging on USB (see [`read_battery_voltage`]).
    pub battery_mv: Option<u16>,
}

/// Pre-radio phase: initialize hardware, read the DHT11 (with retries) and take
/// one battery sample. Keeping these off the radio avoids interrupt corruption
/// of the bit-banged DHT11 read and gives the low-battery guard a value before
/// WiFi is ever powered on.
pub async fn begin_read(p: SensorPeripherals) -> SensorReadout {
    info!("Initializing sensor hardware");
    let mut hw = hardware::initialize_hardware(p).await;
    let dht11 = read_dht11_with_retries(&mut hw.dht11_pin).await;
    let battery_mv = read_battery_voltage(&mut hw.adc1, &mut hw.battery_pin).await;
    SensorReadout {
        hw,
        dht11,
        battery_mv,
    }
}

/// Post-guard phase: sample the ADC sensors and build the averaged SensorData,
/// folding in the DHT11 reading taken in [`begin_read`]. Safe to run inside a
/// `join` alongside the WiFi connection.
pub async fn finish_read(mut readout: SensorReadout) -> SensorData {
    collect_adc_sensor_data(&mut readout.hw, readout.dht11).await
}
