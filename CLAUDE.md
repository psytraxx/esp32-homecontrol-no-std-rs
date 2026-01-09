# CLAUDE.md

ESP32-based plant watering system for the LilyGO T-Display-S3 board using no-std Rust with Embassy async framework.

> **For general ESP32 Rust development guidance**, see `.claude/skills/esp32-rust-embedded/SKILL.md`

## Quick Start

```bash
# Setup (one-time)
espup install
. $HOME/export-esp.sh
cp .env.dist .env  # Edit with WiFi/MQTT credentials

# Build and flash
./run.sh
```

## Project-Specific Details

### Environment Variables (compile-time via `env!()`)
- `WIFI_SSID`, `WIFI_PSK` - WiFi credentials
- `MQTT_HOSTNAME`, `MQTT_USERNAME`, `MQTT_PASSWORD`, `MQTT_PORT` - MQTT broker config

### Task Architecture
- **sensor_task**: Reads sensors every 5s → sends via channel
- **update_task**: MQTT publishing, display updates, Home Assistant discovery
- **relay_task**: Water pump control via signal
- **connection/net_task**: WiFi management, graceful shutdown

### Sleep Cycle
1. Wake from deep sleep (button or timer)
2. Connect WiFi
3. Publish discovery (first boot only) 
4. Read & publish sensors for 30s
5. Disconnect WiFi gracefully
6. Sleep ~1 hour

**RTC Fast Memory** (survives deep sleep):
- `BOOT_COUNT` - increments each wake
- `DISCOVERY_MESSAGES_SENT` - prevents republishing discovery

### Hardware: LilyGO T-Display-S3

| GPIO | Function | Notes |
|------|----------|-------|
| 1 | DHT11 sensor | Temperature/humidity |
| 2 | Pump relay | Active high |
| 4 | Battery voltage | ADC, 11dB attenuation |
| 11 | Soil moisture | ADC, 11dB attenuation |
| 12 | Water level | ADC, 11dB attenuation |
| 14 | Wake button | Deep sleep wake source |
| 15 | Display power | **Must be HIGH** |
| 16 | Moisture sensor power | Toggle for reads |
| 21 | Water level sensor power | Toggle for reads |
| 38 | Display backlight | PWM capable |
| 6-9, 39-48 | ST7789 display | 8-bit parallel interface |

### Code Organization

| File | Purpose |
|------|---------|
| [main.rs](src/main.rs) | Entry, peripheral setup, task spawning |
| [sensors_task.rs](src/sensors_task.rs) | ADC reads, averaging, outlier removal |
| [update_task.rs](src/update_task.rs) | MQTT client with reconnect loop, HA discovery |
| [relay_task.rs](src/relay_task.rs) | Pump control with signal pattern |
| [wifi.rs](src/wifi.rs) | WiFi connection, graceful shutdown |
| [sleep.rs](src/sleep.rs) | Deep sleep with RTC memory, dual wake sources |
| [display.rs](src/display.rs) | ST7789 LCD with embedded-graphics |
| [domain.rs](src/domain.rs) | Sensor types (MoistureLevel, WaterLevel), thresholds |
| [dht11.rs](src/dht11.rs) | DHT11 bit-bang protocol |
| [config.rs](src/config.rs) | Timing constants (awake/sleep duration) |

### Data Flow

```
Sensors → ADC (sensors_task) → domain types → Channel → 
update_task → MQTT publish + display update
         ↓
MQTT command → relay_task → pump GPIO
```

### MQTT Integration

**Published topics:**
- `homeassistant/sensor/{DEVICE_ID}/{sensor}/config` - Discovery
- `homeassistant/sensor/{DEVICE_ID}/{sensor}/state` - Readings

**Subscribed topics:**
- `homeassistant/switch/{DEVICE_ID}/pump/set` - Pump commands (ON/OFF)

Auto-clears retained pump commands after activation.
