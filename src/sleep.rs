use embassy_time::Duration;
use esp_hal::gpio::{AnyPin, Event, Input, InputConfig, Pull, WakeupConfig};
use esp_hal::peripherals::LPWR;
use esp_hal::rtc_cntl::sleep::{LowPower, RtcSleepConfig};
use esp_hal::time::{Duration as HalDuration, Instant};

/// Enter deep sleep mode for the specified duration.
///
/// Callers should log and flush output (e.g. `Timer::after(100ms).await`)
/// before calling this function — once `sleep_deep` is invoked the USB CDC
/// serial has no opportunity to drain its transmit buffer.
pub fn enter_deep(wakeup_pin: AnyPin<'static>, rtc_cntl: LPWR<'static>, interval: Duration) -> ! {
    // The button pulls the pad low; hold it high the rest of the time so the
    // pad only wakes the chip on an actual press, not while floating.
    let mut wakeup_pin = Input::new(wakeup_pin, InputConfig::default().with_pull(Pull::Up));
    wakeup_pin
        .apply_wakeup_config(&WakeupConfig::default().with_low_power_path(true))
        .unwrap();
    wakeup_pin.listen(Event::LowLevel);

    let mut lpwr = LowPower::new(rtc_cntl);
    let deadline = Instant::now() + HalDuration::from_micros(interval.as_micros());
    lpwr.set_wakeup_deadline(deadline);

    lpwr.sleep_deep(RtcSleepConfig::deep());
}
