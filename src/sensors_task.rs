use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{
    analog::adc::{
        Adc, AdcCalCurve, AdcCalLine, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation,
        RegisterAccess,
    },
    gpio::{DriveMode, GpioPin, Level, Output, OutputConfig, Pull},
    peripherals::{ADC1, ADC2},
    Blocking,
};
use esp_println::println;
use heapless::Vec;

use crate::{
    config::AWAKE_DURATION_SECONDS,
    dht11::Dht11,
    domain::{Sensor, SensorData, WaterLevel},
    BOOT_COUNT,
};

/// Number of boots between pump trigger events.
/// The pump will be enabled every Nth boot, where N is this value.
const PUMP_TRIGGER_INTERVAL: u32 = 10;
const USB_CHARGING_VOLTAGE: u16 = 4100;
const DHT11_WARMUP_DELAY_MILLISECONDS: u64 = 2000;
const SENSOR_WARMUP_DELAY_MILLISECONDS: u64 = 50;
// in this case we keep 3 samples for averaging - first and last are ignored
const SENSOR_SAMPLE_COUNT: usize = 5;

pub struct SensorPeripherals {
    pub dht11_digital_pin: GpioPin<1>,
    pub battery_pin: GpioPin<4>,
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
    println!("Create");

    let mut adc2_config = AdcConfig::new();
    let mut moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let mut waterlevel_pin = adc2_config.enable_pin(p.water_level_analog_pin, Attenuation::_11dB);
    let mut adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let mut battery_pin = adc1_config
        .enable_pin_with_cal::<GpioPin<4>, AdcCalLine<ADC1>>(p.battery_pin, Attenuation::_11dB);
    let mut adc1 = Adc::new(p.adc1, adc1_config);

    let mut moisture_power_pin =
        Output::new(p.moisture_power_pin, Level::Low, OutputConfig::default());
    let mut water_level_power_pin =
        Output::new(p.water_level_power_pin, Level::Low, OutputConfig::default());

    loop {
        // Collect samples for each sensor type
        let mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
        let mut air_temperature_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
        let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
        let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
        let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

        for i in 0..SENSOR_SAMPLE_COUNT {
            println!("Reading sensor data {}/{}", (i + 1), SENSOR_SAMPLE_COUNT);

            let mut dht11_pin = Output::new(
                &mut p.dht11_digital_pin,
                Level::High,
                OutputConfig::default()
                    .with_drive_mode(DriveMode::OpenDrain)
                    .with_pull(Pull::None),
            )
            .into_flex();
            dht11_pin.enable_input(true);

            let mut dht11_sensor = Dht11::new(dht11_pin, Delay);

            // DHT11 needs a longer initial delay
            Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MILLISECONDS)).await;
            if let Ok(result) = dht11_sensor.read() {
                air_temperature_samples
                    .push(result.temperature)
                    .expect("Too many samples");
                air_humidity_samples
                    .push(result.humidity)
                    .expect("Too many samples");
            }
            // Immediately put pin into low power mode after DHT11 usage
            Output::new(
                &mut p.dht11_digital_pin,
                Level::Low,
                OutputConfig::default()
                    .with_drive_mode(DriveMode::PushPull)
                    .with_pull(Pull::None),
            );

            moisture_power_pin.set_high();

            if let Some(result) = sample_adc(&mut adc2, &mut moisture_pin).await {
                soil_moisture_samples
                    .push(result)
                    .expect("Too many samples");
            } else {
                println!("Error reading soil moisture sensor");
            }

            moisture_power_pin.set_low();

            water_level_power_pin.set_high();

            if let Some(value) = sample_adc(&mut adc2, &mut waterlevel_pin).await {
                water_level_samples.push(value).expect("Too many samples");
            } else {
                println!("Error reading water level sensor");
            }

            water_level_power_pin.set_low();

            if let Some(value) = sample_adc(&mut adc1, &mut battery_pin).await {
                let value = value * 2; // The battery voltage divider is 2:1
                if value < USB_CHARGING_VOLTAGE {
                    battery_voltage_samples
                        .push(value)
                        .expect("Too many samples");
                } else {
                    println!(
                        "Battery voltage too high - looks we are charging on USB: {}mV",
                        value
                    );
                }
            } else {
                println!("Error reading battery voltage");
            }
        }

        // Calculate the average of the samples
        let mut sensor_data = SensorData::default();

        if let Some(avg_air_humidity) = calculate_average(&mut air_humidity_samples) {
            println!("Air humidity: {}%", avg_air_humidity);
            sensor_data
                .data
                .push(Sensor::AirHumidity(avg_air_humidity))
                .expect("Too many samples");
        } else {
            println!(
                "Unable to generate average value of air humidity - we had {} samples",
                air_humidity_samples.len()
            );
        }

        if let Some(avg_air_temperature) = calculate_average(&mut air_temperature_samples) {
            println!("Air temperature: {}°C", avg_air_temperature);
            sensor_data
                .data
                .push(Sensor::AirTemperature(avg_air_temperature))
                .expect("Too many samples");
        } else {
            println!(
                "Unable to generate average value of air temperature, we had {} samples",
                air_temperature_samples.len()
            );
        }

        if let Some(avg_water_level) = calculate_average(&mut water_level_samples) {
            let waterlevel: WaterLevel = avg_water_level.into();
            println!("Pot base water level: {}", waterlevel);
            sensor_data
                .data
                .push(Sensor::WaterLevel(avg_water_level.into()))
                .expect("Too many samples");
        } else {
            println!("Unable to generate average value of water level");
        }

        if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
            println!("Raw Moisture: {}", avg_soil_moisture);
            sensor_data
                .data
                .push(Sensor::SoilMoistureRaw(avg_soil_moisture.into()))
                .expect("Too many samples");

            sensor_data
                .data
                .push(Sensor::SoilMoisture(avg_soil_moisture.into()))
                .expect("Too many samples");
        } else {
            println!("Unable to generate average value of soil moisture");
        }

        let boot_count = unsafe { BOOT_COUNT };

        let pump_enabled = boot_count % PUMP_TRIGGER_INTERVAL == 0;

        sensor_data
            .data
            .push(Sensor::PumpTrigger(pump_enabled))
            .expect("Too many samples");

        if let Some(avg_battery_voltage) = calculate_average(&mut battery_voltage_samples) {
            println!("Battery voltage: {}mV", avg_battery_voltage);
            sensor_data
                .data
                .push(Sensor::BatteryVoltage(avg_battery_voltage))
                .expect("Too many samples");
        }

        // no battery samples - no publish!
        sensor_data.publish = !battery_voltage_samples.is_empty();

        sender.send(sensor_data).await;

        let sampling_period = Duration::from_secs(AWAKE_DURATION_SECONDS);
        Timer::after(sampling_period).await;
    }
}

/// Sample an ADC pin and return the value
async fn sample_adc<PIN, ADCI, ADCC>(
    adc: &mut Adc<'_, ADCI, Blocking>,
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
            println!("Error reading sensor: {:?}", &e);
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
