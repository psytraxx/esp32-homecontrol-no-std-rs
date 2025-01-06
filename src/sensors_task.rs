use alloc::vec::Vec;
use defmt::{error, info};
use dht11::Dht11;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{
    analog::adc::{Adc, AdcCalCurve, AdcConfig, AdcPin, Attenuation},
    gpio::{GpioPin, Level, OutputOpenDrain, Pull},
    peripherals::{ADC1, ADC2},
    prelude::nb,
};

use crate::{
    config::AWAKE_DURATION_SECONDS,
    domain::{Sensor, SensorData, WaterLevel},
};

const BATTERY_VOLTAGE: u32 = 3700;
const DHT11_MAX_RETRIES: u8 = 3;
const DHT11_RETRY_DELAY_MS: u64 = 2000;
const MOISTURE_MIN: u16 = 1400;
const MOISTURE_MAX: u16 = 3895;
const WATER_LEVEL_THRESHOLD: u16 = 3000;
const SENSOR_READING_DELAY_MILLISECONDS: u64 = 10;
const MAX_SENSOR_SAMPLE_COUNT: usize = 32;

pub struct SensorPeripherals {
    pub dht11_pin: GpioPin<1>,
    pub battery_pin: GpioPin<4>,
    pub moisture_pin: GpioPin<11>,
    pub water_level_pin: GpioPin<12>,
    pub adc1: ADC1,
    pub adc2: ADC2,
}

#[embassy_executor::task]
pub async fn sensor_task(
    sender: Sender<'static, NoopRawMutex, SensorData, 3>,
    p: SensorPeripherals,
) {
    info!("Create");
    let dht11_pin = OutputOpenDrain::new(p.dht11_pin, Level::High, Pull::None);
    let mut dht11_sensor = Dht11::new(dht11_pin);

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_pin, Attenuation::Attenuation11dB);
    let mut waterlevel_pin =
        adc2_config.enable_pin(p.water_level_pin, Attenuation::Attenuation11dB);
    let mut adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let mut battery_pin = adc1_config.enable_pin_with_cal::<GpioPin<4>, AdcCalCurve<ADC1>>(
        p.battery_pin,
        Attenuation::Attenuation11dB,
    );
    let mut adc1 = Adc::new(p.adc1, adc1_config);

    loop {
        info!("Reading sensors");
        let mut sensor_data = SensorData::default();

        read_dht11(&mut dht11_sensor, &mut sensor_data).await;
        Timer::after(Duration::from_millis(SENSOR_READING_DELAY_MILLISECONDS)).await;
        read_moisture(&mut adc2, &mut moisture_pin, &mut sensor_data).await;
        Timer::after(Duration::from_millis(SENSOR_READING_DELAY_MILLISECONDS)).await;
        read_water_level(&mut adc2, &mut waterlevel_pin, &mut sensor_data);
        Timer::after(Duration::from_millis(SENSOR_READING_DELAY_MILLISECONDS)).await;
        read_battery(&mut adc1, &mut battery_pin, &mut sensor_data).await;

        sender.send(sensor_data).await;
        // next reading will be the device came back from deep sleep
        let sampling_period = Duration::from_secs(AWAKE_DURATION_SECONDS * 2);
        Timer::after(sampling_period).await;
    }
}

async fn read_dht11<'a>(
    dht11_sensor: &mut Dht11<OutputOpenDrain<'a>>,
    sensor_data: &mut SensorData,
) {
    let mut attempts = 0;
    while attempts < DHT11_MAX_RETRIES {
        match dht11_sensor.perform_measurement(&mut Delay) {
            Ok(measurement) => {
                let temperature = measurement.temperature / 10;
                let humidity = measurement.humidity / 10;

                info!(
                    "DHT11 reading... Temperature: {}°C, Humidity: {}%",
                    temperature, humidity
                );

                sensor_data.data.push(Sensor::AirTemperature(temperature));
                sensor_data.data.push(Sensor::AirHumidity(humidity));
                return;
            }
            Err(_) => {
                attempts += 1;
                error!(
                    "Error reading DHT11 sensor (attempt {}/{})",
                    attempts, DHT11_MAX_RETRIES
                );
                Timer::after(Duration::from_millis(DHT11_RETRY_DELAY_MS)).await;
            }
        }
    }
}

async fn read_moisture<'a>(
    adc: &mut Adc<'a, ADC2>,
    pin: &mut AdcPin<GpioPin<11>, ADC2, AdcCalCurve<ADC2>>,
    sensor_data: &mut SensorData,
) {
    let mut samples = Vec::new();

    while samples.len() < MAX_SENSOR_SAMPLE_COUNT {
        match nb::block!(adc.read_oneshot(pin)) {
            Ok(value) => {
                // double range - sum of all samples will not overflow
                samples.push(value as u32);
            }
            Err(_) => error!("Error reading moisture sensor"),
        }
        Timer::after(Duration::from_millis(SENSOR_READING_DELAY_MILLISECONDS)).await;
    }

    if let Some(average) = samples
        .iter()
        .sum::<u32>()
        .checked_div(samples.len() as u32)
    {
        info!("Analog Moisture reading: {}", average);
        sensor_data
            .data
            .push(Sensor::SoilMoistureRaw(average as u16));

        let moisture = (normalise_humidity_data(average as u16) * 100.0) as u8;
        info!("Normalized Moisture reading: {}%", moisture);
        sensor_data.data.push(Sensor::SoilMoisture(moisture));
    } else {
        error!("Error calculating moisture sensor average");
    }
}

fn read_water_level(
    adc: &mut Adc<ADC2>,
    pin: &mut AdcPin<GpioPin<12>, ADC2>,
    sensor_data: &mut SensorData,
) {
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => {
            info!("Water level reading: {}", value);
            sensor_data.data.push(Sensor::WaterLevel(value.into()));
        }
        Err(_) => error!("Error reading water level sensor"),
    }
}

async fn read_battery<'a>(
    adc: &mut Adc<'a, ADC1>,
    pin: &mut AdcPin<GpioPin<4>, ADC1, AdcCalCurve<ADC1>>,
    sensor_data: &mut SensorData,
) {
    let mut samples = Vec::new();
    while samples.len() < MAX_SENSOR_SAMPLE_COUNT {
        match nb::block!(adc.read_oneshot(pin)) {
            Ok(raw) => {
                let sample: u32 = raw as u32 * 2;
                samples.push(sample);
            }
            Err(_) => {
                error!("Error reading battery voltage");
            }
        }
        Timer::after(Duration::from_millis(SENSOR_READING_DELAY_MILLISECONDS)).await;
    }
    if let Some(avg_sample) = samples
        .iter()
        .sum::<u32>()
        .checked_div(samples.len() as u32)
    {
        let is_usb = avg_sample > BATTERY_VOLTAGE;
        let avg_sample = avg_sample.min(BATTERY_VOLTAGE);

        info!(
            "Battery: {}mV{}",
            avg_sample,
            if is_usb { " [USB]" } else { "" }
        );
        sensor_data
            .data
            .push(Sensor::BatteryVoltage(avg_sample as u16));
    } else {
        error!("Error calculating battery voltage");
    }
}

/// The hw390 moisture sensor returns a value between 3000 and 4095.
/// From our measurements, the sensor was in water at 3000 and in air at 4095.
/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let clamped = readout.clamp(MOISTURE_MIN, MOISTURE_MAX);

    (MOISTURE_MAX - clamped) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32
}

impl From<u16> for WaterLevel {
    fn from(value: u16) -> Self {
        if value < WATER_LEVEL_THRESHOLD {
            WaterLevel::Empty
        } else {
            WaterLevel::Full
        }
    }
}
