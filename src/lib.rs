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
    } else if #[cfg(unix)] {
        mod posix;
        mod waiter_queue;
    } else if #[cfg(windows)] {
        mod windows;
        mod futex_like;
    }
}

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
    /// # Safety
    /// If any of the reserved bits where changed while there where threads waiting, this function
    /// may fail to wake threads, or even dereference invalid pointers.
    unsafe fn store_and_wake(&self, new: usize);
}

pub trait Parker {
    fn park(&self);

    fn park_timed(&self, timeout: Duration) -> bool;

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
