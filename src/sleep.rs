use defmt::info;
use embassy_time::Duration;
use esp_hal::peripherals::LPWR;
use esp_hal::rtc_cntl::sleep::{RtcSleepConfig, TimerWakeupSource};
use esp_hal::rtc_cntl::Rtc;

/// Enter deep sleep mode for the specified duration.
pub fn enter_deep(rtc_cntl: LPWR, interval: Duration) -> ! {
    let wakeup_source_timer = TimerWakeupSource::new(interval.into());

    let mut rtc = Rtc::new(rtc_cntl);

    let mut config = RtcSleepConfig::deep();
    config.set_rtc_fastmem_pd_en(false);

    info!("Entering deep sleep for {}", interval);
    rtc.sleep(&config, &[&wakeup_source_timer]);
    unreachable!();
}
