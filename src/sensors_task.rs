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

/// ADC and GPIO handles
struct SensorHardware {
    adc1: Adc<'static, ADC1, Blocking>,
    adc2: Adc<'static, ADC2, Blocking>,
    moisture_pin: AdcPin<GpioPin<11>, ADC2, AdcCalCurve<ADC2>>,
    waterlevel_pin: AdcPin<GpioPin<12>, ADC2, AdcCalCurve<ADC2>>,
    battery_pin: AdcPin<GpioPin<4>, ADC1, AdcCalLine<ADC1>>,
    moisture_power_pin: Output<'static>,
    water_level_power_pin: Output<'static>,
    dht11_digital_pin: GpioPin<1>,
}

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
    p: SensorPeripherals,
) {
    println!("Initializing sensor task");

    let mut hardware = initialize_hardware(p).await;

    loop {
        let sensor_data = collect_all_sensor_data(&mut hardware).await;
        sender.send(sensor_data).await;

        let sampling_period = Duration::from_secs(AWAKE_DURATION_SECONDS);
        Timer::after(sampling_period).await;
    }
}

/// Initialize all sensor hardware
async fn initialize_hardware(p: SensorPeripherals) -> SensorHardware {
    let mut adc2_config = AdcConfig::new();
    let moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let waterlevel_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.water_level_analog_pin, Attenuation::_11dB);
    let adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let battery_pin = adc1_config
        .enable_pin_with_cal::<GpioPin<4>, AdcCalLine<ADC1>>(p.battery_pin, Attenuation::_11dB);
    let adc1 = Adc::new(p.adc1, adc1_config);

    let moisture_power_pin = Output::new(p.moisture_power_pin, Level::Low, OutputConfig::default());
    let water_level_power_pin =
        Output::new(p.water_level_power_pin, Level::Low, OutputConfig::default());

    SensorHardware {
        adc1,
        adc2,
        moisture_pin,
        waterlevel_pin,
        battery_pin,
        moisture_power_pin,
        water_level_power_pin,
        dht11_digital_pin: p.dht11_digital_pin,
    }
}

/// Collect data from all sensors
async fn collect_all_sensor_data(hardware: &mut SensorHardware) -> SensorData {
    let mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut air_temperature_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

    for i in 0..SENSOR_SAMPLE_COUNT {
        println!("Reading sensor data {}/{}", (i + 1), SENSOR_SAMPLE_COUNT);

        // Read DHT11 (temperature & humidity)
        read_dht11_sensor(
            &mut hardware.dht11_digital_pin,
            &mut air_temperature_samples,
            &mut air_humidity_samples,
        )
        .await;

        // Read soil moisture
        read_moisture_sensor(
            &mut hardware.adc2,
            &mut hardware.moisture_pin,
            &mut hardware.moisture_power_pin,
            &mut soil_moisture_samples,
        )
        .await;

        // Read water level
        read_water_level_sensor(
            &mut hardware.adc2,
            &mut hardware.waterlevel_pin,
            &mut hardware.water_level_power_pin,
            &mut water_level_samples,
        )
        .await;

        // Read battery voltage
        read_battery_voltage(
            &mut hardware.adc1,
            &mut hardware.battery_pin,
            &mut battery_voltage_samples,
        )
        .await;
    }

    build_sensor_data(
        air_humidity_samples,
        air_temperature_samples,
        soil_moisture_samples,
        battery_voltage_samples,
        water_level_samples,
    )
}

/// Read DHT11 temperature and humidity sensor
async fn read_dht11_sensor(
    dht11_pin: &mut GpioPin<1>,
    temperature_samples: &mut Vec<u8, SENSOR_SAMPLE_COUNT>,
    humidity_samples: &mut Vec<u8, SENSOR_SAMPLE_COUNT>,
) {
    let mut pin = Output::new(
        dht11_pin,
        Level::High,
        OutputConfig::default()
            .with_drive_mode(DriveMode::OpenDrain)
            .with_pull(Pull::None),
    )
    .into_flex();
    pin.enable_input(true);

    let mut dht11_sensor = Dht11::new(pin, Delay);
    Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MILLISECONDS)).await;

    if let Ok(result) = dht11_sensor.read() {
        let _ = temperature_samples.push(result.temperature);
        let _ = humidity_samples.push(result.humidity);
    }

    // Pin is now owned by dht11_sensor and will be dropped automatically
}

/// Read soil moisture sensor
async fn read_moisture_sensor(
    adc: &mut Adc<'_, ADC2, Blocking>,
    pin: &mut AdcPin<GpioPin<11>, ADC2, AdcCalCurve<ADC2>>,
    power_pin: &mut Output<'_>,
    samples: &mut Vec<u16, SENSOR_SAMPLE_COUNT>,
) {
    power_pin.set_high();

    if let Some(result) = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await {
        let _ = samples.push(result);
    } else {
        println!("Error reading soil moisture sensor");
    }

    power_pin.set_low();
}

/// Read water level sensor
async fn read_water_level_sensor(
    adc: &mut Adc<'_, ADC2, Blocking>,
    pin: &mut AdcPin<GpioPin<12>, ADC2, AdcCalCurve<ADC2>>,
    power_pin: &mut Output<'_>,
    samples: &mut Vec<u16, SENSOR_SAMPLE_COUNT>,
) {
    power_pin.set_high();

    if let Some(value) = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await {
        let _ = samples.push(value);
    } else {
        println!("Error reading water level sensor");
    }

    power_pin.set_low();
}

/// Read battery voltage
async fn read_battery_voltage(
    adc: &mut Adc<'_, ADC1, Blocking>,
    pin: &mut AdcPin<GpioPin<4>, ADC1, AdcCalLine<ADC1>>,
    samples: &mut Vec<u16, SENSOR_SAMPLE_COUNT>,
) {
    if let Some(value) = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await {
        let value = value * 2; // The battery voltage divider is 2:1
        if value < USB_CHARGING_VOLTAGE {
            let _ = samples.push(value);
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

/// Sample ADC with configurable warmup delay
async fn sample_adc_with_warmup<PIN, ADCI, ADCC>(
    adc: &mut Adc<'_, ADCI, Blocking>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    warmup_ms: u64,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess,
    ADCC: AdcCalScheme<ADCI>,
{
    Timer::after(Duration::from_millis(warmup_ms)).await;
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => Some(value),
        Err(e) => {
            println!("Error reading sensor: {:?}", &e);
            None
        }
    }
}

/// Build final sensor data structure
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
        println!("Air humidity: {}%", avg_air_humidity);
        let _ = sensor_data.data.push(Sensor::AirHumidity(avg_air_humidity));
    } else {
        println!(
            "Unable to generate average value of air humidity - we had {} samples",
            air_humidity_samples.len()
        );
    }

    // Process air temperature
    if let Some(avg_air_temperature) = calculate_average(&mut air_temperature_samples) {
        println!("Air temperature: {}°C", avg_air_temperature);
        let _ = sensor_data
            .data
            .push(Sensor::AirTemperature(avg_air_temperature));
    } else {
        println!(
            "Unable to generate average value of air temperature, we had {} samples",
            air_temperature_samples.len()
        );
    }

    // Process water level
    if let Some(avg_water_level) = calculate_average(&mut water_level_samples) {
        let waterlevel: WaterLevel = avg_water_level.into();
        println!("Pot base water level: {}", waterlevel);
        let _ = sensor_data
            .data
            .push(Sensor::WaterLevel(avg_water_level.into()));
    } else {
        println!("Unable to generate average value of water level");
    }

    // Process soil moisture
    if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
        println!("Raw Moisture: {}", avg_soil_moisture);
        let _ = sensor_data
            .data
            .push(Sensor::SoilMoistureRaw(avg_soil_moisture.into()));
        let _ = sensor_data
            .data
            .push(Sensor::SoilMoisture(avg_soil_moisture.into()));
    } else {
        println!("Unable to generate average value of soil moisture");
    }

    // Add pump trigger logic
    let boot_count = unsafe { BOOT_COUNT };
    let pump_enabled = boot_count % PUMP_TRIGGER_INTERVAL == 0;
    let _ = sensor_data.data.push(Sensor::PumpTrigger(pump_enabled));

    // Process battery voltage
    if let Some(avg_battery_voltage) = calculate_average(&mut battery_voltage_samples) {
        println!("Battery voltage: {}mV", avg_battery_voltage);
        let _ = sensor_data
            .data
            .push(Sensor::BatteryVoltage(avg_battery_voltage));
    }

    // Only publish if we have battery samples
    sensor_data.publish = !battery_voltage_samples.is_empty();

    sensor_data
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
