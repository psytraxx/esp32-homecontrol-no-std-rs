use embassy_time::{Delay, Duration, Timer};
use heapless::Vec;
use log::{error, info};

use crate::{
    BOOT_COUNT,
    config::{DHT11_WARMUP_DELAY_MS, PUMP_TRIGGER_INTERVAL, SENSOR_SAMPLE_COUNT},
    dht11::{Dht11, Measurement},
    domain::{Actuator, Sensor, SensorData, WaterLevel},
};

use super::adc::{calculate_average, read_battery_voltage, read_powered_adc_sensor};
use super::hardware::SensorHardware;

/// Read a single DHT11 measurement after the required warmup delay.
async fn read_dht11_sensor(dht11_pin: &mut esp_hal::gpio::Flex<'static>) -> Option<Measurement> {
    let mut dht11_sensor = Dht11::new(dht11_pin, Delay);
    Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MS)).await;
    dht11_sensor.read().ok()
}

/// Collect SENSOR_SAMPLE_COUNT readings from every sensor and build the averaged SensorData.
pub(super) async fn collect_all_sensor_data(hardware: &mut SensorHardware<'static>) -> SensorData {
    let mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut air_temperature_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

    // DHT11 is read once per wake cycle: it needs a 2s warmup after power-on and
    // cannot be sampled faster than ~1Hz anyway. Filling all sample slots from one
    // reading keeps the averaging logic intact without burning 2s × N of idle time.
    if let Some(measurement) = read_dht11_sensor(&mut hardware.dht11_pin).await {
        for _ in 0..SENSOR_SAMPLE_COUNT {
            let _ = air_temperature_samples.push(measurement.temperature);
            let _ = air_humidity_samples.push(measurement.humidity);
        }
    } else {
        error!("DHT11 read failed");
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
    mut air_temperature_samples: Vec<u8, SENSOR_SAMPLE_COUNT>,
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

    // Process water level
    if let Some(avg_water_level) = calculate_average(&mut water_level_samples) {
        let waterlevel: WaterLevel = avg_water_level.into();
        info!("Pot base water level: {}", waterlevel);
        if sensor_data
            .data
            .push(Sensor::WaterLevel(avg_water_level.into()))
            .is_err()
        {
            error!("Failed to push WaterLevel to sensor_data");
        }
    } else {
        error!("Unable to generate average value of water level");
    }

    // Process soil moisture
    if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
        info!("Raw Moisture: {}", avg_soil_moisture);
        if sensor_data
            .data
            .push(Sensor::SoilMoistureRaw(avg_soil_moisture.into()))
            .is_err()
        {
            error!("Failed to push SoilMoistureRaw to sensor_data");
        }
        if sensor_data
            .data
            .push(Sensor::SoilMoisture(avg_soil_moisture.into()))
            .is_err()
        {
            error!("Failed to push SoilMoisture to sensor_data");
        }
    } else {
        error!("Unable to generate average value of soil moisture");
    }

    // Record pump actuator state (triggers every PUMP_TRIGGER_INTERVAL boots)
    let boot_count = BOOT_COUNT.get();
    let pump_enabled = boot_count.is_multiple_of(PUMP_TRIGGER_INTERVAL);
    if sensor_data
        .actuators
        .push(Actuator::Pump(pump_enabled))
        .is_err()
    {
        error!("Failed to push Pump actuator");
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

    // Only publish sensor data if we received battery readings (proxy for healthy reads)
    sensor_data.publish = !battery_voltage_samples.is_empty();

    sensor_data
}
