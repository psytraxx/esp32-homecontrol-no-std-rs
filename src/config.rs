pub const DEVICE_ID: &str = "esp32_breadboard";
pub const AWAKE_DURATION_SECONDS: u64 = 30;
pub const SAMPLING_INTERVAL_SECONDS: u64 = 15;
pub const DISPLAY_WIDTH: u16 = 320;
pub const DISPLAY_HEIGHT: u16 = 170;
pub const HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX: &str = "homeassistant";
pub const HOMEASSISTANT_SENSOR_TOPIC: &str = "sensor";
pub const HOMEASSISTANT_SENSOR_SWITCH: &str = "switch";
// ESP will go to deep sleep and not report any data for this duration
pub const DEEP_SLEEP_DURATION_SECONDS: u64 = 3600 - AWAKE_DURATION_SECONDS;
