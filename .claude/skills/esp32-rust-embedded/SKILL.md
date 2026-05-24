---
name: esp32-rust-embedded
description: Expert embedded Rust development for ESP32 microcontrollers using no-std, Embassy async framework, and the ESP-RS ecosystem (esp-hal, esp-rtos, esp-radio). Use when building, debugging, flashing, or adding features to ESP32 projects. Covers sensor integration (ADC, GPIO, I2C, SPI), power management (deep sleep, RTC memory), WiFi networking, MQTT clients, display drivers, async task patterns, memory allocation, error handling, and dependency management. Ideal for LilyGO boards, Espressif chips (ESP32, ESP32-S3, ESP32-C3), and any no-std Xtensa/RISC-V embedded development.
compatibility: Requires ESP Rust toolchain (espup), cargo-espflash, and ESP32/ESP32-S3/ESP32-C3 hardware. Designed for Claude Code and similar AI coding assistants.
---

# ESP32 Embedded Rust Specialist

Expert guidance for no-std Rust development on ESP32 microcontrollers using the ESP-RS ecosystem and Embassy async framework.

## ESP-RS Ecosystem Stack

### Core Dependencies
```toml
esp-hal = { version = "~1.1.0", features = ["esp32s3", "log-04", "unstable"] }
esp-rtos = { version = "0.3.0", features = ["embassy", "esp-alloc", "esp-radio", "esp32s3", "log-04"] }
esp-radio = { version = "0.18.0", features = ["esp-alloc", "esp32s3", "log-04", "unstable", "wifi"] }
esp-bootloader-esp-idf = { version = "0.5.0", features = ["esp32s3", "log-04"] }
esp-alloc = "0.10.0"
esp-println = { version = "0.17.0", features = ["esp32s3", "log-04"] }

# smoltcp is now a DIRECT dependency (no longer a feature of esp-radio)
smoltcp = { version = "0.13.0", default-features = false, features = [
  "log", "medium-ethernet", "multicast",
  "proto-dhcpv4", "proto-dns", "proto-ipv4",
  "socket-dns", "socket-icmp", "socket-raw", "socket-tcp", "socket-udp",
] }
```

### Embassy Framework
```toml
embassy-executor = { version = "0.10.0", features = ["log"] }
embassy-time = { version = "0.5.0", features = ["log"] }
embassy-net = { version = "0.9.1", features = ["dhcpv4", "log", "medium-ethernet", "tcp", "udp"] }
embassy-sync = { version = "0.7.2" }
```

### Dependency Hierarchy
```
esp-radio (WiFi) -> esp-rtos (scheduler) -> esp-hal (HAL) -> esp-phy (PHY)
embassy-executor -> embassy-time -> embassy-sync -> embassy-net
```

### Rust Edition & MSRV
- **Edition**: 2024 (`edition = "2024"` in Cargo.toml)
- **MSRV**: 1.88 (`rust-version = "1.88"`)
- **Binary location**: `src/bin/main.rs` (not `src/main.rs`) — `[[bin]]` entry required in Cargo.toml

## Build & Flash

### Environment Setup
```bash
# Install ESP toolchain (one-time)
espup install
source $HOME/export-esp.sh

# Configure credentials (.env file)
cp .env.dist .env
# Edit: WIFI_SSID, WIFI_PSK, MQTT_HOSTNAME, MQTT_USERNAME, MQTT_PASSWORD
```

### Build Commands
```bash
# Quick build and flash
./run.sh

# Manual release build (recommended)
cargo run --release

# Debug build (slower on device)
cargo run
```

### Cargo Profile Optimization
```toml
[profile.dev]
opt-level = "s"  # Rust debug too slow for ESP32

[profile.release]
lto = 'fat'
opt-level = 's'
codegen-units = 1
```

### Common Build Errors

**Linker error: undefined symbol `_stack_start`**
- Check `build.rs` has linkall.x configuration
- Verify esp-hal version compatibility

**undefined symbol: `esp_rtos_*`** ("esp-radio has no scheduler enabled")
- Ensure esp-rtos is started with BOTH a timer AND a software interrupt (changed in 0.3.0):
```rust
let timg0 = TimerGroup::new(peripherals.TIMG0);
let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(
    peripherals.SW_INTERRUPT
);
esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);
```

**Environment variable errors**
- Variables are compile-time via `env!()` macro
- Changes require full rebuild

## No-Std Patterns

### Application Entry
```rust
#![no_std]
#![no_main]
// Recommended lints (now standard in esp-generate templates)
#![deny(clippy::mem_forget)]  // esp-hal types must not be mem::forgotten
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use esp_hal::{clock::CpuClock, timer::timg::TimerGroup};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    // Initialize logger (reads RUST_LOG env var at compile time via esp-config)
    esp_println::logger::init_logger_from_env();

    // Initialize HAL with max CPU clock
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Setup heap allocator using reclaimed memory (replaces dram2_uninit approach)
    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    // Start RTOS scheduler — now requires BOTH a timer AND a software interrupt
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt = esp_hal::interrupt::software::SoftwareInterruptControl::new(
        peripherals.SW_INTERRUPT
    );
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    // Spawn tasks...
    let _ = spawner;
    loop {}
}
```

### Memory Management
- Use `esp-alloc` for dynamic allocation
- Prefer `heapless` collections with compile-time capacity
- Use `static_cell::StaticCell` for 'static lifetime requirements

### String Handling
```rust
use alloc::string::String;      // Dynamic strings (heap)
use heapless::String;           // Bounded strings (stack)

let s: heapless::String<64> = heapless::String::new();
```
Avoid cloning when possible.

### StaticCell Pattern
```rust
static CHANNEL: StaticCell<Channel<NoopRawMutex, Data, 3>> = StaticCell::new();

// In async function
let channel: &'static mut _ = CHANNEL.init(Channel::new());
let (sender, receiver) = (channel.sender(), channel.receiver());
```

## Hardware Patterns

### GPIO Configuration
```rust
use esp_hal::gpio::{Level, Output, OutputConfig, Pull, DriveMode};

// Standard output
let pin = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());

// Open-drain for sensors like DHT11
let pin = Output::new(
    peripherals.GPIO1,
    Level::High,
    OutputConfig::default()
        .with_drive_mode(DriveMode::OpenDrain)
        .with_pull(Pull::None),
).into_flex();
```

### ADC Reading with Calibration
```rust
use esp_hal::analog::adc::{Adc, AdcConfig, AdcCalCurve, Attenuation};

let mut adc_config = AdcConfig::new();
let pin = adc_config.enable_pin_with_cal::<_, AdcCalCurve<ADC2>>(
    peripherals.GPIO11,
    Attenuation::_11dB  // 0-3.3V range
);
let adc = Adc::new(peripherals.ADC2, adc_config);

// Read with nb::block!
let value = nb::block!(adc.read_oneshot(&mut pin))?;
```

### Peripheral Bundles Pattern
```rust
pub struct SensorPeripherals {
    pub dht11_pin: GPIO1<'static>,
    pub moisture_pin: GPIO11<'static>,
    pub power_pin: GPIO16<'static>,
    pub adc2: ADC2<'static>,
}
```

## Async Task Architecture

### Task Definition
```rust
#[embassy_executor::task]
pub async fn my_task(sender: Sender<'static, NoopRawMutex, Data, 3>) {
    loop {
        // Do work
        sender.send(data).await;
        Timer::after(Duration::from_secs(5)).await;
    }
}
```

### Task Spawning
```rust
spawner.spawn(sensor_task(sender, peripherals)).ok();
spawner.spawn(update_task(stack, display, receiver)).ok();
```

### Inter-Task Communication

**Channel (multiple values)**
```rust
use embassy_sync::{blocking_mutex::raw::NoopRawMutex, channel::Channel};

static CHANNEL: StaticCell<Channel<NoopRawMutex, Data, 3>> = StaticCell::new();
// sender.send(data).await / receiver.receive().await
```

**Signal (single notification)**
```rust
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};

static SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();
// SIGNAL.signal(()) / SIGNAL.wait().await
```

### Reconnection Loop Pattern
```rust
'reconnect: loop {
    let mut client = initialize_client().await?;
    loop {
        match client.process().await {
            Ok(_) => { /* handle messages */ }
            Err(e) => {
                println!("Error: {:?}", e);
                continue 'reconnect;  // Reconnect on error
            }
        }
    }
}
```

## Power Management

### Deep Sleep Configuration
```rust
use esp_hal::rtc_cntl::{Rtc, sleep::{RtcSleepConfig, TimerWakeupSource, RtcioWakeupSource, WakeupLevel}};

pub fn enter_deep(wakeup_pin: &mut dyn RtcPin, rtc_cntl: LPWR, duration: Duration) -> ! {
    // GPIO wake source
    let wakeup_pins: &mut [(&mut dyn RtcPin, WakeupLevel)] = &mut [(wakeup_pin, WakeupLevel::Low)];
    let ext0 = RtcioWakeupSource::new(wakeup_pins);

    // Timer wake source
    let timer = TimerWakeupSource::new(duration.into());

    let mut rtc = Rtc::new(rtc_cntl);
    let mut config = RtcSleepConfig::deep();
    config.set_rtc_fastmem_pd_en(false);  // Keep RTC fast memory powered

    rtc.sleep(&config, &[&ext0, &timer]);
    unreachable!();
}
```

### RTC Fast Memory Persistence
```rust
use esp_hal::ram;

#[ram(unstable(rtc_fast))]
pub static BOOT_COUNT: RtcCell<u32> = RtcCell::new(0);

// Survives deep sleep - read/write with .get()/.set()
let count = BOOT_COUNT.get();
BOOT_COUNT.set(count + 1);
```

### Power Optimization
- Toggle sensor power pins only during reads
- Use power save mode on displays
- Gracefully disconnect WiFi before sleep
- Keep awake duration minimal

## WiFi Networking

### Connection Setup
```rust
// esp_radio::init() is GONE — wifi::new() is called directly (as of esp-radio 0.18+)
use esp_radio::wifi::{self, ClientConfig, ModeConfig, WifiController};

let (mut controller, interfaces) =
    esp_radio::wifi::new(peripherals.WIFI, Default::default())
        .expect("Failed to initialize Wi-Fi controller");

let client_config = ModeConfig::Client(
    ClientConfig::default()
        .with_ssid(env!("WIFI_SSID").try_into().unwrap())
        .with_password(env!("WIFI_PSK").try_into().unwrap()),
);

controller.set_config(&client_config)?;
controller.start_async().await?;
controller.connect_async().await?;
```

### Embassy-Net Stack
```rust
use embassy_net::{Config, Stack, StackResources};

let config = Config::dhcpv4(DhcpConfig::default());
let (stack, runner) = embassy_net::new(wifi_interface, config, stack_resources, seed);

// Wait for link and IP
loop {
    if stack.is_link_up() { break; }
    Timer::after(Duration::from_millis(500)).await;
}

loop {
    if let Some(config) = stack.config_v4() {
        println!("IP: {}", config.address);
        break;
    }
    Timer::after(Duration::from_millis(500)).await;
}
```

### Graceful WiFi Shutdown
```rust
pub static STOP_WIFI_SIGNAL: Signal<CriticalSectionRawMutex, ()> = Signal::new();

// In connection task
STOP_WIFI_SIGNAL.wait().await;
controller.stop_async().await?;

// Before deep sleep
STOP_WIFI_SIGNAL.signal(());
```

## Sensor Patterns

### ADC Sampling with Warmup
```rust
async fn sample_adc_with_warmup<PIN, ADC>(
    adc: &mut Adc<ADC, Blocking>,
    pin: &mut AdcPin<PIN, ADC>,
    warmup_ms: u64,
) -> Option<u16> {
    Timer::after(Duration::from_millis(warmup_ms)).await;
    nb::block!(adc.read_oneshot(pin)).ok()
}
```

### Power-Controlled Sensor Read
```rust
async fn read_sensor(adc: &mut Adc, pin: &mut AdcPin, power: &mut Output) -> Option<u16> {
    power.set_high();
    let result = sample_adc_with_warmup(adc, pin, 50).await;
    power.set_low();
    result
}
```

### Outlier-Resistant Averaging
```rust
fn calculate_average<T: Copy + Ord + Into<u32>>(samples: &mut [T]) -> Option<T> {
    if samples.len() <= 2 { return None; }

    samples.sort_unstable();
    let trimmed = &samples[1..samples.len() - 1];  // Remove min/max

    let sum: u32 = trimmed.iter().map(|&x| x.into()).sum();
    (sum / trimmed.len() as u32).try_into().ok()
}
```

## Display Integration

### ST7789 Parallel Interface
```rust
use mipidsi::{Builder, options::ColorInversion};

let di = display_interface_parallel_gpio::Generic8BitBus::new(/*pins*/);
let mut display = Builder::new(ST7789, di)
    .display_size(320, 170)
    .invert_colors(ColorInversion::Inverted)
    .init(&mut delay)?;
```

### Power Save Mode
```rust
display.set_display_on(false)?;  // Enter power save
// Before deep sleep
power_pin.set_low();
```

## Error Handling

### Module Error Pattern
```rust
#[derive(Debug)]
pub enum Error {
    Wifi(WifiError),
    Display(display::Error),
    Mqtt(MqttError),
}

impl From<WifiError> for Error {
    fn from(e: WifiError) -> Self { Self::Wifi(e) }
}
```

### Fallible Main Pattern
```rust
// Note: #[esp_rtos::main] (not use esp_rtos::main; #[main])
// Note: main now returns `-> !` (diverging)
#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    if let Err(error) = main_fallible(spawner).await {
        log::error!("Fatal: {:?}", error);
        esp_hal::system::software_reset();
    }
    unreachable!()
}

async fn main_fallible(spawner: Spawner) -> Result<(), Error> {
    // Application logic with ? operator
}
```

## Dependency Updates

### Safe Update Process
```bash
cargo outdated
cargo update -p esp-hal
cargo build --release
cargo clippy -- -D warnings
```

### Breaking Change Patterns
- GPIO API changes frequently (OutputConfig)
- Timer initialization changes
- Feature flag renames
- **esp-rtos 0.3.0**: `esp_rtos::start()` now requires 2 args (timer + software interrupt)
- **esp-radio 0.18.0**: `esp_radio::init()` removed; call `esp_radio::wifi::new()` directly
- **smoltcp**: now a direct dependency, not a feature of esp-radio
- **heap_allocator!**: use `#[esp_hal::ram(reclaimed)]` instead of `#[unsafe(link_section = ".dram2_uninit")]`
- Always check esp-hal release notes and migrate with esp-generate's generated templates as reference

### Version Alignment
Update Embassy crates together:
```bash
cargo update -p embassy-executor -p embassy-time -p embassy-sync -p embassy-net
```

### esp-generate (project scaffold tool)
```bash
# Install
cargo install esp-generate --locked

# Generate ESP32-S3 project with Embassy + WiFi + alloc
esp-generate --chip esp32s3 -o unstable-hal -o embassy -o alloc -o wifi -o log my-project

# Available options: unstable-hal, alloc, wifi, ble-trouble, embassy,
#   probe-rs, defmt, log, esp-backtrace, embedded-test, wokwi, ci,
#   vscode, neovim, helix, zed
```
Note: `wifi` option now **requires** both `unstable-hal` and `alloc`.  
Note: `embassy` option requires `unstable-hal`.

## Debugging

### Serial Logging
```rust
use esp_println::println;
// New: reads RUST_LOG at compile time via esp-config (.cargo/esp-config.toml)
esp_println::logger::init_logger_from_env();
// Or explicitly set a level:
// esp_println::logger::init_logger(log::LevelFilter::Info);
log::info!("Debug: value = {}", value);
println!("Debug: value = {}", value); // also works
```

### Common Runtime Issues
- **WiFi fails**: Check 2.4GHz network, signal strength
- **MQTT fails**: Verify DNS resolution, broker credentials
- **Sensors fail**: Check warmup delays, power pin toggling
- **Display blank**: Ensure GPIO15 is HIGH (power enable)
- **Sleep wake fails**: Verify RTC fast memory config

### Software Reset
```rust
use esp_hal::system::software_reset;
software_reset();  // Clean restart on unrecoverable error
```
