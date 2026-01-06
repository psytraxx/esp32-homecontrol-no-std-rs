# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

ESP32-based plant watering system for the LilyGO T-Display-S3 board. Uses no-std Rust with Embassy async framework for sensor monitoring, MQTT communication with Home Assistant, and deep sleep power management.

## Build & Development Commands

### Setup Environment
```bash
# Install ESP Rust toolchain (one-time setup or )
espup install --toolchain-version 1.92.0
. $HOME/export-esp.sh

# Copy environment template and configure credentials
cp .env.dist .env
# Edit .env with WiFi and MQTT credentials
```

### Build & Flash
```bash
# Quick build and flash to device
./run.sh

# Manual build and flash (release mode)
cargo run --release

# Debug build (slower, more verbose)
cargo run
```

### Environment Variables Required
- `MQTT_HOSTNAME`: MQTT broker hostname
- `MQTT_USERNAME`: MQTT username
- `MQTT_PASSWORD`: MQTT password
- `MQTT_PORT`: MQTT port (default: 1883)
- `WIFI_SSID`: WiFi network name
- `WIFI_PSK`: WiFi password

Note: Environment variables are embedded at compile-time via `env!()` macro, not runtime.

## Architecture

### Async Task System
The application uses Embassy executor with multiple concurrent tasks:

1. **sensor_task** (sensors_task.rs): Reads sensors every 5s, sends data via channel
2. **update_task** (update_task.rs): Receives sensor data, manages MQTT publishing, updates display
3. **relay_task** (relay_task.rs): Controls water pump relay based on signals
4. **connection** (wifi.rs): Manages WiFi connection and disconnection
5. **net_task** (wifi.rs): Runs network stack

### Communication Patterns

**Inter-task communication:**
- `CHANNEL`: NoopRawMutex channel for sensor data (sensor_task → update_task)
- `ENABLE_PUMP`: CriticalSectionRawMutex signal for pump control (update_task → relay_task)
- `STOP_WIFI_SIGNAL`: Signal to gracefully disconnect WiFi before sleep

**MQTT Integration:**
- Publishes sensor readings to Home Assistant via MQTT
- Implements Home Assistant discovery protocol
- Subscribes to pump control commands (`{DEVICE_ID}/pump/set`)
- Auto-clears retained pump commands after activation

### Deep Sleep Cycle

The device operates in sleep/wake cycles to conserve battery:

1. Wake from deep sleep
2. Connect to WiFi
3. Publish discovery messages (first boot only)
4. Read sensors and publish for 30s (`AWAKE_DURATION_SECONDS`)
5. Disconnect WiFi
6. Enter deep sleep for ~1 hour (`DEEP_SLEEP_DURATION_SECONDS`)

**RTC Fast Memory:**
- `BOOT_COUNT`: Survives deep sleep, increments each boot
- `DISCOVERY_MESSAGES_SENT`: Prevents republishing discovery on every boot

### Hardware Abstraction

**Peripherals struct pattern:**
- DisplayPeripherals: Bundles all pins for ST7789 LCD (main.rs:101-116)
- SensorPeripherals: Bundles all sensor GPIO pins (main.rs:140-149)

**GPIO Pin Mapping:**
- GPIO1: DHT11 temperature/humidity sensor
- GPIO2: Water pump relay
- GPIO4: Battery voltage (ADC)
- GPIO11: Soil moisture analog (ADC)
- GPIO12: Water level analog (ADC)
- GPIO14: Wake button (deep sleep trigger)
- GPIO15: Power enable pin (MUST be HIGH for display)
- GPIO16: Moisture sensor power
- GPIO21: Water level sensor power
- GPIO38: Display backlight
- GPIO6-9, 39-48: Display interface pins

### Sensor Data Flow

1. `sensor_task` reads raw sensor values
2. Converts to domain types via `From<u16>` traits (domain.rs):
   - `MoistureLevel`: Wet/Moist/Dry (calibrated with thresholds)
   - `WaterLevel`: Full/Empty (drainage detection)
3. Packages into `SensorData` with `Vec<Sensor, 7>`
4. Sends through channel to `update_task`
5. `update_task` publishes to MQTT and updates display

### Display System

**ST7789 LCD via 8-bit parallel interface:**
- Uses mipidsi crate with embedded-graphics
- 320x170 resolution (landscape orientation)
- Supports multiline text rendering with embedded-text
- Power save mode after update cycle

### Error Handling

**Custom Error types per module:**
- Top-level Error enum aggregates Wifi, Display errors
- Errors trigger software reset in main
- MQTT task reconnects on any error (see 'reconnect loop pattern)

## Code Patterns

### StaticCell Pattern
Used extensively for 'static lifetime requirements:
```rust
static CHANNEL: StaticCell<Channel<...>> = StaticCell::new();
let channel: &'static mut _ = CHANNEL.init(Channel::new());
```

### Embassy Task Spawning
```rust
spawner.spawn(task_name(args)).ok();
```
Note: `.ok()` discards Result since spawner errors are rare

### Reconnection Loop Pattern (update_task.rs:70-134)
```rust
'reconnect: loop {
    let mut client = initialize_mqtt_client(...).await?;
    loop {
        // process messages
        if error {
            continue 'reconnect; // Break inner, retry outer
        }
    }
}
```

## Build System

**build.rs:**
- Configures linker with `linkall.x` script
- Provides helpful error messages for common linker issues
- Custom error handling script for undefined symbols

**Cargo Profile Optimizations:**
- Debug builds use `opt-level = "s"` (Rust debug too slow for ESP32)
- Release: LTO enabled, size-optimized, single codegen unit

## No-Std Environment

**Memory allocation:**
- Uses esp-alloc with 73744 bytes heap in DRAM2
- heapless collections (Vec, String) with compile-time capacity
- Static allocations preferred over dynamic

**String handling:**
- `alloc::string::String` for dynamic strings
- `heapless::String` for bounded strings
- Avoid cloning when possible

## Testing with QEMU (Optional)

```bash
cargo build --release
espflash save-image --chip esp32s3 --merge \
  target/xtensa-esp32s3-none-elf/release/esp32-homecontrol image.bin
qemu-system-xtensa --nographic -machine esp32s3 \
  -drive file=image.bin,if=mtd,format=raw -m 4M
```

## Important Notes

- All code is `#![no_std]` and `#![no_main]`
- Denies `clippy::mem_forget` for safety with ESP HAL types
- GPIO15 must be HIGH for anything to display when USB disconnected
- MQTT credentials and WiFi config are compile-time, not runtime
- Boot count persists across deep sleep via RTC Fast RAM
- Discovery messages sent only once per power cycle (not per wake)
