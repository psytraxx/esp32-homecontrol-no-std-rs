use embassy_time::{Duration, Timer};
use esp_hal::{
    Blocking,
    analog::adc::{Adc, AdcCalLine, AdcCalScheme, AdcChannel, AdcPin, RegisterAccess},
    gpio::Output,
    peripherals::{ADC1, GPIO4},
};
use log::{error, info};

use crate::config::{SENSOR_WARMUP_DELAY_MS, USB_CHARGING_VOLTAGE_MV};

/// Power pin on → warmup → ADC sample → power pin off.
///
/// Unified reader for any powered ADC sensor (soil moisture, water level).
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

/// Read battery voltage, applying the 2× voltage divider and filtering out USB-charging readings.
pub(super) async fn read_battery_voltage<'a>(
    adc: &mut Adc<'a, ADC1<'a>, Blocking>,
    pin: &mut AdcPin<GPIO4<'a>, ADC1<'a>, AdcCalLine<ADC1<'a>>>,
) -> Option<u16> {
    let value = sample_adc_with_warmup(adc, pin, SENSOR_WARMUP_DELAY_MS).await? * 2;

    if value < USB_CHARGING_VOLTAGE_MV {
        Some(value)
    } else {
        info!(
            "Battery voltage too high - looks we are charging on USB: {}mV",
            value
        );
        None
    }
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
    T: Copy + Ord + Into<u32>,
    u32: TryInto<T>,
{
    if samples.len() <= 2 {
        return None;
    }

    // Sort and remove outliers (first = lowest, last = highest)
    samples.sort_unstable();
    let samples = &samples[1..samples.len() - 1];

    let sum: u32 = samples.iter().map(|&x| x.into()).sum();
    sum.checked_div(samples.len() as u32)
        .and_then(|avg| avg.try_into().ok())
        .or(None)
}
