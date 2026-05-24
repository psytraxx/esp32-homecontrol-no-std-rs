use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c as _;
use esp_hal::i2c::master::I2c;
use heapless::Vec;
use log::{error, info, warn};

use crate::{
    config::SENSOR_SAMPLE_COUNT,
    domain::{MoistureLevel, Sensor, SensorData, overflow_detected},
};

use super::adc::read_powered_adc_sensor;
use super::hardware::SensorHardware;

/// STEMMA Soil Sensor default I2C address.
const STEMMA_ADDRESS: u8 = 0x36;
/// INA219 shunt resistor value (module uses 0.1 Ω).
const INA219_SHUNT_OHM: f32 = 0.1;

use esp_hal::Async;

type SharedI2c = Mutex<NoopRawMutex, I2c<'static, Async>>;

/// Collect all sensor readings and build the averaged SensorData.
pub(super) async fn collect_all_sensor_data(hw: &mut SensorHardware<'static>) -> SensorData {
    let mut sensor_data = SensorData::default();

    // AHT20: temperature + humidity
    if let Some(aht) = hw.aht20.as_mut() {
        match aht.measure().await {
            Ok(m) => {
                let temp = m.temperature.celsius();
                let hum = m.relative_humidity;
                info!("AHT20 — temp: {:.1}°C  humidity: {:.1}%", temp, hum);
                push(&mut sensor_data, Sensor::AirTemperature(temp), "AirTemperature");
                push(&mut sensor_data, Sensor::AirHumidity(hum), "AirHumidity");
            }
            Err(e) => error!("AHT20 read failed: {:?}", e),
        }
    }

    // BMP280: barometric pressure
    match hw.bme280.read_pressure().await {
        Ok(Some(p)) => {
            let hpa = p / 100.0;
            info!("BMP280 — pressure: {:.1} hPa", hpa);
            push(&mut sensor_data, Sensor::AirPressure(hpa), "AirPressure");
        }
        Ok(None) => warn!("BMP280 pressure not available"),
        Err(e) => error!("BMP280 read failed: {:?}", e),
    }

    // STEMMA Soil: moisture counts + soil temperature
    match read_stemma(hw.i2c_bus).await {
        Ok((moisture, soil_temp)) => {
            let moisture_level = MoistureLevel::from(moisture);
            info!(
                "STEMMA — moisture: {} ({})  soil temp: {:.1}°C",
                moisture, moisture_level, soil_temp
            );
            push(&mut sensor_data, Sensor::SoilMoisture(moisture), "SoilMoisture");
            push(
                &mut sensor_data,
                Sensor::SoilMoistureLevel(moisture_level),
                "SoilMoistureLevel",
            );
            push(
                &mut sensor_data,
                Sensor::SoilTemperature(soil_temp),
                "SoilTemperature",
            );
        }
        Err(e) => error!("STEMMA read failed: {}", e),
    }

    // INA219: battery voltage, current, power
    if let Some(ina) = hw.ina219.as_mut() {
        match ina.next_measurement().await {
            Ok(Some(m)) => {
                let voltage_mv = m.bus_voltage.voltage_mv();
                let shunt_uv = m.shunt_voltage.shunt_voltage_uv();
                let current_ma = shunt_uv as f32 / (INA219_SHUNT_OHM * 1000.0);
                let power_mw = voltage_mv as f32 * current_ma / 1000.0;
                info!(
                    "INA219 — bus: {} mV  shunt: {} µV  I: {:.1} mA  P: {:.1} mW",
                    voltage_mv, shunt_uv, current_ma, power_mw
                );
                push(&mut sensor_data, Sensor::BatteryVoltage(voltage_mv), "BatteryVoltage");
                push(&mut sensor_data, Sensor::BatteryCurrent(current_ma), "BatteryCurrent");
                push(&mut sensor_data, Sensor::BatteryPower(power_mw), "BatteryPower");
            }
            Ok(None) => warn!("INA219: conversion not ready"),
            Err(e) => error!("INA219 read failed: {:?}", e),
        }
    }

    // Water level ADC — binary overflow detection
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    for _ in 0..SENSOR_SAMPLE_COUNT {
        if let Some(v) = read_powered_adc_sensor(
            &mut hw.adc2,
            &mut hw.waterlevel_pin,
            &mut hw.water_level_power_pin,
        )
        .await
        {
            let _ = water_level_samples.push(v);
        }
    }

    if water_level_samples.len() > 2 {
        water_level_samples.sort_unstable();
        let trimmed = &water_level_samples[1..water_level_samples.len() - 1];
        let sum: i32 = trimmed.iter().map(|&x| x as i32).sum();
        if let Some(avg) = sum
            .checked_div(trimmed.len() as i32)
            .and_then(|a| u16::try_from(a).ok())
        {
            let detected = overflow_detected(avg);
            info!(
                "Overflow raw ADC: {}mV → {}",
                avg,
                if detected { "Water in overflow" } else { "No water in overflow" }
            );
            push(
                &mut sensor_data,
                Sensor::OverflowDetected(detected),
                "OverflowDetected",
            );
        }
    } else {
        error!("Overflow sensor: not enough ADC samples");
    }

    sensor_data
}

/// Read STEMMA Soil Sensor via raw I2C transactions.
///
/// Protocol (from Adafruit Seesaw firmware):
/// - Temperature: write `[0x00, 0x04]`, wait 125 ms, read 4 bytes → big-endian i32 × 0.000015258789
///   = °C
/// - Moisture:    write `[0x0F, 0x10]`, wait 5 ms,  read 2 bytes → big-endian u16 (200–2000)
///
/// Each I2C transaction gets its own `I2cDevice` so the bus lock is released between write and
/// read, allowing the Timer delay to run without holding the mutex.
async fn read_stemma(bus: &'static SharedI2c) -> Result<(u16, f32), &'static str> {
    // Temperature: write register address, release bus, wait, then read
    {
        let mut dev = I2cDevice::new(bus);
        dev.write(STEMMA_ADDRESS, &[0x00, 0x04])
            .await
            .map_err(|_| "STEMMA temp write")?;
    }
    Timer::after(Duration::from_millis(125)).await;
    let soil_temp = {
        let mut dev = I2cDevice::new(bus);
        let mut buf = [0u8; 4];
        dev.read(STEMMA_ADDRESS, &mut buf)
            .await
            .map_err(|_| "STEMMA temp read")?;
        i32::from_be_bytes(buf) as f32 * 0.000_015_258_789
    };

    // Moisture: write register address, release bus, wait, then read
    {
        let mut dev = I2cDevice::new(bus);
        dev.write(STEMMA_ADDRESS, &[0x0F, 0x10])
            .await
            .map_err(|_| "STEMMA moisture write")?;
    }
    Timer::after(Duration::from_millis(5)).await;
    let moisture = {
        let mut dev = I2cDevice::new(bus);
        let mut buf = [0u8; 2];
        dev.read(STEMMA_ADDRESS, &mut buf)
            .await
            .map_err(|_| "STEMMA moisture read")?;
        u16::from_be_bytes(buf)
    };

    Ok((moisture, soil_temp))
}

fn push(sensor_data: &mut SensorData, sensor: Sensor, name: &str) {
    if sensor_data.data.push(sensor).is_err() {
        error!("SensorData Vec full — could not push {}", name);
    }
}
