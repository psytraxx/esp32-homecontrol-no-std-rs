# Changelog

All notable changes to this project will be documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

---

## [Unreleased]

### Added
- `HARDWARE_V2.md` — sensor upgrade plan with confirmed BOM: AHT20+BMP280 combo, Adafruit STEMMA soil sensor, INA219 power monitor; all I2C, STEMMA QT connectors; Rust crate analysis, wiring diagram, firmware checklist
- Mermaid wiring diagrams for V1 (current) and V2 (planned) hardware added to `README.md`

### Changed
- MQTT discovery payload now includes `force_update: true` for numeric sensors — prevents Home Assistant recorder from deduplicating unchanged values, giving full hourly history resolution
- Updated `CLAUDE.md` — added changelog and code quality workflow requirements

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
