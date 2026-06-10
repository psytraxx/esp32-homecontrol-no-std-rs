# Test Protocol

Plant watering system on LilyGO T-Display-S3. Device sleeps ~98% of the time; all logic that can be tested without hardware runs on the host. Hardware tests require the physical device and a running MQTT broker.

---

## 1. Host Unit Tests (`cargo test --target x86_64-unknown-linux-gnu`)

These test pure logic in `domain.rs` and `sensors/adc.rs`. No embassy runtime or hardware needed.

> **Setup:** add a `[dev-dependencies]` section with no ESP-specific crates and gate the tests with `#[cfg(test)]` inside each module, or place them under `tests/`.

### 1.1 `overflow_detected`

| Input (mV) | Expected | Rationale |
|------------|----------|-----------|
| 2800       | `false`  | threshold is exclusive (`>`) |
| 2801       | `true`   | just above threshold |
| 2217       | `false`  | typical dry reading |
| 3475       | `true`   | typical submerged reading |
| 0          | `false`  | sensor off / short |
| u16::MAX   | `true`   | saturated reading |

### 1.2 `MoistureLevel::from(u16)`

| Input (mV) | Expected     | Why |
|------------|--------------|-----|
| 800        | `Wet`        | at MOISTURE_MIN, ratio → 1.0 > 0.8 |
| 799        | `Wet`        | clamped to MIN, same as 800 |
| 2150       | `Dry`        | at MOISTURE_MAX, ratio → 0.0 < 0.15 |
| 2200       | `Dry`        | clamped to MAX |
| 1475       | `Moist`      | midpoint ≈ 0.5 |
| 900        | `Wet`        | ratio ≈ 0.93 |
| 2050       | `Dry`        | ratio ≈ 0.07 |

### 1.3 `calculate_average`

| Input slice          | Expected       | Why |
|----------------------|----------------|-----|
| `[]`                 | `None`         | too few |
| `[1]`                | `None`         | too few |
| `[1, 2]`             | `None`         | exactly 2, still too few |
| `[1, 2, 3]`          | `Some(2)`      | trim 1 and 3, average [2] |
| `[1, 1, 100, 1, 1]`  | `Some(1)`      | outlier 100 trimmed |
| `[10, 20, 30, 40, 50]` | `Some(30)`   | trim 10 and 50, average [20,30,40] |
| negative `i8` slice: `[-5, -3, -1]` | `Some(-3)` | signed trim |

### 1.4 `SoilMoistureRawLevel` clamping via `Display`

| Input | Displayed value |
|-------|-----------------|
| 800   | "800"           |
| 500   | "800" (clamped) |
| 2150  | "2150"          |
| 9999  | "2150" (clamped)|

### 1.5 `Sensor::value()` formatting

| Variant                     | Expected string |
|-----------------------------|-----------------|
| `OverflowDetected(true)`    | `"YES"`         |
| `OverflowDetected(false)`   | `"NO"`          |
| `AirTemperature(-3)`        | `"-3"`          |
| `AirHumidity(55)`           | `"55"`          |
| `BatteryVoltage(3700)`      | `"3700"`        |

---

## 2. Build Verification

```bash
cargo fmt --check       # must produce no diff
cargo clippy -- -D warnings   # must produce no warnings
cargo build --release   # must link successfully
```

Run after every code change before flashing.

---

## 3. Integration Tests (device on USB, `./run.sh`)

Monitor serial output with `espflash monitor` or `screen /dev/ttyUSB0 115200`.

### 3.1 Normal wake cycle

**Precondition:** device powered, WiFi reachable, MQTT broker running, pump switch `OFF`.

Expected serial log sequence:
1. `Boot count: N`
2. DHT11 temperature and humidity logged
3. ADC readings for moisture, water level, battery logged
4. WiFi IP address printed
5. MQTT connected, discovery published (first boot only)
6. Sensor state published to `esp32_breadboard/<topic>`
7. Pump command window opens (`polling for pump commands`)
8. After `AWAKE_DURATION_SECONDS` (30 s): WiFi disconnect, deep sleep entered

**Pass criteria:** all sensors appear in Home Assistant with non-zero values; device wakes again ~1 hour later.

### 3.2 USB charging suppression

**Precondition:** device powered via USB (not battery).

Expected: battery voltage log line shows "looks we are charging on USB" and no `BatteryVoltage` entity is published that cycle.

### 3.3 DHT11 failure path

**Precondition:** disconnect DHT11 pin before boot.

Expected: `DHT11 read failed` in serial log; `AirTemperature` and `AirHumidity` absent from published sensor data (not zero).

### 3.4 WiFi timeout

**Precondition:** configure device with wrong `WIFI_SSID` or power off router.

Expected: after `WIFI_CONNECT_TIMEOUT_SECONDS` (30 s), device logs timeout error and enters deep sleep. No panic, no boot loop. Wakes again ~1 hour later.

---

## 4. Pump & Overflow Tests (device on USB with relay wired)

### 4.1 Normal pump run

**Precondition:** overflow sensor dry (ADC < 2800 mV), pump switch set to `ON` in HA before wake.

Expected sequence:
1. Device wakes, reads sensors (overflow = `NO`)
2. Connects MQTT, subscribes to `esp32_breadboard/pump/set`
3. Retained `ON` delivered → device resets switch to `OFF`
4. Relay activates for 10 s (audible/measurable)
5. Switch confirmed `OFF` in HA after wake cycle

### 4.2 Overflow interlock blocks pump

**Precondition:** overflow sensor submerged (ADC > 2800 mV), pump switch `ON`.

Expected: device receives `ON`, reads overflow state as `YES`, logs "pump blocked due to overflow", resets switch to `OFF`. Relay does NOT activate.

**Critical check:** overflow state is determined from sensors read before MQTT subscribe — the retained `ON` cannot race the overflow read.

### 4.3 Pump switch already OFF on wake

**Precondition:** pump switch `OFF` in HA.

Expected: subscribe window elapses with no command, no pump run, no relay activation.

### 4.4 Pump command arrives mid-window

**Precondition:** pump switch `OFF` at wake. Set to `ON` during the 30 s awake window (via HA).

Expected: device receives `ON`, runs pump, resets switch. Confirms non-retained commands also work.

---

## 5. Deep Sleep & RTC Memory Tests

### 5.1 Boot count persists across sleep

After first flash: `Boot count: 1`.  
After each subsequent wake: count increments by 1.  
After hard power cycle (not deep sleep): count resets to 1.

### 5.2 Discovery sent only once

On first wake: discovery MQTT messages published.  
On subsequent wakes: no discovery messages (serial log confirms skip).  
After power cycle: discovery published again.

### 5.3 Button wake

**Precondition:** device in deep sleep.  
Press wake button (GPIO14).  
Expected: device wakes, serial log prefixes display line with IP and boot count.

---

## 6. Regression Checklist (before each release)

- [ ] `cargo fmt --check` passes
- [ ] `cargo clippy -- -D warnings` passes  
- [ ] `cargo build --release` succeeds
- [ ] Normal wake cycle (3.1) passes
- [ ] Pump run (4.1) confirmed with relay activation
- [ ] Overflow interlock (4.2) blocks pump
- [ ] Boot count increments (5.1)
- [ ] CHANGELOG.md updated under `[Unreleased]`
