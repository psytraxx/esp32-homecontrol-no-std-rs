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
    let mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut air_temperature_samples: Vec<i8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

    // DHT11 was read once (pre-radio) for this cycle. Filling all sample slots
    // from that one reading keeps the averaging logic intact.
    if let Some(measurement) = dht11_reading {
        for _ in 0..SENSOR_SAMPLE_COUNT {
            let _ = air_temperature_samples.push(measurement.temperature);
            let _ = air_humidity_samples.push(measurement.relative_humidity);
        }
    }

    for i in 0..SENSOR_SAMPLE_COUNT {
        info!(
            "Reading ADC sensor data {}/{}",
            (i + 1),
            SENSOR_SAMPLE_COUNT
        );

        // Read soil moisture (powered ADC sensor)
        if let Some(moisture) = read_powered_adc_sensor(
            &mut hardware.adc2,
            &mut hardware.moisture_pin,
            &mut hardware.moisture_power_pin,
        )
        .await
            && soil_moisture_samples.push(moisture).is_err()
        {
            error!("Failed to push SoilMoisture to sensor_data");
        }

        // Read water level (powered ADC sensor)
        if let Some(water_level) = read_powered_adc_sensor(
            &mut hardware.adc2,
            &mut hardware.waterlevel_pin,
            &mut hardware.water_level_power_pin,
        )
        .await
            && water_level_samples.push(water_level).is_err()
        {
            error!("Failed to push WaterLevel to sensor_data");
        }

        // Read battery voltage
        if let Some(battery_voltage) =
            read_battery_voltage(&mut hardware.adc1, &mut hardware.battery_pin).await
            && battery_voltage_samples.push(battery_voltage).is_err()
        {
            error!("Failed to push BatteryVoltage to sensor_data");
        }
    }

    build_sensor_data(
        air_humidity_samples,
        air_temperature_samples,
        soil_moisture_samples,
        battery_voltage_samples,
        water_level_samples,
    )
}

/// Average the raw sample vecs and assemble the final SensorData.
fn build_sensor_data(
    mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT>,
    mut air_temperature_samples: Vec<i8, SENSOR_SAMPLE_COUNT>,
    mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT>,
    mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT>,
    mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT>,
) -> SensorData {
    let mut sensor_data = SensorData::default();

    // Process air humidity
    if let Some(avg_air_humidity) = calculate_average(&mut air_humidity_samples) {
        info!("Air humidity: {}%", avg_air_humidity);
        if sensor_data
            .data
            .push(Sensor::AirHumidity(avg_air_humidity))
            .is_err()
        {
            error!("Failed to push AirHumidity to sensor_data");
        }
    } else {
        error!(
            "Unable to generate average value of air humidity - we had {} samples",
            air_humidity_samples.len()
        );
    }

    // Process air temperature
    if let Some(avg_air_temperature) = calculate_average(&mut air_temperature_samples) {
        info!("Air temperature: {}°C", avg_air_temperature);
        if sensor_data
            .data
            .push(Sensor::AirTemperature(avg_air_temperature))
            .is_err()
        {
            error!("Failed to push AirTemperature to sensor_data");
        }
    } else {
        error!(
            "Unable to generate average value of air temperature, we had {} samples",
            air_temperature_samples.len()
        );
    }

    // Process overflow sensor
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
        if sensor_data
            .data
            .push(Sensor::OverflowDetected(detected))
            .is_err()
        {
            error!("Failed to push OverflowDetected to sensor_data");
        }
    } else {
        error!("Unable to generate average value of overflow sensor");
    }

    // Process soil moisture
    if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
        let moisture_level = MoistureLevel::from(avg_soil_moisture);
        info!("Raw Moisture: {} ({})", avg_soil_moisture, moisture_level);
        if sensor_data
            .data
            .push(Sensor::SoilMoistureRaw(avg_soil_moisture.into()))
            .is_err()
        {
            error!("Failed to push SoilMoistureRaw to sensor_data");
        }
        if sensor_data
            .data
            .push(Sensor::SoilMoisture(moisture_level))
            .is_err()
        {
            error!("Failed to push SoilMoisture to sensor_data");
        }
    } else {
        error!("Unable to generate average value of soil moisture");
    }

    // Process battery voltage
    if let Some(avg_battery_voltage) = calculate_average(&mut battery_voltage_samples) {
        info!("Battery voltage: {}mV", avg_battery_voltage);
        if sensor_data
            .data
            .push(Sensor::BatteryVoltage(avg_battery_voltage))
            .is_err()
        {
            error!("Failed to push BatteryVoltage to sensor_data");
        }
    }

    sensor_data
}
