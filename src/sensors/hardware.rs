use embassy_embedded_hal::shared_bus::asynch::i2c::I2cDevice;
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, mutex::Mutex};
use embassy_time::Delay;
use embedded_aht20::{Aht20, DEFAULT_I2C_ADDRESS as AHT20_ADDRESS};
use esp_hal::{
    Async, Blocking,
    analog::adc::{Adc, AdcCalCurve, AdcConfig, AdcPin, Attenuation},
    gpio::{Level, Output, OutputConfig},
    i2c::master::{Config, I2c},
    peripherals::{ADC2, GPIO12, GPIO21, I2C0},
};
use ina219::{AsyncIna219, address::Address, calibration::UnCalibrated};
use log::{error, warn};
use static_cell::StaticCell;

/// BMP280 I2C address (SDO low → 0x76).
const BMP280_ADDRESS: u8 = 0x76;
/// INA219 default I2C address (A0=GND, A1=GND).
const INA219_ADDRESS: u8 = 0x40;

type SharedI2c = Mutex<NoopRawMutex, I2c<'static, Async>>;

// Static storage for the shared async I2C bus.
static I2C_BUS: StaticCell<SharedI2c> = StaticCell::new();

/// I2C and ADC hardware handles — private to the sensors module.
pub(super) struct SensorHardware<'a> {
    pub(super) aht20: Option<Aht20<I2cDevice<'a, NoopRawMutex, I2c<'static, Async>>, Delay>>,
    pub(super) bme280:
        bme280_rs::AsyncBme280<I2cDevice<'a, NoopRawMutex, I2c<'static, Async>>, Delay>,
    pub(super) ina219:
        Option<AsyncIna219<I2cDevice<'a, NoopRawMutex, I2c<'static, Async>>, UnCalibrated>>,
    pub(super) i2c_bus: &'a SharedI2c,
    pub(super) waterlevel_pin: AdcPin<GPIO12<'a>, ADC2<'a>, AdcCalCurve<ADC2<'a>>>,
    pub(super) water_level_power_pin: Output<'a>,
    pub(super) adc2: Adc<'a, ADC2<'a>, Blocking>,
}

/// Peripheral bundle passed from main.rs into the sensor task.
pub struct SensorPeripherals {
    pub i2c: I2C0<'static>,
    pub sda: esp_hal::peripherals::GPIO3<'static>,
    pub scl: esp_hal::peripherals::GPIO10<'static>,
    pub water_level_analog_pin: GPIO12<'static>,
    pub water_level_power_pin: GPIO21<'static>,
    pub adc2: ADC2<'static>,
}

/// Initialize all sensor hardware from the peripheral bundle.
pub(super) async fn initialize_hardware(p: SensorPeripherals) -> SensorHardware<'static> {
    let i2c = I2c::new(
        p.i2c,
        Config::default().with_frequency(esp_hal::time::Rate::from_khz(400)),
    )
    .unwrap()
    .with_sda(p.sda)
    .with_scl(p.scl)
    .into_async();

    let i2c_bus: &'static SharedI2c = I2C_BUS.init(Mutex::new(i2c));

    let aht_dev = I2cDevice::new(i2c_bus);
    let bme_dev = I2cDevice::new(i2c_bus);
    let ina_dev = I2cDevice::new(i2c_bus);

    let aht20 = match Aht20::new(aht_dev, AHT20_ADDRESS, Delay).await {
        Ok(s) => Some(s),
        Err(e) => {
            error!("AHT20 init failed (sensor may not be connected): {:?}", e);
            None
        }
    };

    let mut bme280 = bme280_rs::AsyncBme280::new_with_address(bme_dev, BMP280_ADDRESS, Delay);
    if let Err(e) = bme280.init().await {
        warn!(
            "BMP280 init failed (sensor may not be connected yet): {:?}",
            e
        );
    }

    let ina219 = match AsyncIna219::new(ina_dev, Address::from_byte(INA219_ADDRESS).unwrap()).await
    {
        Ok(s) => Some(s),
        Err(e) => {
            error!("INA219 init failed (sensor may not be connected): {:?}", e);
            None
        }
    };

    let mut adc2_config = AdcConfig::new();
    let waterlevel_pin = adc2_config
        .enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(p.water_level_analog_pin, Attenuation::_11dB);
    let adc2 = Adc::new(p.adc2, adc2_config);

    let water_level_power_pin =
        Output::new(p.water_level_power_pin, Level::Low, OutputConfig::default());

    SensorHardware {
        aht20,
        bme280,
        ina219,
        i2c_bus,
        waterlevel_pin,
        water_level_power_pin,
        adc2,
    }
}
