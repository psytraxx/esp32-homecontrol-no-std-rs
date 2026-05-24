pub const DEVICE_ID: &str = "esp32_breadboard";
pub const AWAKE_DURATION_SECONDS: u64 = 30;
pub const DISPLAY_WIDTH: u16 = 320;
pub const DISPLAY_HEIGHT: u16 = 170;
pub const HOMEASSISTANT_DISCOVERY_TOPIC_PREFIX: &str = "homeassistant";
pub const HOMEASSISTANT_SENSOR_TOPIC: &str = "sensor";
pub const HOMEASSISTANT_VALVE_TOPIC: &str = "valve";
// ESP will go to deep sleep and not report any data for this duration
pub const DEEP_SLEEP_DURATION_SECONDS: u64 = 3600 - AWAKE_DURATION_SECONDS;

// Sensor sampling configuration
/// Number of boots between pump trigger events (pump runs every Nth boot)
pub const PUMP_TRIGGER_INTERVAL: u32 = 10;
/// Battery voltage above this threshold (mV) indicates USB charging — skip reading
pub const USB_CHARGING_VOLTAGE_MV: u16 = 4100;
/// Warmup delay for DHT11 before each read (ms)
pub const DHT11_WARMUP_DELAY_MS: u64 = 2000;
/// Warmup delay for powered ADC sensors (moisture, water level) before each read (ms)
pub const SENSOR_WARMUP_DELAY_MS: u64 = 50;
/// Number of samples to collect per sensor per cycle (min/max trimmed, rest averaged)
pub const SENSOR_SAMPLE_COUNT: usize = 5;
