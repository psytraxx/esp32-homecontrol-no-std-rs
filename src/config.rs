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

/// WiFi reconnect backoff bounds. After a link drop or failed association the
/// connection task waits this long before retrying, doubling up to the cap.
/// Prevents a flaky AP from triggering a ~1s reconnect storm that keeps the
/// radio (and its TX current spikes) busy — important on battery power.
pub const WIFI_RECONNECT_BACKOFF_START_MS: u64 = 1000;
pub const WIFI_RECONNECT_BACKOFF_MAX_MS: u64 = 30_000;

/// Battery voltage below this (mV) means the cell is too weak to safely power
/// the WiFi radio and pump. The cycle skips WiFi/pump and sleeps to avoid a
/// brownout/reset loop that would drain the battery further.
pub const LOW_BATTERY_CUTOFF_MV: u16 = 3300;

/// How many times to retry the (timing-sensitive, bit-banged) DHT11 read before
/// giving up for this cycle.
pub const DHT11_MAX_ATTEMPTS: usize = 3;

/// Set to false to suppress all MQTT publishing (useful during development on USB power).
pub const MQTT_PUBLISH_ENABLED: bool = true;

// Sensor sampling configuration
/// Battery voltage above this threshold (mV) indicates USB charging — skip reading
pub const USB_CHARGING_VOLTAGE_MV: u16 = 4100;
/// Warmup delay for DHT11 before each read (ms)
pub const DHT11_WARMUP_DELAY_MS: u64 = 1000;
/// Warmup delay for powered ADC sensors (moisture, water level) before each read (ms)
pub const SENSOR_WARMUP_DELAY_MS: u64 = 50;
/// Number of samples to collect per sensor per cycle (min/max trimmed, rest averaged)
pub const SENSOR_SAMPLE_COUNT: usize = 5;
