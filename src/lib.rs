//! `valet_boy` provides a cross-platform abstraction over thread parking. The goal is to provide an
//! abstraction with little overhead, which is `no_std`-compatible and requires little overhead.
#![no_std]
#![cfg_attr(all(target_arch = "wasm32", target_feature = "atomics"), feature(stdsimd))]

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

#[cfg(unix)]
mod errno;

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

// Multiple threads can wait on a single `AtomicUsize` until one thread wakes them all up at once.
pub trait Waiters {
    /// Park the current thread. Reparks after a spurious wakeup.
    ///
    /// `compare` is used to decide if this thread needs to be parked. This avoids a race condition
    /// where one thread may try to park itself, while another thread unparks it concurrently. It is
    /// also used to detect whether a wakeup was spurious, in wich case this function will repark
    /// the thread.
    ///
    /// Only the five non-reserved bits will be compared, all other bits must be zero.
    fn compare_and_wait(&self, compare: usize);

    /// Unpark all waiting threads.
    ///
    ///`new` must be provided to set `self` to some value that is not matched by the `compare`
    /// variable passed to `park`. It will be stored with Release ordering or stronger.
    ///
    /// Returns the number of waiting threads that were woken up. FIXME: todo
    ///
    /// # Safety
    /// If any of the reserved bits where changed while there where threads waiting, this function
    /// may fail to wake threads, or even dereference dangling pointers.
    unsafe fn store_and_wake(&self, new: usize);
}

impl Waiters for AtomicUsize {
    fn compare_and_wait(&self, compare: usize) {
        imp::compare_and_wait(self, compare)
    }

    unsafe fn store_and_wake(&self, new: usize) {
        imp::store_and_wake(self, new)
    }
}

/// One thread parkes itself on an `AtomicUsize`, and multiple threads or a timeout are able to wake
/// it up.
pub trait Parker {
    /// Parks the current thread.
    ///
    /// Only one thread can park on `self`. If `park` is called on an atomic that already has a
    /// thread parked on it, it will panic.
    ///
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
    ///   any concern. `park` ignores the timeout if it overflows the maximum.
    ///
    /// [`unpark`]: Parker::unpark
    fn park(&self, timeout: Option<Duration>);

    /// Unparks the waiting thread, if there is one.
    ///
    /// # Safety
    /// If any of the reserved bits where changed while there whas a thread parked, this function
    /// may fail to unpark it, or may even dereference a dangling pointer.
    unsafe fn unpark(&self);
}

impl Parker for AtomicUsize {
    fn park(&self, timeout: Option<Duration>) {
        imp::park(self, timeout)
    }

    unsafe fn unpark(&self) {
        imp::unpark(self)
    }
}

pub const FREE_BITS: usize = 5;
pub const RESERVED_BITS: usize = mem::size_of::<usize>() * 8 - FREE_BITS;
pub const RESERVED_MASK: usize = (1 << RESERVED_BITS) - 1;

// Convert this pointer to an `AtomicUsize` to a pointer to an `*const u32`, pointing to the part
// containing the non-reserved bits.
#[allow(unused)]
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
pub(crate) fn as_u32_pub(ptr: *const AtomicUsize) -> *const u32 {
    unsafe { (ptr as *const _ as *const u32).offset(1) }
}

// Convert this pointer to an `AtomicUsize` to a pointer to an `*const u32`, pointing to the part
// containing the non-reserved bits.
#[allow(unused)]
#[cfg(any(
    target_pointer_width = "32",
    all(target_pointer_width = "64", target_endian = "big")
))]
pub(crate) fn as_u32_pub(ptr: *const AtomicUsize) -> *const u32 {
    ptr as *const _ as *const u32
}

// Convert this pointer to an `AtomicUsize` to a pointer to an `*const u32`, pointing to the part
// containing only reserved bits.
#[allow(unused)]
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
pub(crate) fn as_u32_priv(ptr: *const AtomicUsize) -> *const u32 {
    ptr as *const _ as *const u32
}

// Convert this pointer to an `AtomicUsize` to a pointer to an `*const u32`, pointing to the part
// containing only reserved bits.
#[allow(unused)]
#[cfg(any(
    target_pointer_width = "32",
    all(target_pointer_width = "64", target_endian = "big")
))]
pub(crate) fn as_u32_priv(ptr: *const AtomicUsize) -> *const u32 {
    unsafe { (ptr as *const _ as *const u32).offset(1) }
}
