//! Values that must survive deep sleep, stored in RTC fast memory.
//!
//! `#[ram(unstable(rtc_fast, persistent))]` places each static in RTC fast RAM
//! **and** skips re-running its initializer on wake — the memory is zeroed once
//! on the very first boot, then left untouched across deep sleep, watchdog
//! resets, `software_reset()`, etc. Plain `#[ram(unstable(rtc_fast))]` (without
//! `persistent`) re-applies the initializer on every reset, so the value would
//! reset to its declared default on each wake — which is *not* what we want for
//! a boot counter or a "discovery already sent" flag.
//!
//! Access is unsafe at the language level (`static mut`), but sound here:
//! Embassy is single-core cooperative (no preemption or concurrent tasks), and
//! deep sleep halts execution entirely, so every access is sequential. The
//! `persistent` initial value must be zero, so `BOOT_COUNT` starts at 0 and
//! `DISCOVERY_MESSAGES_SENT` starts at `false` on first boot, as intended.

use esp_hal::ram;

/// Wake cycles since first boot. Increments each wake; persists across deep sleep.
#[ram(unstable(rtc_fast, persistent))]
static mut BOOT_COUNT: u32 = 0;

/// Whether MQTT discovery messages have already been published (0 = no, 1 =
/// yes). Prevents re-sending them on every wake; persists across deep sleep.
/// Stored as `u32` rather than `bool` because `persistent` requires a type with
/// no invalid bit patterns (`bool` has only two valid bytes; `esp_hal` does not
/// implement `Persistable` for it).
#[ram(unstable(rtc_fast, persistent))]
static mut DISCOVERY_MESSAGES_SENT: u32 = 0;

/// Read the persisted boot count.
pub fn boot_count() -> u32 {
    // SAFETY: single-core cooperative scheduling and deep-sleep halts mean no
    // concurrent access to this static (see module docs).
    unsafe { BOOT_COUNT }
}

/// Store the boot count for the next wake.
pub fn set_boot_count(value: u32) {
    // SAFETY: see `boot_count`.
    unsafe { BOOT_COUNT = value }
}

/// Whether discovery messages have already been sent in a prior wake.
pub fn discovery_messages_sent() -> bool {
    // SAFETY: see `boot_count`.
    unsafe { DISCOVERY_MESSAGES_SENT != 0 }
}

/// Record that discovery messages have been sent.
pub fn set_discovery_messages_sent(value: bool) {
    // SAFETY: see `boot_count`.
    unsafe { DISCOVERY_MESSAGES_SENT = value as u32 }
}
