use defmt::{error, info};
use dht11::Dht11;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{
    analog::adc::{Adc, AdcCalCurve, AdcConfig, AdcPin, Attenuation},
    gpio::{AnyPin, GpioPin, Level, OutputOpenDrain, Pull},
    peripherals::{ADC1, ADC2},
    prelude::nb,
};

use crate::{
    clock::Clock,
    config::MEASUREMENT_INTERVAL_SECONDS,
    domain::{Sensor, SensorData, WaterLevel},
};

/// Interval to wait for sensor warmup
const WARMUP_INTERVAL: Duration = Duration::from_millis(10);

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
        info!("Reading sensors");
        let mut sensor_data = SensorData::default();

        Timer::after(WARMUP_INTERVAL).await;
        read_dht11(&mut dht11_sensor, &mut sensor_data);

        Timer::after(WARMUP_INTERVAL).await;
        read_moisture(&mut adc2, &mut moisture_pin, &mut sensor_data);

        Timer::after(WARMUP_INTERVAL).await;
        read_water_level(&mut adc2, &mut waterlevel_pin, &mut sensor_data);

        Timer::after(WARMUP_INTERVAL).await;
        read_battery(&mut adc1, &mut battery_pin, &mut sensor_data);

        sender.send(sensor_data).await;

        let sampling_period = Duration::from_secs(MEASUREMENT_INTERVAL_SECONDS);
        let wait_interval = clock.duration_to_next_rounded_wakeup(sampling_period);
        info!(
            "Sensor data published to channel, waiting {:?} seconds",
            wait_interval.as_secs()
        );
        Timer::after(wait_interval).await;
    }
}

fn read_dht11(dht11_sensor: &mut Dht11<OutputOpenDrain<AnyPin>>, sensor_data: &mut SensorData) {
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
        }
        Err(_) => error!("Error reading DHT11 sensor"),
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
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(raw) => {
            let voltage = raw as u32 * 2;
            let max_voltage = 4300;
            let voltage = voltage.min(max_voltage);
            let percent = (voltage * 100) / max_voltage;

            info!("Battery voltage: {}mV ({}%)", voltage, percent);

            sensor_data.data.push(Sensor::BatteryVoltage(voltage));
            sensor_data.data.push(Sensor::BatteryPercent(percent));
        }
        Err(_) => error!("Error reading battery voltage"),
    }
}

/// The hw390 moisture sensor returns a value between 3000 and 4095.
/// From our measurements, the sensor was in water at 3000 and in air at 4095.
/// We normalize the values to be between 0 and 1, with 1 representing water and 0 representing air.
fn normalise_humidity_data(readout: u16) -> f32 {
    let min_value = 2010;
    let max_value = 3895;
    let normalized_value =
        (readout.saturating_sub(min_value)) as f32 / (max_value - min_value) as f32;
    // Invert the value
    1.0 - normalized_value
}

impl From<u16> for WaterLevel {
    fn from(value: u16) -> Self {
        if value < 1000 {
            WaterLevel::Empty
        } else {
            WaterLevel::Full
        }
    }
}
