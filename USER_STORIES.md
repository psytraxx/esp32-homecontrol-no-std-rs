# User Stories

## Context

The device is a remote plant watering system. The user is typically **not physically present** — they check plant status via Home Assistant from anywhere and decide remotely whether to water.

The device spends most of its time in **deep sleep** (~59.5 min), waking for ~30 s each hour to read sensors and publish to MQTT. It cannot receive MQTT commands while asleep.

---

## Stories

### S1 — Remote monitoring
**As a user** who is away from home,
**I want to** open Home Assistant and see current soil moisture, temperature, humidity, battery voltage, and overflow status,
**so that** I can decide whether the plant needs watering.

### S2 — Remote pump activation
**As a user** who sees that soil is dry in HA,
**I want to** schedule a pump run that will execute on the device's next wake cycle,
**so that** I can water the plant without being physically present.

### S3 — Overflow interlock
**As a user** who has scheduled a pump run,
**I want** the pump to be blocked automatically if the drainage overflow sensor detects water at the pot base,
**so that** the pot doesn't flood even if I misjudged the situation.

### S4 — Pump feedback
**As a user**,
**I want to** see in HA whether the pump ran, was blocked by overflow, or is idle,
**so that** I know what actually happened during the last wake cycle.

> The HA switch reflects outcome: `OFF` means the device acted (ran or blocked) and reset the switch. If the switch is still `ON` after a wake cycle, the device didn't reach MQTT — check connectivity.

### S5 — HA auto-discovery on first boot
**As a user** setting up the device for the first time (or after a broker wipe),
**I want** the device to automatically register all sensors and controls with Home Assistant,
**so that** entities appear in HA without any manual configuration.

> Currently implemented via `DISCOVERY_MESSAGES_SENT` in RTC fast memory — discovery runs once on first boot, skipped on subsequent wake cycles. Reset by flashing or power-cycling with the RTC memory cleared.

### S6 — Battery awareness
**As a user**,
**I want to** see battery voltage in HA,
**so that** I know when to recharge before the device stops working.

### S7 — Pump scheduling from anywhere
**As a user** who is 100 km away and sees dry soil in HA,
**I want to** flip a switch in HA that will trigger the pump on the device's **next wake cycle**,
**so that** I can water the plant remotely without needing to be present when the device happens to be awake.

The switch state must be **retained** by the MQTT broker so it survives the device's deep sleep. On wake, the device reads the retained switch state, checks overflow, runs the pump if safe, then resets the switch to `OFF`.

---

## Key Constraints

- Device is in deep sleep ~98% of the time — it cannot act on MQTT commands in real time
- User is remote — no physical access to the wake button
- Pump commands must survive deep sleep (persist until next wake)
- Overflow check must happen at pump execution time, not at command time
- Overflow state must be read from sensors before the retained pump command is processed — subscribe to the pump topic only after the first sensor reading to avoid a race condition
