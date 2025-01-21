use alloc::vec;
use defmt::{error, info, warn};
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Duration, Timer};
use esp_hal::{
    analog::adc::{
        Adc, AdcCalCurve, AdcCalLine, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation,
        RegisterAccess,
    },
    delay::Delay,
    gpio::{GpioPin, Input, Level, Output, OutputOpenDrain, Pull},
    peripherals::{ADC1, ADC2},
};

use crate::{
    config::AWAKE_DURATION_SECONDS,
    dht11::Dht11,
    domain::{Sensor, SensorData, WaterLevel},
};

const MOISTURE_MIN: u16 = 1700;
const MOISTURE_MAX: u16 = 2000;
const USB_CHARGING_VOLTAGE: u16 = 4200;
const DHT11_WARMUP_DELAY_MILLISECONDS: u64 = 2000;
const SENSOR_WARMUP_DELAY_MILLISECONDS: u64 = 50;
const SENSOR_SAMPLE_COUNT: usize = 5;

pub struct SensorPeripherals {
    pub dht11_digital_pin: GpioPin<1>,
    pub battery_pin: GpioPin<4>,
    pub moisture_digital_pin: GpioPin<10>,
    pub moisture_power_pin: GpioPin<16>,
    pub moisture_analog_pin: GpioPin<11>,
    pub water_level_analog_pin: GpioPin<12>,
    pub water_level_power_pin: GpioPin<21>,
    pub adc1: ADC1,
    pub adc2: ADC2,
}

#[embassy_executor::task]
pub async fn sensor_task(
    sender: Sender<'static, NoopRawMutex, SensorData, 3>,
    mut p: SensorPeripherals,
) {
    info!("Create");

    let delay = Delay::new();

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let mut waterlevel_pin = adc2_config.enable_pin(p.water_level_analog_pin, Attenuation::_11dB);
    let mut adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let mut battery_pin = adc1_config
        .enable_pin_with_cal::<GpioPin<4>, AdcCalLine<ADC1>>(p.battery_pin, Attenuation::_11dB);
    let mut adc1 = Adc::new(p.adc1, adc1_config);

    let moisture_input_pin = Input::new(p.moisture_digital_pin, esp_hal::gpio::Pull::None);

    let mut moisture_power_pin = Output::new(p.moisture_power_pin, Level::Low);
    let mut water_level_power_pin = Output::new(p.water_level_power_pin, Level::Low);

    loop {
        // Collect samples for each sensor type
        let mut air_humidity_samples: vec::Vec<u8> = vec![];
        let mut air_temperature_samples: vec::Vec<u8> = vec![];
        let mut soil_moisture_samples: vec::Vec<u16> = vec![];
        let mut moisture_pin_sample_count: usize = 0;
        let mut battery_voltage_samples: vec::Vec<u16> = vec![];
        let mut water_level_samples: vec::Vec<u16> = vec![];

        // Power on the sensors
        moisture_power_pin.set_high();
        water_level_power_pin.set_high();

        for i in 0..SENSOR_SAMPLE_COUNT {
            info!("Reading sensor data {}/{}", (i + 1), SENSOR_SAMPLE_COUNT);

            let dht11_pin = OutputOpenDrain::new(&mut p.dht11_digital_pin, Level::High, Pull::None);
            let mut dht11_sensor: Dht11<OutputOpenDrain<'_>, Delay> = Dht11::new(dht11_pin, delay);

            // DHT11 needs a longer initial delay
            Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MILLISECONDS)).await;
            if let Ok(result) = dht11_sensor.read() {
                air_temperature_samples.push(result.temperature);
                air_humidity_samples.push(result.humidity);
            }

            if let Some(result) = sample_adc(&mut adc2, &mut moisture_pin).await {
                soil_moisture_samples.push(result);
            } else {
                warn!("Error reading soil moisture sensor");
            }

            if moisture_input_pin.is_high() {
                moisture_pin_sample_count += 1;
            }

            if let Some(value) = sample_adc(&mut adc2, &mut waterlevel_pin).await {
                water_level_samples.push(value);
            } else {
                warn!("Error reading water level sensor");
            }

            if let Some(value) = sample_adc(&mut adc1, &mut battery_pin).await {
                let value = value * 2; // The battery voltage divider is 2:1
                if value < USB_CHARGING_VOLTAGE {
                    battery_voltage_samples.push(value);
                } else {
                    warn!(
                        "Battery voltage too high - looks we are charging on USB: {}mV",
                        value
                    );
                }
            } else {
                warn!("Error reading battery voltage");
            }
        }

        // Calculate the average of the samples
        let mut sensor_data = SensorData::default();

        if let Some(avg) = calculate_average(&mut air_humidity_samples) {
            info!("Air humidity: {}%", avg);
            sensor_data.data.push(Sensor::AirHumidity(avg));
        } else {
            warn!("Unable to generate average value of air humidity");
        }

        if let Some(avg) = calculate_average(&mut air_temperature_samples) {
            info!("Air temperature: {}°C", avg);
            sensor_data.data.push(Sensor::AirTemperature(avg));
        } else {
            warn!("Unable to generate average value of air temperature");
        }

        if let Some(avg) = calculate_average(&mut water_level_samples) {
            let waterlevel: WaterLevel = avg.into();
            info!("Water level: {}", waterlevel);
            sensor_data.data.push(Sensor::WaterLevel(avg.into()));
        } else {
            warn!("Unable to generate average value of water level");
        }

        if let Some(avg) = calculate_average(&mut soil_moisture_samples) {
            info!("Raw Moisture: {}", avg);
            sensor_data.data.push(Sensor::SoilMoistureRaw(avg));
            let moisture = (normalise_humidity_data(avg) * 100.0) as u8;
            info!("Normalized Moisture: {}%", moisture);
            sensor_data.data.push(Sensor::SoilMoisture(moisture));
        } else {
            warn!("Unable to generate average value of soil moisture");
        }

        if let Some(avg) = calculate_average(&mut battery_voltage_samples) {
            info!("Battery voltage: {}mV", avg);
            sensor_data.data.push(Sensor::BatteryVoltage(avg));
        } else {
            warn!("Error measuring battery voltage");
        }

        // Set pump trigger to true if majority of samples indicated it should be on
        let pump_trigger = moisture_pin_sample_count > SENSOR_SAMPLE_COUNT / 2;
        sensor_data.data.push(Sensor::PumpTrigger(pump_trigger));

        sender.send(sensor_data).await;

        // Power off the sensors
        moisture_power_pin.set_low();
        water_level_power_pin.set_low();

        let sampling_period = Duration::from_secs(AWAKE_DURATION_SECONDS);
        Timer::after(sampling_period).await;
    }
}

/// Sample an ADC pin and return the value
async fn sample_adc<PIN, ADCI, ADCC>(
    adc: &mut Adc<'_, ADCI>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess,
    ADCC: AdcCalScheme<ADCI>,
{
    // Wait for the sensor to warm up
    Timer::after(Duration::from_millis(SENSOR_WARMUP_DELAY_MILLISECONDS)).await;
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => Some(value),
        Err(e) => {
            error!("Error reading sensor: {}", defmt::Debug2Format(&e));
            None
        }
    }
}

/// Calculate the average of a slice of samples, removing the highest and lowest values
fn calculate_average<T>(samples: &mut [T]) -> Option<T>
where
    T: Copy + Ord + Into<u32>,
    u32: TryInto<T>,
{
    if samples.len() <= 2 {
        return None;
    }

    // Sort and remove outliers
    samples.sort_unstable();
    let samples = &samples[1..samples.len() - 1]; // Remove lowest and highest values

    let sum: u32 = samples.iter().map(|&x| x.into()).sum();
    sum.checked_div(samples.len() as u32)
        .and_then(|avg| avg.try_into().ok())
        .or(None)
}

/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let clamped = readout.clamp(MOISTURE_MIN, MOISTURE_MAX);

    (MOISTURE_MAX - clamped) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32
}
