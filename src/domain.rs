use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use core::fmt::{Display, Formatter, Result};
use serde::{Deserialize, Serialize};

/// Struct to hold sensor data
#[derive(Default, Debug)]
pub struct SensorData {
    pub data: Vec<Sensor>,
}

impl Display for SensorData {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        self.data.iter().try_for_each(|sensor| {
            let unit = sensor.unit().unwrap_or_default();
            writeln!(f, "{}: {} {}", sensor.name(), sensor.value(), unit)
        })
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
            WaterLevel::Full => write!(f, "Full"),
            WaterLevel::Empty => write!(f, "Empty"),
        }
    }
}

/// Enum to represent different types of sensors
#[derive(Debug)]
pub enum Sensor {
    WaterLevel(WaterLevel),
    AirTemperature(i16),
    AirHumidity(u16),
    SoilMoisture(u16),
    BatteryVoltage(u32),
}

impl Sensor {
    /// Get the unit of the sensor value
    pub fn unit(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) => Some("°C"),
            Sensor::AirHumidity(_) => Some("%"),
            Sensor::BatteryVoltage(_) => Some("mV"),
            Sensor::SoilMoisture(_) => Some("%"),
            _ => None,
        }
    }

    /// Get the device class of the sensor
    /// See https://www.home-assistant.io/integrations/sensor/#device-class
    pub fn device_class(&self) -> Option<&'static str> {
        match self {
            Sensor::AirTemperature(_) => Some("temperature"),
            Sensor::AirHumidity(_) => Some("humidity"),
            Sensor::SoilMoisture(_) => Some("moisture"),
            Sensor::BatteryVoltage(_) => Some("voltage"),
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
        }
    }

    /// Get the value of the sensor as a JSON value
    pub fn value(&self) -> String {
        match self {
            Sensor::AirTemperature(v) => v.to_string(),
            Sensor::AirHumidity(v) => v.to_string(),
            Sensor::SoilMoisture(v) => v.to_string(),
            Sensor::WaterLevel(v) => format!("{}", v),
            Sensor::BatteryVoltage(v) => v.to_string(),
        }
    }
}

impl Display for Sensor {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let unit = self.unit().unwrap_or_default();
        write!(f, "{}: {}{}", self.name(), self.value(), unit)
    }
}
