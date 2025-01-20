use alloc::vec::Vec;
use defmt::{error, info, warn};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Duration, Timer};
use embedded_hal::delay::DelayNs;
use esp_hal::{
    analog::adc::{
        Adc, AdcCalCurve, AdcCalLine, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation,
        RegisterAccess,
    },
    delay::Delay,
    gpio::{GpioPin, Input, Level, OutputOpenDrain, Pull},
    peripherals::{ADC1, ADC2},
};

use crate::{
    config::AWAKE_DURATION_SECONDS,
    dht11::Dht11,
    domain::{Sensor, SensorData},
};

const DHT11_DELAY_MS: u64 = 2000;
const MOISTURE_MIN: u16 = 1600;
const MOISTURE_MAX: u16 = 2050;
const USB_CHARGING_VOLTAGE: u16 = 4200;
const SENSOR_WARMUP_DELAY_MILLISECONDS: u64 = 50;
const MAX_SENSOR_SAMPLE_COUNT: usize = 3;

pub struct SensorPeripherals {
    pub dht11_pin: GpioPin<1>,
    pub battery_pin: GpioPin<4>,
    pub moisture_digital_pin: GpioPin<10>,
    pub moisture_analog_pin: GpioPin<11>,
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

    let delay = Delay::new();

    let mut dht11_sensor = Dht11::new(dht11_pin, delay);

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let mut waterlevel_pin = adc2_config.enable_pin(p.water_level_pin, Attenuation::_11dB);
    let mut adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let mut battery_pin = adc1_config
        .enable_pin_with_cal::<GpioPin<4>, AdcCalLine<ADC1>>(p.battery_pin, Attenuation::_11dB);
    let mut adc1 = Adc::new(p.adc1, adc1_config);

    let digital_input = Input::new(p.moisture_digital_pin, esp_hal::gpio::Pull::None);

    loop {
        info!("Reading sensors");
        let mut sensor_data = SensorData::default();

        Timer::after(Duration::from_millis(DHT11_DELAY_MS)).await;
        if let Some(result) = read_dht11(&mut dht11_sensor).await {
            sensor_data
                .data
                .push(Sensor::AirTemperature(result.temperature));
            sensor_data.data.push(Sensor::AirHumidity(result.humidity));
        }

        Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
        if let Some(result) = read_moisture(&mut adc2, &mut moisture_pin, &digital_input).await {
            sensor_data.data.push(Sensor::SoilMoisture(result.moisture));
            sensor_data
                .data
                .push(Sensor::SoilMoistureRaw(result.moisture_raw));
            sensor_data
                .data
                .push(Sensor::PumpTrigger(result.moisture_trigger));
        }

        Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
        if let Some(value) = read_water_level(&mut adc2, &mut waterlevel_pin).await {
            sensor_data.data.push(Sensor::WaterLevel(value.into()));
        }

        Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
        if let Some(value) = read_battery(&mut adc1, &mut battery_pin).await {
            sensor_data.data.push(Sensor::BatteryVoltage(value));
        }

        sender.send(sensor_data).await;

        let sampling_period = Duration::from_secs(AWAKE_DURATION_SECONDS);
        Timer::after(sampling_period).await;
    }
}

struct DHT11Reading {
    temperature: u8,
    humidity: u8,
}

async fn read_dht11<D>(dht11_sensor: &mut Dht11<OutputOpenDrain<'_>, D>) -> Option<DHT11Reading>
where
    D: DelayNs,
{
    match dht11_sensor.read() {
        Ok(measurement) => {
            let temperature = measurement.temperature;
            let humidity = measurement.humidity;

            info!(
                "DHT11 reading... Temperature: {}°C, Humidity: {}%",
                temperature, humidity
            );

            Some(DHT11Reading {
                temperature,
                humidity,
            })
        }
        Err(_) => {
            error!("Error reading DHT11 sensor");
            None
        }
    }
}

struct MoistureReading {
    moisture: u8,
    moisture_raw: u16,
    moisture_trigger: bool,
}

async fn read_moisture<'a>(
    adc: &mut Adc<'a, ADC2>,
    pin_analog: &mut AdcPin<GpioPin<11>, ADC2, AdcCalCurve<ADC2>>,
    pin_digial: &Input<'a>,
) -> Option<MoistureReading> {
    if let Some(sample) = sample_adc(adc, pin_analog, "moisture").await {
        info!("Analog Moisture reading: {}", sample);
        let moisture = (normalise_humidity_data(sample) * 100.0) as u8;
        info!("Normalized Moisture reading: {}%", moisture);
        let moisture_trigger = pin_digial.is_high();
        info!("Moisture trigger: {}", moisture_trigger);

        Some(MoistureReading {
            moisture,
            moisture_raw: sample,
            moisture_trigger,
        })
    } else {
        error!("Error calculating moisture sensor average");
        None
    }
}

async fn read_water_level(
    adc: &mut Adc<'_, ADC2>,
    pin: &mut AdcPin<GpioPin<12>, ADC2>,
) -> Option<u16> {
    if let Some(sample) = sample_adc(adc, pin, "water_level").await {
        info!("Water level reading: {}", sample);
        Some(sample)
    } else {
        error!("Error calculating water level sensor average");
        None
    }
}

async fn read_battery(
    adc: &mut Adc<'_, ADC1>,
    pin: &mut AdcPin<GpioPin<4>, ADC1, AdcCalLine<ADC1>>,
) -> Option<u16> {
    match sample_adc(adc, pin, "battery").await {
        Some(sample) => {
            let sample = sample * 2; // The battery voltage divider is 2:1
            if sample < USB_CHARGING_VOLTAGE {
                info!("Battery: {}mV", sample);
                Some(sample)
            } else {
                warn!(
                    "Battery voltage too high - looks we are charging on USB: {}mV",
                    sample
                );
                None
            }
        }
        None => {
            error!("Error calculating battery voltage");
            None
        }
    }
}

/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let clamped = readout.clamp(MOISTURE_MIN, MOISTURE_MAX);

    (MOISTURE_MAX - clamped) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32
}

async fn sample_adc<PIN, ADCI, ADCC>(
    adc: &mut Adc<'_, ADCI>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    name: &str,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess,
    ADCC: AdcCalScheme<ADCI>,
{
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => Some(value),
        Err(e) => {
            error!("Error reading sensor {} {}", name, defmt::Debug2Format(&e));
            None
        }
    }
}
