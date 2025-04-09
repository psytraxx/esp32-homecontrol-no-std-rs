use alloc::string::{String, ToString};
use core::fmt::{Display, Formatter, Result};
use heapless::Vec;
use serde::{Deserialize, Serialize};

const WATER_LEVEL_THRESHOLD: u16 = 3000;
//soil is wet
const MOISTURE_MIN: u16 = 800;
// soil is dry
const MOISTURE_MAX: u16 = 2150;
//  more than 80% is wet
const MOISTURE_WET_THRESHOLD: f32 = 0.8;
// less than 15% is dry
const MOISTURE_DRY_THRESHOLD: f32 = 0.15;

/// Struct to hold sensor data
#[derive(Default, Debug)]
pub struct SensorData {
    pub data: Vec<Sensor, 7>,
}

impl Display for SensorData {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        self.data.iter().try_for_each(|sensor| {
            let unit = sensor.unit().unwrap_or_default();
            writeln!(f, "{}: {} {}", sensor.name(), sensor.value(), unit)
        })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub enum MoistureLevel {
    Wet,
    Moist,
    Dry,
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
        let clamped = clamp_soil_moisture(value);

        let value = (MOISTURE_MAX - clamped) as f32 / (MOISTURE_MAX - MOISTURE_MIN) as f32;

        match value {
            p if p > MOISTURE_WET_THRESHOLD => Self::Wet,
            p if p < MOISTURE_DRY_THRESHOLD => Self::Dry,
            _ => Self::Moist,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub enum WaterLevel {
    Full,
    Empty,
}

impl Display for WaterLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Self::Full => write!(f, "Full"),
            Self::Empty => write!(f, "Empty"),
        }
    }
}

impl From<u16> for WaterLevel {
    fn from(value: u16) -> Self {
        if value < WATER_LEVEL_THRESHOLD {
            Self::Empty
        } else {
            Self::Full
        }
    }
}

/// Enum to represent different types of sensors
#[derive(Debug)]
pub enum Sensor {
    WaterLevel(WaterLevel),
    AirTemperature(u8),
    AirHumidity(u8),
    SoilMoisture(MoistureLevel),
    BatteryVoltage(u16),
    SoilMoistureRaw(SoilMoistureRawLevel),
    PumpTrigger(bool),
}

#[derive(Debug)]
pub struct SoilMoistureRawLevel(u16);

impl From<u16> for SoilMoistureRawLevel {
    fn from(value: u16) -> Self {
        Self(clamp_soil_moisture(value))
    }
}

impl Display for SoilMoistureRawLevel {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.0)
    }
}

impl Sensor {
    /// Get the unit of the sensor value
    pub fn unit(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) => Some("°C"),
            Sensor::AirHumidity(_) => Some("%"),
            Sensor::BatteryVoltage(_) => Some("mV"),
            Sensor::SoilMoistureRaw(_) => Some("mV"),
            _ => None,
        }
    }

    /// Get the device class of the sensor
    /// See https://www.home-assistant.io/integrations/sensor/#device-class
    pub fn device_class(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) => Some("temperature"),
            Sensor::AirHumidity(_) => Some("humidity"),
            Sensor::BatteryVoltage(_) => Some("voltage"),
            Sensor::SoilMoistureRaw(_) => Some("voltage"),
            _ => None,
        }
    }

    /// Get the MQTT topic for the sensor
    pub fn topic(&self) -> &'static str {
        match self {
            Sensor::AirTemperature(_) => "temperature",
            Sensor::AirHumidity(_) => "humidity",
            Sensor::SoilMoisture(_) => "moisture",
            Sensor::WaterLevel(_) => "waterlevel",
            Sensor::BatteryVoltage(_) => "batteryvoltage",
            Sensor::SoilMoistureRaw(_) => "moistureraw",
            Sensor::PumpTrigger(_) => "pumptrigger",
        }
    }

    /// Get the name of the sensor
    pub fn name(&self) -> &'static str {
        match self {
            Sensor::AirTemperature(_) => "Room temperature",
            Sensor::AirHumidity(_) => "Room humidity",
            Sensor::SoilMoisture(_) => "Soil moisture",
            Sensor::WaterLevel(_) => "Water level",
            Sensor::BatteryVoltage(_) => "Battery voltage",
            Sensor::SoilMoistureRaw(_) => "Soil moisture (mV)",
            Sensor::PumpTrigger(_) => "Pump trigger",
        }
    }

    /// Get the value of the sensor as a JSON value
    pub fn value(&self) -> String {
        match self {
            Sensor::AirTemperature(v) => v.to_string(),
            Sensor::AirHumidity(v) => v.to_string(),
            Sensor::SoilMoisture(v) => v.to_string(),
            Sensor::WaterLevel(v) => v.to_string(),
            Sensor::BatteryVoltage(v) => v.to_string(),
            Sensor::SoilMoistureRaw(v) => v.to_string(),
            Sensor::PumpTrigger(v) => v.to_string(),
        }
    }
}

impl Display for Sensor {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let unit = self.unit().unwrap_or_default();
        write!(f, "{}: {}{}", self.name(), self.value(), unit)
    }
}

fn clamp_soil_moisture(value: u16) -> u16 {
    value.clamp(MOISTURE_MIN, MOISTURE_MAX)
}
