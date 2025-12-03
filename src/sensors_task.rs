use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Sender};
use embassy_time::{Delay, Duration, Timer};
use esp_hal::{
    analog::adc::{
        Adc, AdcCalCurve, AdcCalLine, AdcCalScheme, AdcChannel, AdcConfig, AdcPin, Attenuation,
        RegisterAccess,
    },
    gpio::{DriveMode, Level, Output, OutputConfig, Pull},
    peripherals::{ADC1, ADC2, GPIO1, GPIO11, GPIO12, GPIO16, GPIO21, GPIO4},
    Blocking,
};
use esp_println::println;
use heapless::Vec;

use crate::{
    config::AWAKE_DURATION_SECONDS,
    dht11::{Dht11, Measurement},
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
struct SensorHardware<'a> {
    adc1: Adc<'a, ADC1<'a>, Blocking>,
    adc2: Adc<'a, ADC2<'a>, Blocking>,
    moisture_pin: AdcPin<GPIO11<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    waterlevel_pin: AdcPin<GPIO12<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    battery_pin: AdcPin<GPIO4<'a>, ADC1<'a>, AdcCalLine<ADC1<'a>>>,
    moisture_power_pin: Output<'a>,
    water_level_power_pin: Output<'a>,
    dht11_pin: esp_hal::gpio::Flex<'a>,
}

pub struct SensorPeripherals {
    pub dht11_digital_pin: GPIO1<'static>,
    pub battery_pin: GPIO4<'static>,
    pub moisture_power_pin: GPIO16<'static>,
    pub moisture_analog_pin: GPIO11<'static>,
    pub water_level_analog_pin: GPIO12<'static>,
    pub water_level_power_pin: GPIO21<'static>,
    pub adc1: ADC1<'static>,
    pub adc2: ADC2<'static>,
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
async fn initialize_hardware(p: SensorPeripherals) -> SensorHardware<'static> {
    let mut adc2_config = AdcConfig::new();
    let moisture_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.moisture_analog_pin, Attenuation::_11dB);
    let waterlevel_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.water_level_analog_pin, Attenuation::_11dB);
    let adc2 = Adc::new(p.adc2, adc2_config);

    let mut adc1_config = AdcConfig::new();
    let battery_pin = adc1_config.enable_pin_with_cal(p.battery_pin, Attenuation::_11dB);
    let adc1 = Adc::new(p.adc1, adc1_config);

    let moisture_power_pin = Output::new(p.moisture_power_pin, Level::Low, OutputConfig::default());
    let water_level_power_pin =
        Output::new(p.water_level_power_pin, Level::Low, OutputConfig::default());

    // Setup DHT11 pin once
    let mut dht11_pin = Output::new(
        p.dht11_digital_pin,
        Level::High,
        OutputConfig::default()
            .with_drive_mode(DriveMode::OpenDrain)
            .with_pull(Pull::None),
    )
    .into_flex();
    dht11_pin.set_input_enable(true);

    SensorHardware {
        adc1,
        adc2,
        moisture_pin,
        waterlevel_pin,
        battery_pin,
        moisture_power_pin,
        water_level_power_pin,
        dht11_pin,
    }
}

/// Collect data from all sensors
async fn collect_all_sensor_data(hardware: &mut SensorHardware<'static>) -> SensorData {
    let mut air_humidity_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut air_temperature_samples: Vec<u8, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut soil_moisture_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut battery_voltage_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();
    let mut water_level_samples: Vec<u16, SENSOR_SAMPLE_COUNT> = Vec::new();

    for i in 0..SENSOR_SAMPLE_COUNT {
        println!("Reading sensor data {}/{}", (i + 1), SENSOR_SAMPLE_COUNT);

        // Read DHT11 (temperature & humidity)
        if let Some(messurement) = read_dht11_sensor(&mut hardware.dht11_pin).await {
            if air_temperature_samples
                .push(messurement.temperature)
                .is_err()
            {
                println!("Failed to push AirTemperature to sensor_data");
            }
            if air_humidity_samples.push(messurement.humidity).is_err() {
                println!("Failed to push AirHumidity to sensor_data");
            }
        }

        // Read soil moisture
        if let Some(moisture) = read_moisture_sensor(
            &mut hardware.adc2,
            &mut hardware.moisture_pin,
            &mut hardware.moisture_power_pin,
        )
        .await
        {
            if soil_moisture_samples.push(moisture).is_err() {
                println!("Failed to push SoilMoisture to sensor_data");
            }
        }

        // Read water level
        if let Some(water_level) = read_water_level_sensor(
            &mut hardware.adc2,
            &mut hardware.waterlevel_pin,
            &mut hardware.water_level_power_pin,
        )
        .await
        {
            if water_level_samples.push(water_level).is_err() {
                println!("Failed to push WaterLevel to sensor_data");
            }
        }

        // Read battery voltage
        if let Some(battery_voltage) =
            read_battery_voltage(&mut hardware.adc1, &mut hardware.battery_pin).await
        {
            if battery_voltage_samples.push(battery_voltage).is_err() {
                println!("Failed to push BatteryVoltage to sensor_data");
            }
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

/// Read DHT11 temperature and humidity sensor
async fn read_dht11_sensor(dht11_pin: &mut esp_hal::gpio::Flex<'static>) -> Option<Measurement> {
    let mut dht11_sensor = Dht11::new(dht11_pin, Delay);
    Timer::after(Duration::from_millis(DHT11_WARMUP_DELAY_MILLISECONDS)).await;

    dht11_sensor.read().ok()
}

/// Read soil moisture sensor
async fn read_moisture_sensor<'a>(
    adc: &mut Adc<'a, ADC2<'a>, Blocking>,
    pin: &mut AdcPin<GPIO11<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    power_pin: &mut Output<'a>,
) -> Option<u16> {
    power_pin.set_high();

    let result = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await;

    power_pin.set_low();
    result
}

/// Read water level sensor
async fn read_water_level_sensor<'a>(
    adc: &mut Adc<'a, ADC2<'a>, Blocking>,
    pin: &mut AdcPin<GPIO12<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    power_pin: &mut Output<'a>,
) -> Option<u16> {
    power_pin.set_high();

    let result = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await;

    power_pin.set_low();
    result
}

/// Read battery voltage
async fn read_battery_voltage<'a>(
    adc: &mut Adc<'a, ADC1<'a>, Blocking>,
    pin: &mut AdcPin<GPIO4<'a>, ADC1<'a>, AdcCalLine<ADC1<'a>>>,
) -> Option<u16> {
    let value = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MILLISECONDS).await? * 2;

    if value < USB_CHARGING_VOLTAGE {
        Some(value)
    } else {
        println!(
            "Battery voltage too high - looks we are charging on USB: {}mV",
            value
        );
        None
    }
}

/// Sample ADC with configurable warmup delay
async fn sample_adc_with_warmup<'a, PIN, ADCI, ADCC>(
    adc: &mut Adc<'a, ADCI, Blocking>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    warmup_ms: u64,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess + 'a,
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
        if sensor_data
            .data
            .push(Sensor::AirHumidity(avg_air_humidity))
            .is_err()
        {
            println!("Failed to push AirHumidity to sensor_data");
        }
    } else {
        println!(
            "Unable to generate average value of air humidity - we had {} samples",
            air_humidity_samples.len()
        );
    }

    // Process air temperature
    if let Some(avg_air_temperature) = calculate_average(&mut air_temperature_samples) {
        println!("Air temperature: {}°C", avg_air_temperature);
        if sensor_data
            .data
            .push(Sensor::AirTemperature(avg_air_temperature))
            .is_err()
        {
            println!("Failed to push AirTemperature to sensor_data");
        }
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
        if sensor_data
            .data
            .push(Sensor::WaterLevel(avg_water_level.into()))
            .is_err()
        {
            println!("Failed to push WaterLevel to sensor_data");
        }
    } else {
        println!("Unable to generate average value of water level");
    }

    // Process soil moisture
    if let Some(avg_soil_moisture) = calculate_average(&mut soil_moisture_samples) {
        println!("Raw Moisture: {}", avg_soil_moisture);
        if sensor_data
            .data
            .push(Sensor::SoilMoistureRaw(avg_soil_moisture.into()))
            .is_err()
        {
            println!("Failed to push SoilMoistureRaw to sensor_data");
        }
        if sensor_data
            .data
            .push(Sensor::SoilMoisture(avg_soil_moisture.into()))
            .is_err()
        {
            println!("Failed to push SoilMoisture to sensor_data");
        }
    } else {
        println!("Unable to generate average value of soil moisture");
    }

    // Add pump trigger logic
    let boot_count = BOOT_COUNT.get();
    let pump_enabled = boot_count.is_multiple_of(PUMP_TRIGGER_INTERVAL);
    if sensor_data
        .data
        .push(Sensor::PumpTrigger(pump_enabled))
        .is_err()
    {
        println!("Failed to push PumpTrigger to sensor_data");
    }

    // Process battery voltage
    if let Some(avg_battery_voltage) = calculate_average(&mut battery_voltage_samples) {
        println!("Battery voltage: {}mV", avg_battery_voltage);
        if sensor_data
            .data
            .push(Sensor::BatteryVoltage(avg_battery_voltage))
            .is_err()
        {
            println!("Failed to push BatteryVoltage to sensor_data");
        }
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
