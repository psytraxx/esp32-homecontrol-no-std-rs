use defmt::info;
use embassy_time::Duration;
use esp_hal::gpio::{self, GpioPin, Input, Pull};
use esp_hal::peripheral::Peripheral;
use esp_hal::peripherals::LPWR;
use esp_hal::rtc_cntl::sleep::{RtcioWakeupSource, TimerWakeupSource, WakeupLevel};
use esp_hal::rtc_cntl::Rtc;

/// Enter deep sleep mode for the specified duration. The device will also wake up when the button connected to the pin is pressed.
pub fn enter_deep(button_pin: GpioPin<14>, rtc_cntl: LPWR, interval: Duration) -> ! {
    let wakeup_source_timer = TimerWakeupSource::new(interval.into());

    let button_pin = Input::new(button_pin, Pull::None);

    let wakeup_pins: &mut [(&mut dyn gpio::RtcPin, WakeupLevel)] =
        &mut [(&mut *button_pin.into_ref(), WakeupLevel::Low)];
    let ext0 = RtcioWakeupSource::new(wakeup_pins);

    let mut rtc = Rtc::new(rtc_cntl);

    info!("Entering deep sleep for {}", interval);
    rtc.sleep_deep(&[&ext0, &wakeup_source_timer]);
}
