# CLAUDE.md

ESP32-based plant watering system for the LilyGO T-Display-S3 board using no-std Rust with Embassy async framework.

> **For general ESP32 Rust development guidance**, see `.claude/skills/esp32-rust-embedded/SKILL.md`

## Required Workflow

Before committing any change:
1. **`cargo fmt`** — format all code (run first, always)
2. **`cargo clippy`** — fix all warnings before proceeding
3. **Update `CHANGELOG.md`** — add entry under `[Unreleased]` describing what changed and why

Never skip these steps. Clippy warnings are treated as errors.

---

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
- **sensor_task**: Reads all sensors once per wake cycle → sends via channel to update_task
- **update_task**: MQTT publishing, display updates, HA discovery, pump state reporting
- **relay_task**: Runs pump for 10 s on `ENABLE_PUMP` signal
- **connect_to_wifi / net_task**: WiFi management, graceful shutdown via `STOP_WIFI_SIGNAL`

### Sleep Cycle
1. Wake from deep sleep (button or timer)
2. Connect WiFi
3. Read sensors (Phase 1 — establishes overflow state)
4. Subscribe to pump command topic (retained ON delivered with overflow state already known)
5. Publish discovery (first boot only)
6. Publish sensors; process pump command if pending
7. Disconnect WiFi gracefully
8. Sleep ~1 hour

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
| [main.rs](src/main.rs) | Entry, peripheral setup, task spawning, global signals |
| [sensors/mod.rs](src/sensors/mod.rs) | Embassy task entry for sensor sampling |
| [sensors/builder.rs](src/sensors/builder.rs) | Assembles `SensorData` from raw samples |
| [sensors/adc.rs](src/sensors/adc.rs) | Generic ADC sampling, averaging, outlier removal |
| [sensors/hardware.rs](src/sensors/hardware.rs) | Peripheral init for all sensor hardware |
| [update_task.rs](src/update_task.rs) | MQTT client, HA discovery, pump state publishing |
| [relay_task.rs](src/relay_task.rs) | 10 s pump run on `ENABLE_PUMP` signal |
| [wifi.rs](src/wifi.rs) | WiFi connection, graceful shutdown |
| [sleep.rs](src/sleep.rs) | Deep sleep with RTC memory, dual wake sources |
| [display.rs](src/display.rs) | ST7789 LCD with embedded-graphics, powersave control |
| [domain.rs](src/domain.rs) | Sensor types (`MoistureLevel`, `OverflowDetected(bool)`), thresholds, `overflow_detected()` |
| [config.rs](src/config.rs) | Timing and sampling constants |

### Data Flow

```
sensor_task → SensorData → Channel → update_task (Phase 1)
  → overflow_detected() → pump_allowed bool
  → MQTT publish + display update
  → subscribe to pump/set topic  ← retained ON delivered here

update_task (Phase 2, inner loop):
  MQTT pump/set ON → overflow? no  → reset switch OFF + ENABLE_PUMP.signal(())
                                              ↓
                                        relay_task → pump GPIO (10 s)

                   → overflow? yes → reset switch OFF, log blocked
```

### MQTT Integration

**Discovery topics (retained):**
- `homeassistant/sensor/{DEVICE_ID}_{sensor}/config` — sensor entities
- `homeassistant/switch/{DEVICE_ID}_pump/config` — pump switch (retained ON/OFF)
**State topics:**
- `{DEVICE_ID}/{sensor}` — sensor readings (`{"value": "..."}`)
- `{DEVICE_ID}/overflow` — `{"value": "YES"}` (water detected) or `{"value": "NO"}` (dry); raw ADC threshold 2800 (~2217 dry, ~3475 submerged)

**Command topics:**
- `{DEVICE_ID}/pump/set` — receives `ON` from HA switch (retained); device resets to `OFF` after acting
