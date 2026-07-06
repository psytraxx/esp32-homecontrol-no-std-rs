use dht_sensor::dht11::Reading;
use embassy_time::{Delay, Duration, Timer};
use heapless::Vec;
use log::{error, info, warn};

use crate::{
    config::{DHT11_MAX_ATTEMPTS, DHT11_WARMUP_DELAY_MS, SENSOR_SAMPLE_COUNT},
    domain::{MoistureLevel, Sensor, SensorData, overflow_detected},
};

use super::adc::{calculate_average, read_battery_voltage, read_powered_adc_sensor};
use super::hardware::SensorHardware;

/// Read the DHT11, retrying on the frequent timing-induced checksum failures.
///
/// The DHT11 read is a blocking, bit-banged transfer whose correctness depends
/// on microsecond edge timing. It is read *before* the WiFi radio starts so its
/// timing is not corrupted by radio interrupts; retries here absorb the
/// remaining transient glitches. Each attempt is preceded by the warmup/settle
/// delay (the sensor cannot be sampled faster than ~1 Hz).
pub(super) async fn read_dht11_with_retries(
    dht11_pin: &mut esp_hal::gpio::Flex<'static>,
) -> Option<Reading> {
    for attempt in 1..=DHT11_MAX_ATTEMPTS {
        Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MS)).await;
        match dht_sensor::dht11::blocking::read(&mut Delay, dht11_pin) {
            Ok(reading) => return Some(reading),
            Err(error) => warn!(
                "DHT11 read attempt {}/{} failed: {:?}",
                attempt, DHT11_MAX_ATTEMPTS, error
            ),
        }
    }
    error!("DHT11 read failed after {} attempts", DHT11_MAX_ATTEMPTS);
    None
}

/// Collect the ADC sensors (moisture, water level, battery) and assemble the
/// averaged SensorData, folding in the already-taken DHT11 reading.
pub(super) async fn collect_adc_sensor_data(
    hardware: &mut SensorHardware<'static>,
    dht11_reading: Option<Reading>,
) -> SensorData {
    let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

    for i in 0..SENSOR_SAMPLE_COUNT {
        info!(
            "Reading ADC sensor data {}/{}",
            (i + 1),
            SENSOR_SAMPLE_COUNT
        );

        if let Some(moisture) = read_powered_adc_sensor(
            &mut hardware.adc1,
            &mut hardware.moisture_pin,
            &mut hardware.moisture_power_pin,
        )
        .await
            && soil_moisture_samples.push(moisture).is_err()
        {
            error!("Failed to push SoilMoisture to sensor_data");
        }

        if let Some(water_level) = read_powered_adc_sensor(
            &mut hardware.adc1,
            &mut hardware.waterlevel_pin,
            &mut hardware.water_level_power_pin,
        )
        .await
            && water_level_samples.push(water_level).is_err()
        {
            error!("Failed to push WaterLevel to sensor_data");
        }

        if let Some(battery_voltage) =
            read_battery_voltage(&mut hardware.adc1, &mut hardware.battery_pin).await
            && battery_voltage_samples.push(battery_voltage).is_err()
        {
            error!("Failed to push BatteryVoltage to sensor_data");
        }
    }

    let mut sensor_data = SensorData::default();

    // DHT11 was read once (pre-radio) for this cycle — no averaging needed.
    if let Some(reading) = dht11_reading {
        info!(
            "Air temperature: {}°C, air humidity: {}%",
            reading.temperature, reading.relative_humidity
        );
        push(
            &mut sensor_data,
            Sensor::AirTemperature(reading.temperature),
        );
        push(
            &mut sensor_data,
            Sensor::AirHumidity(reading.relative_humidity),
        );
    } else {
        error!("No DHT11 reading available this cycle");
    }

    if let Some(avg_water_level) = calculate_average(&mut water_level_samples) {
        let detected = overflow_detected(avg_water_level);
        info!(
            "Overflow raw ADC: {}mV → {}",
            avg_water_level,
            if detected {
                "Water in overflow"
            } else {
                "No water in overflow"
            }
        );
        push(&mut sensor_data, Sensor::OverflowDetected(detected));
    } else {
        error!("Unable to generate average value of overflow sensor");
    }

    if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
        let moisture_level = MoistureLevel::from(avg_soil_moisture);
        info!("Raw Moisture: {} ({})", avg_soil_moisture, moisture_level);
        push(
            &mut sensor_data,
            Sensor::SoilMoistureRaw(avg_soil_moisture.into()),
        );
        push(&mut sensor_data, Sensor::SoilMoisture(moisture_level));
    } else {
        error!("Unable to generate average value of soil moisture");
    }

    if let Some(avg_battery_voltage) = calculate_average(&mut battery_voltage_samples) {
        info!("Battery voltage: {}mV", avg_battery_voltage);
        push(
            &mut sensor_data,
            Sensor::BatteryVoltage(avg_battery_voltage),
        );
    } else {
        error!("Unable to generate average value of battery voltage");
    }

    sensor_data
}

fn push(sensor_data: &mut SensorData, sensor: Sensor) {
    if sensor_data.data.push(sensor).is_err() {
        error!("Failed to push sensor reading to sensor_data");
    }
}
