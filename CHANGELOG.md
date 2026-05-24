# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Added
- `HARDWARE_V2.md` — sensor upgrade plan with confirmed BOM: AHT20+BMP280 combo, Adafruit STEMMA soil sensor, INA219 power monitor; all I2C, STEMMA QT connectors; Rust crate analysis, wiring diagram, firmware checklist
- Mermaid wiring diagrams for V1 (current) and V2 (planned) hardware added to `README.md`
- `Actuator` enum in `domain.rs` — mirrors the `Sensor` enum pattern; currently holds `Pump(bool)`; adding a new actuator (e.g. humidifier) is just a new variant + bump of `Vec` capacity
- `src/sensors/` module replacing `sensors_task.rs`: `hardware.rs` (peripheral init), `adc.rs` (generic ADC sampling, unified `read_powered_adc_sensor` eliminates moisture/water-level duplication), `builder.rs` (data assembly), `mod.rs` (thin Embassy task entry)
- Sensor sampling constants (`PUMP_TRIGGER_INTERVAL`, `USB_CHARGING_VOLTAGE_MV`, `DHT11_WARMUP_DELAY_MS`, `SENSOR_WARMUP_DELAY_MS`, `SENSOR_SAMPLE_COUNT`) moved to `config.rs`

### Changed
- `update_task.rs`: fixed MQTT event loop starvation — the inner select now uses `select3` to race sensor data, MQTT poll, and `STOP_UPDATE_TASK_SIGNAL` simultaneously; when the stop signal fires the display `enable_powersave()` is called immediately before the task exits, preserving power-save behaviour without blocking MQTT for 30 s
- `update_task.rs` / `main.rs`: introduced `DISPLAY_POWERSAVE_SIGNAL` (separate from `STOP_WIFI_SIGNAL`) — Embassy `Signal` stores only one waker, so sharing a single signal between two tasks meant only one task was reliably notified; each task now has its own signal fired together from `main`
- `enter_deep` in `sleep.rs`: removed log statement that fired immediately before `rtc.sleep()` (USB CDC has no chance to flush it); caller in `main.rs` now logs + awaits 100 ms before entering sleep so all pending output is transmitted

### Documentation
- `README.md`: added explicit note that MQTT publishing is intentionally suppressed when battery voltage exceeds `USB_CHARGING_VOLTAGE_MV` (board powered via USB); this is a design decision, not a bug
- MQTT discovery payload now includes `force_update: true` for numeric sensors — prevents Home Assistant recorder from deduplicating unchanged values, giving full hourly history resolution
- Updated `CLAUDE.md` — added changelog and code quality workflow requirements
- `PumpTrigger(bool)` removed from the `Sensor` enum — it was incorrectly appearing as a Home Assistant sensor entity; pump state now lives in `SensorData.actuators: Vec<Actuator, 1>`
- `update_task.rs` reads pump state from `sensor_data.actuators` instead of iterating `sensor_data.data` for `Sensor::PumpTrigger`

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
