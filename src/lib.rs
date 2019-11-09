//! `valet_boy` provides a cross-platform abstraction over thread parking. The goal is to provide an
//! abstraction with little overhead, which is `no_std`-compatible and requires little overhead.

use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux;

pub trait Waiters {
    /// Park the current thread. Reparks after a spurious wakeup.
    unsafe fn wait<P>(&self, should_wait: P)
    where
        P: Fn(usize) -> bool;

    /// Unpark all waiting threads. `new` must be provided set `self` to some value that is not
    /// matched by the `should_park` function passed to `park`.
    ///
    /// # Safety
    /// * Don't change any of the reserved bits while there can be threads parked.
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
