use embassy_time::{Duration, Timer};
use esp_hal::{
    Blocking,
    analog::adc::{Adc, AdcCalScheme, AdcChannel, AdcPin, RegisterAccess},
    gpio::Output,
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
