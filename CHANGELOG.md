# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Fixed
- **Wake-cycle stability (DHT11 failures, WiFi instability, brownout/reset loop)**: a cluster
  of instability appeared after the esp-radio 0.18.0 migration and the WiFi auto-reconnect
  change. Addressed on several fronts:
  - **DHT11 decoupled from the radio**: the bit-banged, timing-sensitive DHT11 read now runs
    *before* the WiFi radio is powered on (`sensors::begin_read`), so radio interrupts can no
    longer corrupt its microsecond edge timing. The ADC sensors still overlap WiFi via `join`
    (`sensors::finish_read`). The DHT read also retries up to `DHT11_MAX_ATTEMPTS` (3) to
    absorb transient glitches instead of dropping the reading on the first checksum failure.
  - **WiFi reconnect backoff**: the connection task replaced its flat 1 s retry with
    exponential backoff (`WIFI_RECONNECT_BACKOFF_START_MS` → `WIFI_RECONNECT_BACKOFF_MAX_MS`,
    reset on successful association), so a flaky AP no longer drives a reconnect storm that
    keeps the radio's TX current spikes busy.
  - **Low-battery guard**: an early battery sample (taken pre-radio) is checked against
    `LOW_BATTERY_CUTOFF_MV` (3300 mV); below it the cycle skips WiFi and the pump, shows the
    readings, and sleeps — breaking the brownout/reset loop that drained a weak LiPo.
  - **Reset-reason diagnostics**: the SoC reset reason is logged at boot and prepended to the
    button-wake display, so a brownout/watchdog/panic reset (vs. a clean `CoreDeepSleep`) is
    visible without a serial cable.

### Fixed
- **WiFi reconnect after link drop**: the connection task now uses `select` to race `WIFI_SIGNAL` against `wait_for_disconnect_async`. Previously, if the AP dropped the link after a successful association, the task was stuck waiting on the stop signal while `link_up` stayed false, causing a `WifiTimeout` error. Now it reconnects automatically within 1 s.

### Changed
- **esp-radio 0.18.0 migration**: `Interface::station()` + `WifiController::new()` replaced with `esp_radio::wifi::new(peripheral, config)` which returns `(WifiController, Interfaces)`. Station interface is now accessed via `interfaces.station` field.

### Changed
- **Architecture: the wake cycle is now one linear async flow** (`main.rs::run_cycle`). The device is a batch job (wake → work → sleep ~1 h), so the task/channel/signal structure was replaced with sequential steps: `join(connect_to_wifi, read_sensors)` → display → `mqtt::connect` → publish → subscribe → poll for pump commands until the awake deadline. Embassy is still used where there is real concurrency: the WiFi tasks, `join` to overlap DHCP with the DHT11 warmup, and `with_deadline`/`with_timeout` for the command window and WiFi bound.
  - `update_task.rs` → `mqtt.rs`: plain async functions (`connect`, `publish`, `subscribe_to_pump_commands`, `wait_for_pump_command`); no task, no display access, no reconnect loop — on broker failure the device just sleeps and retries next wake.
  - `relay_task.rs` → `pump.rs`: `run_pump()` is awaited inline by the wake cycle.
  - `sensors/mod.rs`: `sensor_task` → one-shot `read_sensors()`; the 30 s resample loop never produced a second reading before sleep anyway.
  - The display is owned by `run_cycle` and updated inline; sensor reading + overflow interlock now complete *before* MQTT connects, so the retained-ON race is impossible by construction (supersedes the two-phase subscribe workaround below).
- Pump is now exclusively controlled via Home Assistant; local soil-moisture auto-triggering removed entirely.
- HA pump integration changed from a valve entity to a **switch**: the switch publishes retained `ON`/`OFF` to `{DEVICE_ID}/pump/set`; device resets switch to `OFF` after acting. Retained switch survives device deep sleep — command is never lost. Separate pump state sensor removed; switch state reflects everything.
- When HA presses the button, the relay runs the pump for exactly 10 s then stops automatically.
- Pump start is blocked (state → `blocked`) if the overflow sensor reports water at the pot base at command time.
- `ENABLE_PUMP` signal changed from `Signal<bool>` to `Signal<()>` — fire-and-forget trigger; overflow check lives entirely in `update_task`. *(Signal removed entirely by the linear refactor above.)*
- `DisplayTrait::set_powersave(bool)` replaces the old separate `enable_powersave` / `disable_powersave` methods. Called with `true` before deep sleep (via `DISPLAY_SLEEP`), and `false` lazily on first `write_multiline` call.
- Display on button/boot wake: backlight and pixels enabled on first write. Display on timer wake: initialised in sleep state, never turned on.
- `SensorData.publish` flag removed — sensor data is always published.
- Power: CPU clock reduced from 240 MHz to 80 MHz — sufficient for I/O-bound workload, cuts CPU dynamic power ~3×.
- Power: DHT11 is now read once per wake cycle (one 2 s warmup) instead of once per sample (5 × 2 s = 10 s). Saves ~8 s of active time per wake.
- `enter_deep` in `sleep.rs`: log + 100 ms flush delay moved to caller in `main.rs` so USB CDC output is transmitted before sleep.

### Fixed
- A pump run can no longer be cut short by deep sleep: previously a command arriving late in the awake window started the 10 s relay run on a separate task while `main` entered deep sleep on its own timer (GPIO releases in deep sleep), after HA had already been reset to `OFF`. The run is now awaited inline before sleeping.
- A failed wake cycle (WiFi down, broker unreachable) now logs the error and enters deep sleep instead of `software_reset()` — no more boot-looping with the radio on while the network is down. WiFi connection is additionally bounded by `WIFI_CONNECT_TIMEOUT_SECONDS` (30 s) instead of waiting forever.
- Race condition: retained `ON` pump command was delivered on subscribe before sensor_task sent first reading, causing the pump to fire even when overflow was present. Fix: `update_task` now waits for the first sensor reading before subscribing to the pump topic, so overflow state is always known before any retained command is delivered. No flags needed. *(Now structurally impossible — see the linear refactor above.)*

### Added
- `overflow_detected(adc_mv: u16) -> bool` in `domain.rs` — converts raw ADC reading to overflow state; threshold is 2800 mV (private, measured: ~2217 mV dry, ~3475 mV submerged). Replaces old `WaterLevel` enum and `From<u16>` conversion.
- `WIFI_CONNECT_TIMEOUT_SECONDS` (30 s) in `config.rs` — bounds the WiFi connection attempt so the device sleeps instead of waiting forever.
- `HARDWARE_V2.md` — sensor upgrade plan with confirmed BOM: AHT20+BMP280 combo, Adafruit STEMMA soil sensor, INA219 power monitor; all I2C, STEMMA QT connectors; Rust crate analysis, wiring diagram, firmware checklist.
- Mermaid wiring diagrams for V1 (current) and V2 (planned) hardware added to `README.md`.
- `src/sensors/` module replacing `sensors_task.rs`: `hardware.rs` (peripheral init), `adc.rs` (unified `read_powered_adc_sensor`), `builder.rs` (data assembly), `mod.rs` (one-shot `read_sensors()`).
- Sensor sampling constants (`USB_CHARGING_VOLTAGE_MV`, `DHT11_WARMUP_DELAY_MS`, `SENSOR_WARMUP_DELAY_MS`, `SENSOR_SAMPLE_COUNT`) moved to `config.rs`.
- MQTT discovery payload includes `force_update: true` for numeric sensors — prevents HA recorder from deduplicating unchanged values.

### Removed
- `update_task`, `relay_task`, `sensor_task` and their statics: `CHANNEL` (sensor data), `ENABLE_PUMP` and `DISPLAY_SLEEP` signals. Only `WIFI_SIGNAL` remains (graceful WiFi shutdown).
- `Actuator` enum and `SensorData.actuators` field — pump is no longer triggered from sensor readings.
- `SensorData.publish` field — sensor data is always published.
- `PUMP_TRIGGER_INTERVAL` constant and boot-count modulo scheduling.
- Valve MQTT entity replaced by switch + sensor (see above). **Delete old retained discovery topics `homeassistant/valve/...` and `homeassistant/button/...` from broker after flashing.**
- `WaterLevel` enum replaced by `Sensor::OverflowDetected(bool)` — simpler, no intermediate type. MQTT topic changed from `waterlevel` to `overflow`; published value is `"YES"` (water detected) or `"NO"` (dry). Water level pin now reads without ADC calibration (`()` cal scheme) matching observed raw counts. **Delete the old retained discovery topic `homeassistant/sensor/{DEVICE_ID}_waterlevel/config` from the broker after flashing.**

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
