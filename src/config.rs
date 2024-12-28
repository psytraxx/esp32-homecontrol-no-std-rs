pub const DEVICE_ID: &str = "esp32_breadboard";
// Number of measurements to take before going to deep sleep again
pub const MEASUREMENTS_NEEDED: u64 = 3;
pub const MEASUREMENT_INTERVAL_SECONDS: u64 = 20;
pub const DISPLAY_WIDTH: u16 = 320;
pub const DISPLAY_HEIGHT: u16 = 170;
pub const HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX: &str = "homeassistant";
pub const HOMEASSISTANT_SENSOR_TOPIC: &str = "sensor";
pub const HOMEASSISTANT_SENSOR_SWITCH: &str = "switch";
// ESP will go to deep sleep and not report any data for this duration
pub const DEEP_SLEEP_DURATION: u64 = 600;
