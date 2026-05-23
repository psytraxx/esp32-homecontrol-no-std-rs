use alloc::string::{String, ToString};
use core::fmt::{Display, Formatter, Result};
use heapless::Vec;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

const OVERFLOW_THRESHOLD: u16 = 2800;

/// STEMMA soil sensor capacitive range (200 = dry, 2000 = saturated).
const MOISTURE_MIN: u16 = 200;
const MOISTURE_MAX: u16 = 2000;
/// Normalised fraction above which soil is considered Wet (>70%).
const MOISTURE_WET_THRESHOLD: f32 = 0.7;
/// Normalised fraction below which soil is considered Dry (<20%).
const MOISTURE_DRY_THRESHOLD: f32 = 0.2;

/// Struct to hold sensor data
#[derive(Default, Debug)]
pub struct SensorData {
    pub data: Vec<Sensor, 10>,
}

impl Display for SensorData {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        self.data.iter().try_for_each(|sensor| {
            let unit = sensor.unit().unwrap_or_default();
            writeln!(f, "{}: {} {}", sensor.name(), sensor.value(), unit)
        })
    }
}

/// Represents the qualitative state of soil moisture as interpreted from sensor readings.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum MoistureLevel {
    Wet,   // Soil is wet
    Moist, // Soil is moist (intermediate)
    #[default]
    Dry, // Soil is dry
}

impl Display for MoistureLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Wet => write!(f, "Wet"),
            Self::Moist => write!(f, "Moist"),
            Self::Dry => write!(f, "Dry"),
        }
    }
}

impl From<u16> for MoistureLevel {
    fn from(value: u16) -> Self {
        let clamped = value.clamp(MOISTURE_MIN, MOISTURE_MAX);
        // Normalise to 0.0 (dry) .. 1.0 (wet)
        let normalised = (clamped - MOISTURE_MIN) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32;

        if normalised > MOISTURE_WET_THRESHOLD {
            Self::Wet
        } else if normalised < MOISTURE_DRY_THRESHOLD {
            Self::Dry
        } else {
            Self::Moist
        }
    }
}

/// Represents all supported sensor types and their current readings.
#[derive(Debug, EnumIter)]
pub enum Sensor {
    /// true = water at pot base, pump blocked
    OverflowDetected(bool),
    /// Air temperature from AHT20 in °C (±0.3 °C)
    AirTemperature(f32),
    /// Relative humidity from AHT20 in % (±2 %RH)
    AirHumidity(f32),
    /// Barometric pressure from BMP280 in hPa (±1 hPa)
    AirPressure(f32),
    /// Soil temperature from STEMMA sensor in °C
    SoilTemperature(f32),
    /// Raw capacitive soil moisture counts from STEMMA (200 = dry, 2000 = wet)
    SoilMoisture(u16),
    /// Qualitative soil moisture level derived from raw counts
    SoilMoistureLevel(MoistureLevel),
    /// Battery bus voltage from INA219 in mV
    BatteryVoltage(u16),
    /// Battery current from INA219 in mA (positive = charging, negative = discharging)
    BatteryCurrent(f32),
    /// Battery power from INA219 in mW
    BatteryPower(f32),
}

impl Sensor {
    /// Get the unit of the sensor value
    pub fn unit(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) => Some("°C"),
            Sensor::AirHumidity(_) => Some("%"),
            Sensor::AirPressure(_) => Some("hPa"),
            Sensor::SoilTemperature(_) => Some("°C"),
            Sensor::BatteryVoltage(_) => Some("mV"),
            Sensor::BatteryCurrent(_) => Some("mA"),
            Sensor::BatteryPower(_) => Some("mW"),
            _ => None,
        }
    }

    /// Get the Home Assistant device class for the sensor.
    /// See https://www.home-assistant.io/integrations/sensor/#device-class
    pub fn device_class(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) | Sensor::SoilTemperature(_) => Some("temperature"),
            Sensor::AirHumidity(_) => Some("humidity"),
            Sensor::AirPressure(_) => Some("atmospheric_pressure"),
            Sensor::BatteryVoltage(_) => Some("voltage"),
            Sensor::BatteryCurrent(_) => Some("current"),
            Sensor::BatteryPower(_) => Some("power"),
            _ => None,
        }
    }

    /// Get the MQTT topic suffix for the sensor
    pub fn topic(&self) -> &'static str {
        match self {
            Sensor::OverflowDetected(_) => "overflow",
            Sensor::AirTemperature(_) => "temperature",
            Sensor::AirHumidity(_) => "humidity",
            Sensor::AirPressure(_) => "pressure",
            Sensor::SoilTemperature(_) => "soiltemperature",
            Sensor::SoilMoisture(_) => "moistureraw",
            Sensor::SoilMoistureLevel(_) => "moisture",
            Sensor::BatteryVoltage(_) => "batteryvoltage",
            Sensor::BatteryCurrent(_) => "batterycurrent",
            Sensor::BatteryPower(_) => "batterypower",
        }
    }

    /// Get the human-readable name of the sensor
    pub fn name(&self) -> &'static str {
        match self {
            Sensor::OverflowDetected(_) => "Overflow detected",
            Sensor::AirTemperature(_) => "Room temperature",
            Sensor::AirHumidity(_) => "Room humidity",
            Sensor::AirPressure(_) => "Air pressure",
            Sensor::SoilTemperature(_) => "Soil temperature",
            Sensor::SoilMoisture(_) => "Soil moisture",
            Sensor::SoilMoistureLevel(_) => "Soil moisture level",
            Sensor::BatteryVoltage(_) => "Battery voltage",
            Sensor::BatteryCurrent(_) => "Battery current",
            Sensor::BatteryPower(_) => "Battery power",
        }
    }

    /// Get the sensor value as a string for MQTT publishing
    pub fn value(&self) -> String {
        match self {
            Sensor::OverflowDetected(v) => if *v { "YES" } else { "NO" }.to_string(),
            Sensor::AirTemperature(v) => format_f32(*v),
            Sensor::AirHumidity(v) => format_f32(*v),
            Sensor::AirPressure(v) => format_f32(*v),
            Sensor::SoilTemperature(v) => format_f32(*v),
            Sensor::SoilMoisture(v) => v.to_string(),
            Sensor::SoilMoistureLevel(v) => v.to_string(),
            Sensor::BatteryVoltage(v) => v.to_string(),
            Sensor::BatteryCurrent(v) => format_f32(*v),
            Sensor::BatteryPower(v) => format_f32(*v),
        }
    }
}

impl Display for Sensor {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let unit = self.unit().unwrap_or_default();
        write!(f, "{}: {}{}", self.name(), self.value(), unit)
    }
}

pub fn overflow_detected(adc_mv: u16) -> bool {
    adc_mv > OVERFLOW_THRESHOLD // ~2217 mV dry, ~3475 mV submerged
}

/// Format a float to 1 decimal place without std
fn format_f32(v: f32) -> String {
    let integer = v as i32;
    let frac = ((v - integer as f32).abs() * 10.0) as u32;
    alloc::format!("{}.{}", integer, frac)
}
