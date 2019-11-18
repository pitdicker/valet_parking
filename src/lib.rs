//! `valet_boy` provides a cross-platform abstraction over thread parking. The goal is to provide an
//! abstraction with little overhead, which is `no_std`-compatible and requires little overhead.

use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use cfg_if::cfg_if;

cfg_if! {
    if #[cfg(all(unix, feature = "posix"))] {
        mod posix;
        mod waiter_queue;
    } else if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod linux;
        mod futex_like;
    } else if #[cfg(target_os = "freebsd")] {
        mod freebsd;
        mod futex_like;
    } else if #[cfg(target_os = "redox")] {
        mod redox;
        mod futex_like;
    } else if #[cfg(target_os = "fuchsia")] {
        mod fuchsia;
        mod futex_like;
    } else if #[cfg(any(target_os = "macos", target_os = "ios"))] {
        mod darwin;
        mod futex_like;
    } else if #[cfg(target_os = "openbsd")] {
        mod openbsd;
        mod futex_like;
    } else if #[cfg(target_os = "dragonfly")] {
        mod dragonfly;
        mod futex_like;
    } else if #[cfg(unix)] {
        mod posix;
        mod waiter_queue;
    } else if #[cfg(windows)] {
        mod windows;
        mod futex_like;
    }
}

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
