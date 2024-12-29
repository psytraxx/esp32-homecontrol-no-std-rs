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
    clock::Clock,
    config::MEASUREMENT_INTERVAL_SECONDS,
    domain::{Sensor, SensorData, WaterLevel},
};

const BATTERY_VOLTAGE: u32 = 3700;
const DHT11_MAX_RETRIES: u8 = 3;
const DHT11_RETRY_DELAY_MS: u64 = 2000;
const MOISTURE_MIN: u16 = 2010;
const MOISTURE_MAX: u16 = 3895;
const WATER_LEVEL_THRESHOLD: u16 = 1000;
const SENSOR_COOLDOWN_MILLISECONDS: u64 = 10;

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
    clock: Clock,
    p: SensorPeripherals,
) {
    info!("Create");
    let dht11_pin = OutputOpenDrain::new(p.dht11_pin, Level::High, Pull::None);
    let mut dht11_sensor = Dht11::new(dht11_pin);

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config.enable_pin(p.moisture_pin, Attenuation::Attenuation11dB);
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
        let sampling_period = Duration::from_secs(MEASUREMENT_INTERVAL_SECONDS);
        let wait_interval = clock.duration_to_next_rounded_wakeup(sampling_period);
        info!(
            "Waiting {:?} seconds before reading sensors",
            wait_interval.as_secs()
        );
        Timer::after(wait_interval).await;
        info!("Reading sensors");
        let mut sensor_data = SensorData::default();

        read_dht11(&mut dht11_sensor, &mut sensor_data).await;
        Timer::after(Duration::from_millis(SENSOR_COOLDOWN_MILLISECONDS)).await;
        read_moisture(&mut adc2, &mut moisture_pin, &mut sensor_data);
        Timer::after(Duration::from_millis(SENSOR_COOLDOWN_MILLISECONDS)).await;
        read_water_level(&mut adc2, &mut waterlevel_pin, &mut sensor_data);
        Timer::after(Duration::from_millis(SENSOR_COOLDOWN_MILLISECONDS)).await;
        read_battery(&mut adc1, &mut battery_pin, &mut sensor_data);

        sender.send(sensor_data).await;
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

fn read_moisture(
    adc: &mut Adc<ADC2>,
    pin: &mut AdcPin<GpioPin<11>, ADC2>,
    sensor_data: &mut SensorData,
) {
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => {
            info!("Analog Moisture reading: {}", value);
            let value = normalise_humidity_data(value);
            let value = (value * 100.0) as u16;
            info!("Normalized Moisture reading: {}%", value);
            sensor_data.data.push(Sensor::SoilMoisture(value));
        }
        Err(_) => error!("Error reading moisture sensor"),
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

fn read_battery(
    adc: &mut Adc<ADC1>,
    pin: &mut AdcPin<GpioPin<4>, ADC1, AdcCalCurve<ADC1>>,
    sensor_data: &mut SensorData,
) {
    if let Ok(raw) = nb::block!(adc.read_oneshot(pin)) {
        let raw_mv = (raw as u32) * 2;
        let is_usb = raw_mv > BATTERY_VOLTAGE;
        let voltage = raw_mv.min(BATTERY_VOLTAGE);

        info!(
            "Battery: {}mV{}",
            voltage,
            if is_usb { " [USB]" } else { "" }
        );
        sensor_data.data.push(Sensor::BatteryVoltage(voltage));
    } else {
        error!("Error reading battery voltage");
    }
}

/// The hw390 moisture sensor returns a value between 3000 and 4095.
/// From our measurements, the sensor was in water at 3000 and in air at 4095.
/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let normalized =
        (readout.saturating_sub(MOISTURE_MIN)) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32;
    normalized.clamp(0.0, 1.0)
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
