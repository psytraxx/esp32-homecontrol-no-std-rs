# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Changed
- Pump is now exclusively controlled via Home Assistant; local soil-moisture auto-triggering removed entirely.
- HA pump integration changed from a valve entity to a **button + sensor** pair: pressing the button sends `PRESS` to `{DEVICE_ID}/pump/set`; pump state (`idle` / `running` / `blocked`) is published to `{DEVICE_ID}/pump/state` and shown via a sensor entity. No retained messages, no toggle reset needed.
- When HA presses the button, the relay runs the pump for exactly 10 s then stops automatically.
- Pump start is blocked (state → `blocked`) if the drainage water-level sensor reports overflow at command time.
- `ENABLE_PUMP` signal changed from `Signal<bool>` to `Signal<()>` — fire-and-forget trigger; overflow check lives entirely in `update_task`.
- `PUMP_STATE: Signal<bool>` added — `relay_task` signals `true` on start and `false` on stop; `update_task` publishes `running` / `idle` accordingly.
- `DisplayTrait::set_powersave(bool)` replaces the old separate `enable_powersave` / `disable_powersave` methods. Called with `true` before deep sleep (via `DISPLAY_SLEEP`), and `false` lazily on first `write_multiline` call.
- Display on button/boot wake: backlight and pixels enabled on first write. Display on timer wake: initialised in sleep state, never turned on.
- `SensorData.publish` flag removed — sensor data is always published.
- Power: CPU clock reduced from 240 MHz to 80 MHz — sufficient for I/O-bound workload, cuts CPU dynamic power ~3×.
- Power: DHT11 is now read once per wake cycle (one 2 s warmup) instead of once per sample (5 × 2 s = 10 s). Saves ~8 s of active time per wake.
- `enter_deep` in `sleep.rs`: log + 100 ms flush delay moved to caller in `main.rs` so USB CDC output is transmitted before sleep.

### Added
- `DISPLAY_SLEEP` — fired by `main` before deep sleep so `update_task` can call `set_powersave(true)` on the display it owns.
- `HARDWARE_V2.md` — sensor upgrade plan with confirmed BOM: AHT20+BMP280 combo, Adafruit STEMMA soil sensor, INA219 power monitor; all I2C, STEMMA QT connectors; Rust crate analysis, wiring diagram, firmware checklist.
- Mermaid wiring diagrams for V1 (current) and V2 (planned) hardware added to `README.md`.
- `src/sensors/` module replacing `sensors_task.rs`: `hardware.rs` (peripheral init), `adc.rs` (unified `read_powered_adc_sensor`), `builder.rs` (data assembly), `mod.rs` (Embassy task entry).
- Sensor sampling constants (`USB_CHARGING_VOLTAGE_MV`, `DHT11_WARMUP_DELAY_MS`, `SENSOR_WARMUP_DELAY_MS`, `SENSOR_SAMPLE_COUNT`) moved to `config.rs`.
- MQTT discovery payload includes `force_update: true` for numeric sensors — prevents HA recorder from deduplicating unchanged values.

### Removed
- `Actuator` enum and `SensorData.actuators` field — pump is no longer triggered from sensor readings.
- `SensorData.publish` field — sensor data is always published.
- `PUMP_TRIGGER_INTERVAL` constant and boot-count modulo scheduling.
- Valve MQTT entity replaced by button + sensor (see above).

---

## [0.1.0] — 2026-05-23

### Added
- Initial release
- DHT11 temperature/humidity sensing (bit-bang)
- Capacitive soil moisture sensor (ADC, 5-sample averaged with outlier removal)
- Water level overflow detection (ADC binary threshold)
- Battery voltage monitoring (ADC with ×2 voltage divider)
- ST7789 display via 8-bit parallel interface
- WiFi connection with DHCP
- MQTT integration with Home Assistant auto-discovery
- Deep sleep (~59.5 min) with RTC memory persistence (`BOOT_COUNT`, `DISCOVERY_MESSAGES_SENT`)
- Dual wake sources: RTC timer + GPIO14 button
- Remote pump control via MQTT (`pump/set`)
- Auto-clear of retained pump command after activation
- `force_update` in MQTT sensor discovery payload (HA recorder fix)
