use alloc::vec::Vec;
use defmt::{error, info, warn};
use dht11::Dht11;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{
    analog::adc::{
        Adc, AdcCalCurve, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation, RegisterAccess,
    },
    gpio::{GpioPin, Input, Level, OutputOpenDrain, Pull},
    peripherals::{ADC1, ADC2},
    prelude::nb,
};

use crate::{
    config::SAMPLING_INTERVAL_SECONDS,
    domain::{Sensor, SensorData},
};

const DHT11_MAX_RETRIES: u8 = 3;
const DHT11_RETRY_DELAY_MS: u64 = 2000;
const MOISTURE_MIN: u16 = 1600;
const MOISTURE_MAX: u16 = 2100;
const USB_CHARGING_VOLTAGE: u16 = 4200;
const SENSOR_WARMUP_DELAY_MILLISECONDS: u64 = 10;
const MAX_SENSOR_SAMPLE_COUNT: usize = 8;

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
    let mut dht11_sensor = Dht11::new(dht11_pin);

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config.enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(
        p.moisture_analog_pin,
        Attenuation::Attenuation11dB,
    );
    let mut waterlevel_pin =
        adc2_config.enable_pin(p.water_level_pin, Attenuation::Attenuation11dB);
    let mut adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let mut battery_pin = adc1_config.enable_pin_with_cal::<GpioPin<4>, AdcCalCurve<ADC1>>(
        p.battery_pin,
        Attenuation::Attenuation11dB,
    );
    let mut adc1 = Adc::new(p.adc1, adc1_config);

    let digital_input = Input::new(p.moisture_digital_pin, esp_hal::gpio::Pull::None);

    loop {
        info!("Reading sensors");
        let mut sensor_data = SensorData::default();

        read_dht11(&mut dht11_sensor, &mut sensor_data).await;
        read_moisture(
            &mut adc2,
            &mut moisture_pin,
            &digital_input,
            &mut sensor_data,
        )
        .await;
        read_water_level(&mut adc2, &mut waterlevel_pin, &mut sensor_data).await;
        read_battery(&mut adc1, &mut battery_pin, &mut sensor_data).await;

        sender.send(sensor_data).await;
        // next reading will be the device came back from deep sleep
        let sampling_period = Duration::from_secs(SAMPLING_INTERVAL_SECONDS);
        Timer::after(sampling_period).await;
    }
}

async fn read_dht11<'a>(
    dht11_sensor: &mut Dht11<OutputOpenDrain<'a>>,
    sensor_data: &mut SensorData,
) {
    Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
    for attempt in 1..=DHT11_MAX_RETRIES {
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
                error!(
                    "Error reading DHT11 sensor (attempt {}/{})",
                    attempt, DHT11_MAX_RETRIES
                );
                Timer::after(Duration::from_millis(DHT11_RETRY_DELAY_MS)).await;
            }
        }
    }
}

async fn read_moisture<'a>(
    adc: &mut Adc<'a, ADC2>,
    pin_analog: &mut AdcPin<GpioPin<11>, ADC2, AdcCalCurve<ADC2>>,
    pin_digial: &Input<'a>,
    sensor_data: &mut SensorData,
) {
    if let Some(sample) = sample_adc(adc, pin_analog, "moisture").await {
        info!("Analog Moisture reading: {}", sample);
        sensor_data.data.push(Sensor::SoilMoistureRaw(sample));

        let moisture = (normalise_humidity_data(sample) * 100.0) as u8;
        info!("Normalized Moisture reading: {}%", moisture);
        sensor_data.data.push(Sensor::SoilMoisture(moisture));

        let moisture_trigger = pin_digial.is_high();

        sensor_data.data.push(Sensor::PumpTrigger(moisture_trigger));

        info!("Moisture trigger: {}", moisture_trigger);
    } else {
        error!("Error calculating moisture sensor average");
    }
}

async fn read_water_level<'a>(
    adc: &mut Adc<'a, ADC2>,
    pin: &mut AdcPin<GpioPin<12>, ADC2>,
    sensor_data: &mut SensorData,
) {
    if let Some(sample) = sample_adc(adc, pin, "water_level").await {
        info!("Water level reading: {}", sample);
        sensor_data.data.push(Sensor::WaterLevel(sample.into()));
    } else {
        error!("Error calculating water level sensor average");
    }
}

async fn read_battery<'a>(
    adc: &mut Adc<'a, ADC1>,
    pin: &mut AdcPin<GpioPin<4>, ADC1, AdcCalCurve<ADC1>>,
    sensor_data: &mut SensorData,
) {
    match sample_adc(adc, pin, "battery").await {
        Some(sample) => {
            let sample = sample * 2; // The battery voltage divider is 2:1
            if sample < USB_CHARGING_VOLTAGE {
                info!("Battery: {}mV", sample);
                sensor_data.data.push(Sensor::BatteryVoltage(sample));
            } else {
                warn!(
                    "Battery voltage too high - looks we are charging on USB: {}mV",
                    sample
                );
            }
        }
        None => {
            error!("Error calculating battery voltage");
        }
    }
}

/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let clamped = readout.clamp(MOISTURE_MIN, MOISTURE_MAX);

    (MOISTURE_MAX - clamped) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32
}

async fn sample_adc<'a, PIN, ADCI, ADCC>(
    adc: &mut Adc<'a, ADCI>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    name: &str,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess,
    ADCC: AdcCalScheme<ADCI>,
{
    let mut samples = Vec::with_capacity(MAX_SENSOR_SAMPLE_COUNT);

    // Collect samples with a warm-up delay
    while samples.len() < MAX_SENSOR_SAMPLE_COUNT {
        Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
        match nb::block!(adc.read_oneshot(pin)) {
            Ok(value) => samples.push(value),
            Err(_) => error!("Error reading sensor {}", name),
        }
    }

    //info!("Samples: {} {}", defmt::Debug2Format(&samples), name);

    if samples.len() <= 2 {
        warn!("Not enough samples to calculate average for {}", name);
        return None;
    }

    // Sort and remove outliers
    samples.sort_unstable();
    let samples = &samples[1..samples.len() - 1]; // Remove lowest and highest values

    samples
        .iter()
        .map(|&x| x as u32)
        .sum::<u32>()
        .checked_div(samples.len() as u32)
        .map(|avg| avg as u16)
        .or_else(|| {
            warn!("Error calculating moisture sensor average for {}", name);
            None
        })
}
