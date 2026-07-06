//! Safe interior-mutability wrapper for statics placed in RTC fast memory
//! (`#[ram(unstable(rtc_fast))]`), which persists across deep sleep but
//! requires `static mut` for mutation. `RtcCell` protects all access with a
//! critical section instead: interrupts are disabled, Embassy is single-core
//! cooperative (no concurrent task execution), and deep sleep halts execution
//! entirely, so cross-boot access is inherently sequential.

use core::cell::UnsafeCell;

pub struct RtcCell<T> {
    value: UnsafeCell<T>,
}

impl<T> RtcCell<T> {
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            value: UnsafeCell::new(value),
        }
    }

    #[inline]
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        critical_section::with(|_cs| {
            // SAFETY: critical section excludes concurrent access (see module docs).
            unsafe { *self.value.get() }
        })
    }

    #[inline]
    pub fn set(&self, value: T) {
        critical_section::with(|_cs| {
            // SAFETY: critical section excludes concurrent access (see module docs).
            unsafe {
                *self.value.get() = value;
            }
        })
    }
}

// SAFETY: all access goes through a critical section (see module docs), so
// sharing across tasks cannot race even with interior mutability.
unsafe impl<T> Sync for RtcCell<T> where T: Send {}
