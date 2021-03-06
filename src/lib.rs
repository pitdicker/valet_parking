//! `valet_boy` provides a cross-platform abstraction over thread parking. The goal is to provide an
//! abstraction with little overhead, which is `no_std`-compatible and requires little overhead.
#![cfg_attr(not(any(test, target_vendor = "fortanix")), no_std)]
#![cfg_attr(
    all(target_arch = "wasm32", target_feature = "atomics"),
    feature(stdsimd)
)]
#![cfg_attr(target_vendor = "fortanix", feature(sgx_platform))]

use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

// All platforms that have some futex-like interface
#[cfg(any(
    target_os = "android",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "fuchsia",
    target_os = "linux",
    target_os = "ios",
    target_os = "macos",
    target_os = "openbsd",
    target_os = "redox",
    all(target_arch = "wasm32", target_feature = "atomics"),
    windows
))]
pub mod futex;

#[cfg(any(
    target_os = "android",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "fuchsia",
    target_os = "linux",
    target_os = "ios",
    target_os = "macos",
    target_os = "openbsd",
    target_os = "redox",
    all(target_arch = "wasm32", target_feature = "atomics"),
    windows
))]
#[doc(inline)]
pub use futex::{Futex, WakeupReason};

// All platforms for which the futex interface is always available.
#[cfg(all(
    any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd",
        target_os = "redox",
        all(target_arch = "wasm32", target_feature = "atomics")
    ),
    not(feature = "fallback")
))]
use futex as imp;

// Windows needs a fallback.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

#[cfg(all(any(target_os = "macos", target_os = "ios"), not(feature = "fallback")))]
mod darwin;

#[cfg(all(any(target_os = "macos", target_os = "ios"), not(feature = "fallback")))]
use darwin as imp;

#[cfg(unix)]
#[allow(unused)]
mod posix;

#[cfg(all(
    unix,
    any(
        not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "ios",
            target_os = "macos",
            target_os = "openbsd",
            target_os = "redox"
        )),
        feature = "fallback"
    )
))]
use posix as imp;

#[allow(unused)]
mod waiter_queue;

#[cfg(target_vendor = "fortanix")]
mod fortanix;
#[cfg(target_vendor = "fortanix")]
use fortanix as imp;

mod utils;

/// Multiple threads can wait on a single [`AtomicUsize`] until one thread wakes them all up at
/// once.
///
/// The [`AtomicUsize`] may be used for other purposes when there can be no threads waiting on it.
/// But when [`compare_and_wait`] and [`store_and_wake`] may be used, all but the five high-order
/// bits are reserved.
///
/// # Safety
/// To use this trait safely, ensure that the reserved bits are not modified while threads may be
/// waiting:
/// - All reserved bits must be zero before the first use of [`compare_and_wait`].
/// - None of the reserved bits are changed before [`store_and_wake`].
///
/// The constants [`FREE_BITS`], [`RESERVED_BITS`] and [`RESERVED_MASK`] can be helpful.
///
/// [`AtomicUsize`]: https://doc.rust-lang.org/core/sync/atomic/struct.AtomicUsize.html
/// [`compare_and_wait`]: #tymethod.compare_and_wait
/// [`store_and_wake`]: #tymethod.store_and_wake
/// [`FREE_BITS`]: constant.FREE_BITS.html
/// [`RESERVED_BITS`]: constant.RESERVED_BITS.html
/// [`RESERVED_MASK`]: constant.RESERVED_MASK.html
pub trait Waiters {
    /// Make the current thread wait until it receives a wake signal. Guaranteed not to wake up
    /// spuriously.
    ///
    /// The `expected` value is used to decide if this thread needs to be parked. This avoids a race
    /// condition where one thread may try to park itself, while another thread unparks it
    /// concurrently. It is also used to detect whether a wakeup was spurious, in wich case this
    /// `compare_and_wait` will repark the thread. Only the five non-reserved high order bits will
    /// be compared.
    ///
    /// # Atomic ordering
    /// `compare_and_wait` is a primitive intended for thread parking, not for data synchronization.
    /// The atomic comparions this function does before waiting and after waking are not guaranteed
    /// to have any ordering stronger than [`Relaxed`].
    ///
    /// If you need to read data written by the thread that waked this thread, manually do an
    /// [`Acquire`] after the `compare_and_wait`. This can be either a [`load`] on the atomic with
    /// [`Acquire`] ordering, or a [`fence`] with [`Acquire`] ordering.
    ///
    /// [`load`]: https://doc.rust-lang.org/core/sync/atomic/struct.AtomicUsize.html#method.load
    /// [`fence`]: https://doc.rust-lang.org/core/sync/atomic/fn.fence.html
    /// [`Acquire`]: https://doc.rust-lang.org/core/sync/atomic/enum.Ordering.html#variant.Acquire
    /// [`Relaxed`]: https://doc.rust-lang.org/core/sync/atomic/enum.Ordering.html#variant.Relaxed
    fn compare_and_wait(&self, expected: usize);

    /// Wake up all waiting threads.
    ///
    /// `new` must be provided to set `self` to some value that is not matched by the `expected`
    /// value passed to [`compare_and_wait`].
    ///
    /// # Atomic ordering
    /// The atomic store will be done with [`Release`] ordering. Other threads may do an [`Acquire`]
    /// after waking to see all writes made by this thread.
    ///
    /// # Safety
    /// If any of the reserved bits where changed while there where threads waiting, this function
    /// may fail to wake threads, or even dereference dangling pointers.
    ///
    /// [`compare_and_wait`]: #tymethod.compare_and_wait
    /// [`Acquire`]: https://doc.rust-lang.org/core/sync/atomic/enum.Ordering.html#variant.Acquire
    /// [`Release`]: https://doc.rust-lang.org/core/sync/atomic/enum.Ordering.html#variant.Release
    unsafe fn store_and_wake(&self, new: usize);
}

impl Waiters for AtomicUsize {
    fn compare_and_wait(&self, expected: usize) {
        imp::compare_and_wait(self, expected & !RESERVED_MASK)
    }

    unsafe fn store_and_wake(&self, new: usize) {
        imp::store_and_wake(self, new)
    }
}

/// One thread parkes itself on an `AtomicUsize`, and multiple threads or a timeout are able to wake
/// it up.
pub struct Parker {
    inner: imp::Parker,
}

impl Parker {
    pub const fn new() -> Parker {
        Parker {
            inner: imp::Parker::new(0),
        }
    }

    /// Parks the current thread.
    ///
    /// Only one thread can park on `self`. If `park` is called on an atomic that already has a
    /// thread parked on it, it will panic.
    ///
    /// # Timeouts and spurious wakeups
    /// If `timeout` is `None` this function will only return after another thread called [`unpark`]
    /// on `self`. It will repark the thread on spurious wakeups or interrupts.
    ///
    /// If there is a `timeout` specified, this thread can be woken up by an [`unpark`], by the
    /// expiration of the timeout, or spuriously.
    ///
    /// Platforms differ in the granularity of timeouts they support, the longest supported timeout,
    /// and what happens if the timeout is zero. This crate enforces some restrictions:
    /// - `park` panics if the timeout is 0.
    /// - Timeouts are rounded *up* to the nearest granularity supported by the platform.
    ///   Millisecond resolution is the coarsest of the current implementations.
    /// - The maximum timeout is on all platforms in the order of days or longer, so not really of
    ///   any concern. If the timeout overflows the maximum `park` will acts as if no timeout was
    ///   supplied.
    ///
    /// # Atomic ordering
    /// `park` will perform an atomic store with `Release` ordering before parking the thread. This
    /// guarantees that any preparations done before the call to `park` will actually have been
    /// executed before it. Neither the processor nor the compiler are allowed to reorder them to
    /// happen later.
    ///
    /// [`unpark`]: #method.unpark
    pub fn park(&self, timeout: Option<Duration>) {
        imp::park(&self.inner, timeout)
    }

    /// Unparks the waiting thread, if there is one.

    ///
    /// # Atomic ordering
    /// `unpark` will perform an atomic store with `Release` ordering. This guarantees that any
    /// preparations done before unparking the thread will actually be executed before the `unpark`.
    /// Neither the processor nor the compiler are not allowed to reorder them to happen later.
    #[allow(unused_unsafe)]
    pub fn unpark(&self) {
        unsafe { imp::unpark(&self.inner) }
    }
}

/// Number of high-order bits which are not reserved while using the
/// [`Waiters`](trait.Waiters.html) trait.
pub const FREE_BITS: usize = 5;
/// Number of low-order bits which are reserved while using the [`Waiters`](trait.Waiters.html)
/// trait.
pub const RESERVED_BITS: usize = mem::size_of::<usize>() * 8 - FREE_BITS;
/// Mask matching the bits which are reserved while using the [`Waiters`](trait.Waiters.html) trait.
pub const RESERVED_MASK: usize = (1 << RESERVED_BITS) - 1;

#[cfg(test)]
mod test {
    use crate::Parker;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread::spawn;
    use std::time::Duration;

    const TEST_ROUNDS: usize = 20_000;

    #[test]
    #[cfg(not(target_os = "redox"))] // currently causes a kernel panic
    fn unpark_ordering() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        static PARKER: Parker = Parker::new();

        for _ in 0..TEST_ROUNDS {
            spawn(|| {
                PARKER.unpark();
                FLAG.store(true, Ordering::Relaxed);
                PARKER.unpark();
            });

            while !FLAG.load(Ordering::Relaxed) {
                PARKER.park(None);
            }
            FLAG.store(false, Ordering::Relaxed);
        }
    }

    #[test]
    fn unpark_timeout() {
        let parker = Parker::new();
        parker.park(Some(Duration::from_millis(10)));
    }
}
