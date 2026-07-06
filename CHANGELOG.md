# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Fixed
- **Boot count rendered as a categorical colored bar in HA**: `state_class` was only set when a
  sensor had a unit, so the unitless boot counter was published with no `state_class` and Home
  Assistant charted it with the discrete/enum renderer (one color per integer value, forever)
  instead of a numeric line. `state_class` is now chosen per sensor via `Sensor::state_class()`,
  independent of the unit — `total_increasing` for the boot counter, `measurement` for the
  continuous numeric sensors, and none for the qualitative overflow/moisture-level sensors.
- **RTC memory was reset on every wake (`BOOT_COUNT`/`DISCOVERY_MESSAGES_SENT` never persisted)**:
  both statics used `#[ram(unstable(rtc_fast))]` *without* the `persistent` option. Plain
  `rtc_fast` places the value in RTC fast RAM but re-applies its initializer on every reset,
  including deep-sleep wake — so `BOOT_COUNT` read back `0` and `DISCOVERY_MESSAGES_SENT` read
  back `false` on each cycle (confirmed remotely: the published boot count stayed at `0` across
  deep-sleep wakes). Both now use `#[ram(unstable(rtc_fast, persistent))]`, which zeroes the
  memory once on first boot and then preserves it across resets. `persistent` requires a type
  with no invalid bit patterns (`esp_hal::Persistable`), which `bool` does not implement, so the
  discovery flag is stored as `u32` (0/1). The `RtcCell` wrapper — incompatible with `persistent`
  because `UnsafeCell` isn't `Persistable` — was removed in favor of plain `static mut` accessed
  through safe helpers in `rtc_memory.rs` (sound because Embassy is single-core cooperative and
  deep sleep halts execution, so access is strictly sequential).
- **Pump interlock was fail-open**: `pump_allowed` was computed as "no `OverflowDetected(true)`
  reading present", so a cycle where the water-level average failed (sensor fault, ADC error)
  and the overflow sensor was never pushed into `SensorData` would silently allow the pump to
  run. The interlock now requires an explicit `OverflowDetected(false)` reading to allow the
  pump — a missing overflow reading blocks the pump instead of defaulting to allow, and logs a
  warning.
- **MQTT session had no upper bound on awake time**: `mqtt::connect` (DNS query, TCP connect,
  broker handshake) and the initial `publish`/`subscribe` calls had no timeout of their own —
  only the pump-command poll loop respected the awake deadline. A stalled broker or blackholed
  connection could keep the radio on indefinitely instead of the device going back to sleep and
  retrying next hour. The whole MQTT session is now wrapped in `with_deadline` against the same
  cycle deadline used for the command window.
- **Water-level ADC pin had no calibration curve**: only the moisture and battery pins used
  `enable_pin_with_cal`; the water-level pin used a plain `enable_pin`, giving it raw
  (uncalibrated) ADC counts on the same channel type as the other two, now-calibrated pins.
  Added `AdcCalCurve` calibration to match.
- **ADC attenuation bug**: moisture and water-level pins were enabled on a separate `AdcConfig`
  that was never passed to `Adc::new` — only the config passed to `Adc::new` has its
  attenuations programmed into hardware, so both channels were silently read without the
  intended 11 dB attenuation. All three ADC1 pins (moisture, water level, battery) now share
  one config. **Existing `OVERFLOW_THRESHOLD`/`MOISTURE_MIN`/`MOISTURE_MAX` values in
  `domain.rs` were calibrated against the buggy readings and need to be re-checked on hardware.**
- **Unitless sensors were deduplicated by HA's recorder**: `force_update: true` was only set in
  the discovery config when a sensor had a unit, so the unitless sensors (overflow, qualitative
  soil moisture, boot count) had their `last_updated` frozen by Home Assistant whenever the
  value was unchanged. A constantly-`NO` overflow reading then looked identical to a dead device.
  `force_update` is now set for every sensor, so a frozen timestamp reliably means the device
  stopped reporting.

### Added
- **`BootCount` MQTT sensor**: boot count (previously shown only on the display on attended
  wakes) is now published as a regular sensor every cycle, timer wakes included. Lets a gap in
  Home Assistant's history be diagnosed remotely — a boot count that jumps by more than one per
  missing hour means the device woke and failed before publishing (WiFi/MQTT); a contiguous
  count across a long gap means the timer wake itself didn't fire.
- **DNS query panic**: an empty DNS response for the MQTT broker hostname would panic on `a[0]`
  indexing instead of returning an error; now returns `Error::DnsNoRecords`.
- **Backlight left on during unattended wakes**: `Display::new` claimed to skip panel/backlight
  work on timer wakes, but `write_multiline` unconditionally powered the backlight regardless —
  every hourly timer wake (no button press) was driving the backlight and panel for the whole
  MQTT command window. The display is now only constructed at all on an attended wake; timer
  wakes skip it entirely. Same fix applied to the low-battery display path.
- **Display blank on cold boot / USB reset**: the above fix initially gated display construction
  on `wakeup_cause() == Ext0` (button press), but a fresh flash or USB reset is neither an Ext0
  nor a Timer wake, so `button_wake` was false and the display never showed anything — exactly
  when someone is plugged in and watching. The gate is now "not a Timer wake" instead of
  "is a button wake", so cold boots show sensor data again while hourly timer wakes still skip
  the display.
- **WiFi connection task could miss a stop signal**: the graceful-disconnect signal was only
  checked while connected; a stop request arriving during backoff sleep or mid-connect kept the
  radio powered until deep sleep force-cut it. The signal is now checked at the top of the loop
  and raced against both backoff sleeps.

### Changed
- Replaced polling loops for WiFi link-up/IP-address with `Stack::wait_link_up`/`wait_config_up`.
- Collapsed the repetitive per-sensor averaging blocks in `sensors/builder.rs` into a small
  `push` helper; the DHT11 reading is now used directly instead of being replicated into sample
  vectors to fit the same averaging code path.
- Consolidated MQTT publish boilerplate into a `publish_str` helper and cached the pump
  command topic string on `MqttSession` instead of rebuilding it per call.
- Removed unused `DisplayTrait` (single implementor), unused `Serialize`/`Deserialize` derives
  on `MoistureLevel` (dropped the `serde` dependency), and a no-op `.or(None)`.
- **GPIO re-pin: pump relay GPIO2→GPIO13, moisture ADC GPIO11→GPIO2, water level ADC GPIO12→GPIO3**: all three sensor ADC pins moved from ADC2 to ADC1, eliminating the second ADC peripheral. `SensorPeripherals` and `SensorHardware` no longer carry `adc2`; `builder.rs` reads both sensors through `adc1`. Pump relay moved to GPIO13 to free GPIO2 for the moisture ADC.

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
- Mermaid wiring diagram added to `README.md`.
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
