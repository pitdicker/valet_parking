use core::sync::atomic::Ordering::{Relaxed, Release};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use core::time::Duration;
#[cfg(feature = "std")]
use std::time::Instant;

use crate::RESERVED_MASK;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod darwin; // relative
#[cfg(target_os = "dragonfly")]
mod dragonfly; // relative
#[cfg(target_os = "freebsd")]
mod freebsd; // relative/absolute, monotonic/realtime
#[cfg(target_os = "fuchsia")]
mod fuchsia; // relative/absolute?
#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux; // relative/absolute, monotonic/realtime
#[cfg(target_os = "openbsd")]
mod openbsd; // relative
#[cfg(target_os = "redox")]
mod redox; // relative monotonic
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
mod wasm_atomic; // relative
#[cfg(windows)]
mod windows; // relative (keyedevent also absolute)

/// Reason the operating system provided for waking up a thread. Because of the limited guarantees
/// of some platforms, this turns out not to be all that useful except for documentation purposes.
#[allow(dead_code)]
pub(crate) enum WakeupReason {
    /// Thread did not get parked, because the compare value did not match.
    /// Not all operating systems report this case.
    NoMatch,
    /// Thread got woken up because its timeout expired.
    /// Only DragonFly BSD does not report this reliably.
    TimedOut,
    /// Thread got woken up because of an interrupt.
    Interrupt,
    /// Thread got woken up by a `wake` call.
    WokenUp,
    /// Thread may be woken up by a `wake` call, but it may also have been for other reasons.
    Unknown,
}

pub(crate) trait Futex {
    /// Park the current thread if `self` equals `compare`. Most implementations will only compare
    /// the 32 high-order bits.
    ///
    /// `timeout` is relative duration, not an absolute deadline.
    ///
    /// This function does not guard against spurious wakeups.
    fn wait(&self, compare: i32, timeout: Option<Duration>) -> WakeupReason;

    #[cfg(feature = "std")]
    fn wait_until(&self, compare: i32, deadline: Instant) -> WakeupReason {
        unimplemented!();
    }

    /// Wake all threads waiting on `self`, and set `self` to `new`.
    ///
    /// Some implementations need to set `self` to another value before waking up threads, in order
    /// to detect spurious wakeups. Other implementations need to change `self` later, like NT Keyed
    /// Events for one needs to know the number of threads parked. So we make it up to the
    /// implementation to set set `self` to `new`.
    ///
    /// We don't support waking n out of m waiting threads. This gets into pretty advanced use cases,
    /// and it is not clear this can be supported cross-platform and without too much overhead.
    fn wake(&self) -> usize;
}

//
// Implementation of the Waiters trait
//
const HAS_WAITERS: usize = 0x1 << UNCOMPARED_LO_BITS;
pub(crate) fn compare_and_wait(atomic: &AtomicUsize, compare: usize) {
    loop {
        match atomic.compare_exchange_weak(
            compare,
            compare | HAS_WAITERS,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(current) => {
                if current & !RESERVED_MASK != compare {
                    return;
                }
                debug_assert!(current == compare | HAS_WAITERS);
            }
        }
    }
    loop {
        unsafe {
            let atomic_i32 = get_i32_ref(atomic);
            let compare = ((compare | HAS_WAITERS) >> UNCOMPARED_LO_BITS) as u32 as i32;
            atomic_i32.wait(compare, None);
        }
        if atomic.load(Ordering::Relaxed) != (compare | HAS_WAITERS) {
            break;
        }
    }
}

pub(crate) fn store_and_wake(atomic: &AtomicUsize, new: usize) {
    if atomic.swap(new, Ordering::Release) & HAS_WAITERS == HAS_WAITERS {
        unsafe {
            let atomic_i32 = get_i32_ref(atomic);
            atomic_i32.wake();
        }
    }
}

/// The `Waiters` trait has to be implemented on an `AtomicUsize` because we need a pointer-sized
/// value for some implementations. But the `Futex` trait is implemented on an `AtomicI32` because
/// that is wait the OS interface relies on. On 64-bit platforms we are going to crate a reference
/// to only a 32-bit portion of the `AtomicUsize`.
///
/// There are the obvious concerns about size, alignment, and endianness. But this is above all a
/// questionable operation because it is not explicitly supported by the C++ memory model. There is
/// little information on what happens when you do atomic operations on only a part of the atomic.
/// One paper is [Mixed-size Concurrency: ARM, POWER, C/C++11, and SC][Mixed-size Concurrency].
///
/// We should not assume that the kernel does anything stonger with the atomic than a relaxed load.
/// But it may also do a CAS loop that writes to the atomic, as long as the value is not modified
/// (DragonFly BSD is a documented case).
///
/// The one thing to worry about for us is preserving *modification order consistency* of the atomic
/// integer. This normally relies on the integer having the same address. The processor may not
/// track 'overlapping footprints' of the smaller integer (as the paper calls it). So when the
/// smaller integer part of an atomic starts at a different address, we would have to use orderings
/// such as Release or SeqCst to prevent reordering of operations on the smaller integer with
/// operations on the full atomic.
///
/// As we don't control the memory orderings the kernel uses, our only option is to use the part of
/// the atomic that starts at the same address. On little-endian this are the 32 low-order bits, on
/// big-endian the 32 high-order bits. Notably this part may not contain the (high-order) bits that
/// match the `compare` value of `compare_and_wait`.
///
/// Mixed-size Concurrency: https://hal.inria.fr/hal-01413221/document
unsafe fn get_i32_ref(ptr_sized: &AtomicUsize) -> &AtomicI32 {
    &*(ptr_sized as *const AtomicUsize as *const AtomicI32)
}
#[cfg(target_pointer_width = "32")]
const UNCOMPARED_LO_BITS: usize = 0;
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
const UNCOMPARED_LO_BITS: usize = 0;
#[cfg(all(target_pointer_width = "64", target_endian = "big"))]
const UNCOMPARED_LO_BITS: usize = 32;

//
// Implementation of the Parker trait
//
pub(crate) type Parker = AtomicI32;

// States for Parker
const NOT_PARKED: i32 = 0x0;
const PARKED: i32 = 0x1;
const NOTIFIED: i32 = 0x2;

pub(crate) fn park(atomic: &AtomicI32, timeout: Option<Duration>) {
    match atomic.compare_exchange(NOT_PARKED, PARKED, Release, Relaxed) {
        Ok(_) => {}
        Err(NOTIFIED) => {
            atomic.store(NOT_PARKED, Relaxed);
            return;
        }
        Err(_) => panic!(
            "Tried to call park on an atomic while \
             another thread is already parked on it"
        ),
    };
    loop {
        atomic.wait(PARKED, timeout);
        if timeout.is_some() {
            // We don't guarantee there are no spurious wakeups when there was a timeout supplied.
            atomic.store(NOT_PARKED, Relaxed);
            return;
        }
        if atomic
            .compare_exchange(NOTIFIED, NOT_PARKED, Relaxed, Relaxed)
            .is_ok()
        {
            break;
        }
    }
}

pub(crate) fn unpark(atomic: &AtomicI32) {
    if atomic.swap(NOTIFIED, Release) == PARKED {
        atomic.wake();
    }
}
