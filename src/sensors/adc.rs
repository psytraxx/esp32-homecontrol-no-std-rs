use embassy_time::{Duration, Timer};
use esp_hal::{
    analog::adc::{Adc, AdcCalScheme, AdcChannel, AdcPin, RegisterAccess},
    gpio::Output,
    Blocking,
};
use log::error;

use crate::config::SENSOR_WARMUP_DELAY_MS;

/// Power pin on → warmup → ADC sample → power pin off.
///
/// Unified reader for any powered ADC sensor (water level).
/// Each call site is monomorphised over its concrete pin/ADC types.
pub(super) async fn read_powered_adc_sensor<'a, PIN, ADCI, ADCC>(
    adc: &mut Adc<'a, ADCI, Blocking>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    power_pin: &mut Output<'a>,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess + 'a,
    ADCC: AdcCalScheme<ADCI>,
{
    power_pin.set_high();
    let result = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MS).await;
    power_pin.set_low();
    result
}

/// Sample an ADC pin after a configurable warmup delay.
pub(super) async fn sample_adc_with_warmup<'a, PIN, ADCI, ADCC>(
    adc: &mut Adc<'a, ADCI, Blocking>,
    pin: &mut AdcPin<PIN, ADCI, ADCC>,
    warmup_ms: u64,
) -> Option<u16>
where
    PIN: AdcChannel,
    ADCI: RegisterAccess + 'a,
    ADCC: AdcCalScheme<ADCI>,
{
    Timer::after(Duration::from_millis(warmup_ms)).await;
    match nb::block!(adc.read_oneshot(pin)) {
        Ok(value) => Some(value),
        Err(e) => {
            error!("Error reading sensor: {:?}", &e);
            None
        }
    }
}

/// Calculate the trimmed mean of a sample slice: remove the min and max, then average the rest.
///
/// Returns `None` if fewer than 3 samples are present.
pub(super) fn calculate_average<T>(samples: &mut [T]) -> Option<T>
where
    T: Copy + Ord + Into<i32>,
    i32: TryInto<T>,
{
    if samples.len() <= 2 {
        return None;
    }

    // Sort and remove outliers (first = lowest, last = highest)
    samples.sort_unstable();
    let samples = &samples[1..samples.len() - 1];

    let sum: i32 = samples.iter().map(|&x| x.into()).sum();
    sum.checked_div(samples.len() as i32)
        .and_then(|avg| avg.try_into().ok())
        .or(None)
}
