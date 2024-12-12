use chrono::{DateTime, Utc};
use defmt::Format;
use embassy_net::Stack;
use embassy_time::{Duration, Instant};
use esp_hal::macros::ram;

use crate::ntp;

/// Stored boot time between deep sleep cycles
///
/// This is a statically allocated variable and it is placed in the RTC Fast
/// memory, which survives deep sleep.
#[ram(rtc_fast)]
static mut BOOT_TIME: u64 = 0;

/// A clock
#[derive(Clone, Debug)]
pub struct Clock {
    unix_time: u64,
}

impl Clock {
    /// Create a new clock
    pub fn new(unix_time: u64) -> Self {
        Self { unix_time }
    }

    /// Return the current time
    pub fn now(&self) -> Option<DateTime<Utc>> {
        let epoch = self.now_as_epoch();
        DateTime::from_timestamp(epoch as i64, 0)
    }

    /// Create a new clock by synchronizing with a server
    pub async fn from_server(stack: Stack<'static>) -> Result<Self, Error> {
        let seconds = ntp::get_unix_time(stack).await?;

        Ok(Self::new(seconds as u64))
    }

    /// Initialize clock from RTC Fast memory
    pub fn from_rtc_memory() -> Option<Self> {
        // SAFETY:
        // There is only one thread
        let unix_time = unsafe { BOOT_TIME };

        if unix_time == 0 {
            None
        } else {
            Some(Self::new(unix_time))
        }
    }

    /// Store clock into RTC Fast memory
    pub fn save_to_rtc_memory(&self, expected_sleep_duration: Duration) {
        let now = self.now_as_epoch();
        let then = now + expected_sleep_duration.as_secs();
        // SAFETY:
        // There is only one thread
        unsafe {
            BOOT_TIME = then;
        }
    }

    /// Compute the next wakeup rounded down to a period
    ///
    /// * At 09:46:12 with period 1 minute, next rounded wakeup is 09:47:00.
    /// * At 09:46:12 with period 5 minutes, next rounded wakeup is 09:50:00.
    /// * At 09:46:12 with period 1 hour, next rounded wakeup is 10:00:00.
    pub fn duration_to_next_rounded_wakeup(&self, period: Duration) -> Duration {
        let epoch = Duration::from_secs(self.now_as_epoch());
        duration_to_next_rounded_wakeup(epoch, period)
    }

    /// Return current time as a Unix epoch
    pub fn now_as_epoch(&self) -> u64 {
        let from_boot = Instant::now().as_secs();
        self.unix_time + from_boot
    }
}

/// Compute the next wakeup rounded down to a period
///
/// * At 09:46:12 with period 1 minute, next rounded wakeup is 09:47:00.
/// * At 09:46:12 with period 5 minutes, next rounded wakeup is 09:50:00.
/// * At 09:46:12 with period 1 hour, next rounded wakeup is 10:00:00.
fn next_rounded_wakeup(now: Duration, period: Duration) -> Duration {
    let then = now + period;
    Duration::from_secs((then.as_secs() / period.as_secs()) * period.as_secs())
}

/// Compute the duration to next wakeup rounded down to a period
fn duration_to_next_rounded_wakeup(now: Duration, period: Duration) -> Duration {
    let then = next_rounded_wakeup(now, period);
    then - now
}

/// A clock error
#[derive(Debug, Format)]
pub enum Error {
    NtpError(ntp::Error),
}

impl From<ntp::Error> for Error {
    fn from(error: ntp::Error) -> Self {
        Self::NtpError(error)
    }
}
