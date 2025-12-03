//! Safe wrapper for RTC fast memory variables
//!
//! This module provides [`RtcCell<T>`], a safe interior mutability wrapper
//! for static variables placed in ESP32's RTC fast memory using the
//! `#[ram(unstable(rtc_fast))]` attribute.
//!
//! # Safety Model
//!
//! RTC fast memory persists across deep sleep cycles but requires `static mut`
//! for mutability, forcing unsafe access. `RtcCell` eliminates unsafe blocks
//! at call sites by using critical sections to protect all access.
//!
//! This is safe because:
//! - Critical sections disable interrupts, preventing concurrent access
//! - Embassy uses cooperative multitasking on single core (no thread races)
//! - Deep sleep halts execution, so cross-boot access is inherently sequential
//!
//! # Example
//!
//! ```rust
//! use rtc_memory::RtcCell;
//!
//! #[ram(unstable(rtc_fast))]
//! static BOOT_COUNT: RtcCell<u32> = RtcCell::new(0);
//!
//! fn increment_boot_count() {
//!     let count = BOOT_COUNT.get();
//!     BOOT_COUNT.set(count + 1);
//! }
//! ```

use core::cell::UnsafeCell;

/// A safe wrapper for mutable static data in RTC fast memory.
///
/// Provides interior mutability with critical section protection,
/// ensuring safe access across Embassy async tasks. Data persists
/// across deep sleep when placed in RTC fast memory.
///
/// # Interior Mutability
///
/// `RtcCell` allows mutation through a shared reference using
/// [`UnsafeCell`] and critical sections. All operations are atomic
/// with respect to interrupts.
///
/// # Memory Layout
///
/// `RtcCell<T>` has the same memory layout as `T` due to
/// `#[repr(transparent)]` on `UnsafeCell`, ensuring RTC persistence
/// works correctly.
pub struct RtcCell<T> {
    value: UnsafeCell<T>,
}

impl<T> RtcCell<T> {
    /// Creates a new `RtcCell` containing the given value.
    ///
    /// This is a `const fn`, allowing static initialization with
    /// the `#[ram(unstable(rtc_fast))]` attribute.
    ///
    /// # Examples
    ///
    /// ```rust
    /// static COUNTER: RtcCell<u32> = RtcCell::new(0);
    /// ```
    #[inline]
    pub const fn new(value: T) -> Self {
        Self {
            value: UnsafeCell::new(value),
        }
    }

    /// Returns a copy of the contained value.
    ///
    /// This method is protected by a critical section, ensuring
    /// no interrupts occur during the read operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let cell = RtcCell::new(42u32);
    /// let value = cell.get();
    /// assert_eq!(value, 42);
    /// ```
    #[inline]
    pub fn get(&self) -> T
    where
        T: Copy,
    {
        critical_section::with(|_cs| {
            // SAFETY: We're in a critical section with interrupts disabled.
            // Embassy's cooperative multitasking on single core ensures no
            // concurrent task execution. The pointer is valid for the
            // lifetime of the static variable.
            unsafe { *self.value.get() }
        })
    }

    /// Sets the contained value.
    ///
    /// This method is protected by a critical section, ensuring
    /// no interrupts occur during the write operation.
    ///
    /// # Examples
    ///
    /// ```rust
    /// let cell = RtcCell::new(0u32);
    /// cell.set(42);
    /// assert_eq!(cell.get(), 42);
    /// ```
    #[inline]
    pub fn set(&self, value: T) {
        critical_section::with(|_cs| {
            // SAFETY: We're in a critical section with interrupts disabled.
            // Embassy's cooperative multitasking on single core ensures no
            // concurrent task execution. The pointer is valid for the
            // lifetime of the static variable.
            unsafe {
                *self.value.get() = value;
            }
        })
    }
}

// SAFETY: RtcCell can be safely shared between tasks because all access
// is protected by critical sections which disable interrupts. This prevents
// data races even though we have interior mutability.
//
// The `T: Send` bound ensures that the contained type itself can be safely
// transferred between tasks, which is sufficient for our use case since
// we never expose direct references to T across task boundaries.
unsafe impl<T> Sync for RtcCell<T> where T: Send {}
