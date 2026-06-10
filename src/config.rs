pub const DEVICE_ID: &str = "esp32_breadboard";
pub const AWAKE_DURATION_SECONDS: u64 = 30;
pub const DISPLAY_WIDTH: u16 = 320;
pub const DISPLAY_HEIGHT: u16 = 170;
pub const HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX: &str = "homeassistant";
pub const HOMEASSISTANT_SENSOR_TOPIC: &str = "sensor";
pub const HOMEASSISTANT_SWITCH_TOPIC: &str = "switch";
// ESP will go to deep sleep and not report any data for this duration
pub const DEEP_SLEEP_DURATION_SECONDS: u64 = 3600 - AWAKE_DURATION_SECONDS;
/// Give up on WiFi after this long and go back to sleep instead of waiting forever
pub const WIFI_CONNECT_TIMEOUT_SECONDS: u64 = 30;

/// Set to false to suppress all MQTT publishing (useful during development on USB power).
pub const MQTT_PUBLISH_ENABLED: bool = true;

// Sensor sampling configuration
/// Warmup delay for powered ADC sensors (moisture, water level) before each read (ms)
pub const SENSOR_WARMUP_DELAY_MS: u64 = 50;
/// Number of samples to collect per water-level sensor per cycle (min/max trimmed, rest averaged)
pub const SENSOR_SAMPLE_COUNT: usize = 5;
