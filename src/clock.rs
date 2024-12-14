use chrono::{DateTime, Utc};
use embassy_time::{Duration, Instant};

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
