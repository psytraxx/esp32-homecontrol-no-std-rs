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

### Architecture
The wake cycle is one **linear async flow** in `main.rs::run_cycle()` — no inter-task channels or signals besides `WIFI_SIGNAL`. The only spawned tasks are the WiFi ones (`net_task` polls the embassy-net runner; `connection` reconnects on drop and disconnects gracefully on `WIFI_SIGNAL`). Sensors, display, MQTT and pump are plain async functions called in order; `main` always reaches `enter_deep`, so a failed cycle (router down, broker unreachable) retries in an hour instead of boot-looping.

### Sleep Cycle
1. Wake from deep sleep (button or timer)
2. Connect WiFi **in parallel with** reading all sensors (`join` — DHCP overlaps the DHT11 warmup; WiFi bounded by `WIFI_CONNECT_TIMEOUT_SECONDS`)
3. Overflow state is now a local — compute `pump_allowed` before MQTT exists (retained ON can never race the interlock)
4. Update display (IP/boot count prepended on button wake)
5. Connect MQTT; publish discovery (first boot only) and sensor state
6. Subscribe to pump command topic (retained ON delivered here)
7. Poll for pump commands until the `AWAKE_DURATION_SECONDS` deadline; an accepted ON resets the switch to OFF, then runs the pump 10 s **inline** (deep sleep can't truncate a run)
8. Disconnect WiFi gracefully, display powersave, sleep ~1 hour

**RTC Fast Memory** (survives deep sleep):
- `BOOT_COUNT` - increments each wake
- `DISCOVERY_MESSAGES_SENT` - prevents republishing discovery

### Hardware: LilyGO T-Display-S3 (V2)

All sensors share a single I2C bus (GPIO3=SDA, GPIO10=SCL, 400 kHz). See `HARDWARE_V2.md` for full BOM and wiring.

| GPIO | Function | Notes |
|------|----------|-------|
| 2 | Pump relay | Active high |
| 3 | I2C SDA | Shared bus for all sensors |
| 10 | I2C SCL | Shared bus for all sensors |
| 12 | Water level | ADC, 11dB attenuation (overflow detector) |
| 14 | Wake button | Deep sleep wake source |
| 15 | Display power | **Must be HIGH** |
| 21 | Water level sensor power | Toggle for reads |
| 38 | Display backlight | PWM capable |
| 6-9, 39-48 | ST7789 display | 8-bit parallel interface |

#### I2C Sensors

| Address | Sensor | Measurements |
|---------|--------|--------------|
| `0x38` | AHT20 | Air temperature (°C), humidity (%) |
| `0x76` | BMP280 | Barometric pressure (hPa) |
| `0x36` | STEMMA Soil | Soil moisture counts (200–2000), soil temperature (°C) |
| `0x40` | INA219 | Battery voltage (mV), current (mA), power (mW) |

### Code Organization

| File | Purpose |
|------|---------|
| [main.rs](src/main.rs) | Entry, peripheral setup, linear wake cycle (`run_cycle`), sleep orchestration |
| [sensors/mod.rs](src/sensors/mod.rs) | `read_sensors()` — one-shot sampling of all sensors |
| [sensors/builder.rs](src/sensors/builder.rs) | Assembles `SensorData` from raw samples |
| [sensors/adc.rs](src/sensors/adc.rs) | Generic ADC sampling, averaging, outlier removal |
| [sensors/hardware.rs](src/sensors/hardware.rs) | Peripheral init for all sensor hardware |
| [mqtt.rs](src/mqtt.rs) | MQTT connect, HA discovery, sensor publishing, pump command window |
| [pump.rs](src/pump.rs) | `run_pump()` — inline 10 s relay run |
| [wifi.rs](src/wifi.rs) | WiFi connection, graceful shutdown |
| [sleep.rs](src/sleep.rs) | Deep sleep with RTC memory, dual wake sources |
| [display.rs](src/display.rs) | ST7789 LCD with embedded-graphics, powersave control |
| [domain.rs](src/domain.rs) | Sensor types (`MoistureLevel`, `OverflowDetected(bool)`), thresholds, `overflow_detected()` |
| [config.rs](src/config.rs) | Timing and sampling constants |

### Data Flow

```
join(connect_to_wifi, read_sensors) → (stack, SensorData)
  → overflow_detected() → pump_allowed (local bool)
  → display update
  → mqtt::connect → publish discovery + sensors
  → subscribe to pump/set topic  ← retained ON delivered here
  → poll until awake deadline:
      ON → reset switch OFF → pump_allowed? → run_pump (10 s, inline)
                            → overflow?     → log blocked
  → display powersave → wifi disconnect → deep sleep
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

Auto-clears retained pump commands after activation. All numeric sensors include `force_update: true` in discovery to prevent HA recorder deduplication.
