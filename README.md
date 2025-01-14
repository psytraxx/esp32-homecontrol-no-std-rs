# ESP32 Plant Watering System

A microcontroller-based system for automated plant watering with ESP32. Monitor soil moisture and control watering pump.

Built for [LilyGO T-Display-S3](https://github.com/Xinyuan-LilyGO/T-Display-S3) ESP32-S3 development board

![T-Display-S3 Board](https://raw.githubusercontent.com/Xinyuan-LilyGO/T-Display-S3/refs/heads/main/image/T-DISPLAY-S3.jpg)

**Board Features:**
- ESP32-S3 dual-core MCU
- 1.9" LCD Display (170x320)
- USB-C connector
- Built-in battery management

## Wiring

- DHT11 (Temperature/Humidity) → GPIO1  
- Relay for pump control → GPIO2 
- Soil moisture sensor → GPIO11  
- Water level sensor → GPIO12  


## Core Features

- **Basic Control**
  - ESP32-based moisture monitoring
  - Simple pump on/off control
  - Manual override button
  - Status LED indicators

- **Sensors**
  - Capacitive soil moisture sensor support
  - Temperature and humidity monitoring
  - Water overflow detection

- **Display Interface**
  - OLED/LCD display support
  - Sensor readings visualization
  - System status indicators

- **Modern Architecture**
  - Asynchronous programming using Embassy framework
  - Event-driven design for efficient resource usage
  - Type-safe Rust implementation

---

## Dependencies

The project uses several Rust crates to provide functionality:

## Async/Embedded Frameworks
These crates provide support for asynchronous programming models and embedded futures.

- [`embassy`](https://crates.io/crates/embassy)  
- [`embassy-executor`](https://crates.io/crates/embassy-executor)  
- [`embassy-futures`](https://crates.io/crates/embassy-futures)  
- [`embassy-net`](https://crates.io/crates/embassy-net)  
- [`embassy-sync`](https://crates.io/crates/embassy-sync)  
- [`embassy-time`](https://crates.io/crates/embassy-time)  

---

## Hardware Abstraction & Embedded I/O
These crates are used to interact with embedded hardware and interfaces.

- [`embedded-hal`](https://crates.io/crates/embedded-hal)  
- [`embedded-text`](https://crates.io/crates/embedded-text)  
- [`embedded-graphics`](https://crates.io/crates/embedded-graphics)  

---

## Networking
Crates for network communication and protocols.

- [`rust-mqtt`](https://crates.io/crates/rust-mqtt)  
- [`esp-wifi`](https://crates.io/crates/esp-wifi)  

---

## ESP32-Specific Crates
Crates related to ESP32 platforms, Wi-Fi support, and memory management.

- [`esp-alloc`](https://crates.io/crates/esp-alloc)  
- [`esp-backtrace`](https://crates.io/crates/esp-backtrace)  
- [`esp-hal`](https://crates.io/crates/esp-hal)  
- [`esp-hal-embassy`](https://crates.io/crates/esp-hal-embassy)  

---

## Display
Crates used for display interfaces.

- [`mipidsi`](https://crates.io/crates/mipidsi)  

---

## Sensor Support
Crates for interfacing with sensors like DHT11.

- [`dht11`](https://crates.io/crates/dht11)  

---

## Serialization
Crates for data serialization and deserialization.

- [`serde`](https://crates.io/crates/serde)  
- [`serde_json`](https://crates.io/crates/serde_json)  



---

## Miscellaneous
Other supporting crates for various use cases.

- [`heapless`](https://crates.io/crates/heapless)  
- [`static_cell`](https://crates.io/crates/static_cell)  
- [`rand_core`](https://crates.io/crates/rand_core)  
- and more...

---

## Setup

### 1. Clone the repository

```sh
git clone https://github.com/yourusername/esp32-homecontrol-no-std-rs.git
cd esp32-homecontrol-no-std-rs
```

---

### 2. Install Rust and the necessary tools

https://docs.esp-rs.org/book/introduction.html

Install espup
https://github.com/esp-rs/espup
and probe-rs
https://github.com/probe-rs/probe-rs

```sh
espup install
. $HOME/export-esp.sh

```

---

### 3. Build and run the project

```sh
cp .env.dist .env
./run.sh
```

---

## Usage

To flash the firmware to your ESP32 device, use the following command:

```sh
cargo run --release
```

---

## Useful Links

### DHCP & Wi-Fi
- [DHCP Wi-Fi Example with Embassy](https://github.com/esp-rs/esp-hal/blob/main/examples/src/bin/wifi_embassy_dhcp.rs)  

---

### MQTT Communication
- [MQTT Example](https://github.com/etiennetremel/esp32-home-sensor/blob/fff5f7ca4055e38ed5c296d0544fa8e61d855388/src/main.rs)  

---

### Display Interfaces
- [MIDISPI Example with Display](https://github.com/embassy-rs/embassy/blob/227e073fca97bcbcec42d9705e0a8ef19fc433b5/examples/rp/src/bin/spi_gc9a01.rs#L6)  
- [Display Example via SPI](https://github.com/embassy-rs/embassy/blob/227e073fca97bcbcec42d9705e0a8ef19fc433b5/examples/rp/src/bin/spi_display.rs#L6)  

---

### Sensors
- [DHT11 Sensor Integration Example](https://github.com/rust-dd/embedded-dht-rs)  
- [Moisture Sensor Example](https://github.com/nand-nor/plant-minder/blob/4bc70142a9ec11e860b5422deb9d85ad192bab66/pmindp-esp32-thread/src/sensor/probe_circuit.rs#L63)  

---

### UI & Graphics
- [Icons and UI Example with Display & Publish](https://github.com/sambenko/esp32s3box-display-and-publish)  

---

### Projects and Games
- [Pacman Game Example](https://github.com/georgik/esp32-spooky-maze-game)  
- [Motion Sensors & Body Tracking Example](https://github.com/SlimeVR/SlimeVR-Rust/blob/9eff429f4f01c8b7c607f3c3988de82729c753b3/firmware/src/peripherals/esp32/esp32c3.rs#L38)  

---

### Networking
- [Netstack & MQTT Struct Example](https://github.com/mirkomartn/esp32c3-embassy-poc/blob/9ad954dcba19897a973e3453fd83196829eee485/src/netstack.rs)  
- [HTTP Request Example with Embassy](https://github.com/embassy-rs/embassy/blob/86578acaa4d4dbed06ed4fcecec25884f6883e82/examples/rp/src/bin/wifi_webrequest.rs#L136)  
- [NTP Socket Example](https://github.com/vpetrigo/sntpc/blob/2711f17d42b9a681ced02639780fe72cd8042b36/examples/smoltcp-request/src/main.rs)  

---

### Miscellaneous Examples
- [Joystick Analog Pin Input Example](https://github.com/WJKPK/rc-car/blob/f1ce37658c7b8b6cbc47c844243ea8b90d1e1483/pilot/src/main.rs)  
- [Battery Monitoring Example](https://github.com/longxiangam/work_timer/blob/788c0bee18ec47adce07e3ba71e884920e6473e1/src/battery.rs)  

---

### Tutorials
- [ESP32 Rust HAL Tutorials](https://blog.theembeddedrustacean.com/series/esp32c3-embedded-rust-hal)  

---

### Advanced Examples
- [Futures Example](https://github.com/kamo104/esp32-rust-mqtt-esp-now-gateway/blob/main/src/main.rs)  
- [Sleep, Display Layout, Logging, Dashboard, Multiple Tasks](https://github.com/claudiomattera/esp32c3-embassy/blob/master/esp32c3-embassy/src/sleep.rs)  

---

### Bitcoin Device, USB, & OTA Updates
- [Fancy Bitcoin Device with GUI Example](https://github.com/frostsnap/frostsnap/blob/0b2d589bcf8a0863e1067595aae8c9376cfb4867/device/src/graphics/animation.rs)  

---

This documentation is curated to help you get started with various functionalities, libraries, and examples for ESP32 projects using Rust.