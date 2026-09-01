# Changelog

All notable changes to this project will be documented here.

Entries describe **what changed and why it matters**, not how it was implemented.
Changes are grouped by date, newest first.

---

## 2026-09-01

### Fixed
- Restored a clean build against the updated dependencies (patch-level bumps).

---

## 2026-08-14

### Documentation
- README dependency list synced with `Cargo.toml`.
- Documented the `bootcount` MQTT topic, the low-battery guard, and attended-wake-only display.
- Corrected the pump-control flow: sensors read in parallel with WiFi, fail-closed overflow
  interlock, MQTT session bounded by the awake deadline.

---

## 2026-07-06

### Added
- `bootcount` MQTT sensor, published every cycle including timer wakes — lets a gap in Home
  Assistant history be diagnosed remotely.

### Fixed
- Boot count now charts as a numeric line in Home Assistant instead of a categorical colored bar.
- `BOOT_COUNT` and `DISCOVERY_MESSAGES_SENT` now survive deep sleep; both previously reset to
  their initial values on every wake, so the published boot count never left `0`.
- Pump interlock is now fail-closed: a missing overflow reading blocks the pump instead of
  allowing it.
- The MQTT session is now bounded by the awake deadline, so a stalled broker can no longer keep
  the radio on indefinitely.
- Unitless sensors (overflow, moisture level, boot count) are no longer deduplicated by Home
  Assistant's recorder, so a frozen timestamp reliably means the device stopped reporting.
- Water-level ADC pin now uses the same calibration curve as the other ADC pins.
- All three ADC1 pins now apply the intended 11 dB attenuation; moisture and water level were
  silently read without it. **`OVERFLOW_THRESHOLD`/`MOISTURE_MIN`/`MOISTURE_MAX` were calibrated
  against the buggy readings and need re-checking on hardware.**
- An empty DNS response for the broker hostname returns an error instead of panicking.

### Changed
- WiFi link-up and IP-address polling loops replaced with `Stack::wait_link_up`/`wait_config_up`.
- Removed unused `DisplayTrait`, unused serde derives on `MoistureLevel`, and dead code.

---

## 2026-06-23

### Fixed
- Wake-cycle stability after the esp-radio 0.18.0 migration — DHT11 read failures, WiFi
  instability, and a brownout/reset loop that drained weak batteries:
  - DHT11 is read before the radio powers on, and retries up to 3 times, so radio interrupts no
    longer corrupt its timing-sensitive read.
  - WiFi reconnect uses exponential backoff instead of a flat 1 s retry, so a flaky AP no longer
    drives a reconnect storm.
  - Below 3300 mV the cycle skips WiFi and the pump, breaking the brownout/reset loop.
  - The SoC reset reason is logged at boot and shown on the button-wake display, so brownouts and
    watchdog resets are visible without a serial cable.
- WiFi reconnects automatically within 1 s when the AP drops the link, instead of failing the
  cycle with a timeout.
- The WiFi task no longer misses a shutdown signal that arrives during backoff or mid-connect.
- The backlight and panel are no longer driven during unattended timer wakes.
- The display works again on cold boot and USB reset, which are neither button nor timer wakes.

### Changed
- GPIO re-pin: pump relay GPIO2→GPIO13, moisture ADC GPIO11→GPIO2, water level ADC GPIO12→GPIO3.
  All sensor ADC pins now sit on ADC1, eliminating the second ADC peripheral.

---

## 2026-06-09

### Changed
- **The wake cycle is now one linear async flow.** The device is a batch job (wake → work →
  sleep ~1 h), so the task/channel/signal structure was replaced with sequential steps. Only the
  WiFi tasks remain. A failed cycle now sleeps and retries next hour instead of boot-looping.
- Pump is controlled exclusively from Home Assistant via a **switch** entity; local
  soil-moisture auto-triggering was removed. The retained switch state survives deep sleep, so a
  command is never lost, and the device resets it to `OFF` after acting.
- A pump run lasts exactly 10 s and is awaited inline, so deep sleep can no longer cut it short.
- Pump start is blocked when the overflow sensor reports water at the pot base.
- Overflow state is established before MQTT connects, making the retained-`ON` race impossible.
- CPU clock reduced 240 MHz → 80 MHz for the I/O-bound workload, cutting CPU dynamic power ~3×.
- DHT11 is read once per wake instead of once per sample, saving ~8 s of active time per wake.
- WiFi connection is bounded by a 30 s timeout instead of waiting forever.

### Added
- Mermaid wiring diagram in `README.md`.
- `src/sensors/` module split: hardware init, ADC sampling, and data assembly.

### Removed
- `update_task`, `relay_task`, `sensor_task` and their channels/signals; only the WiFi signal remains.
- Valve and button MQTT entities, replaced by the switch. **Delete the retained
  `homeassistant/valve/...` and `homeassistant/button/...` discovery topics from the broker.**
- `WaterLevel` enum, replaced by a binary overflow sensor. The MQTT topic changed from
  `waterlevel` to `overflow` with values `YES`/`NO`. **Delete the retained
  `homeassistant/sensor/{DEVICE_ID}_waterlevel/config` topic from the broker.**

---

## 2026-06-03

### Changed
- Migrated to esp-radio 0.18.0.

---

## 2026-05-23

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
