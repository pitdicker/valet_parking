use core::mem;
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use core::time::Duration;

use crate::RESERVED_MASK;

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod darwin;
#[cfg(target_os = "dragonfly")]
mod dragonfly;
#[cfg(target_os = "freebsd")]
mod freebsd;
#[cfg(target_os = "fuchsia")]
mod fuchsia;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux;
#[cfg(target_os = "openbsd")]
mod openbsd;
#[cfg(target_os = "redox")]
mod redox;
#[cfg(all(target_arch = "wasm32", target_feature = "atomics"))]
mod wasm_atomic;
#[cfg(windows)]
mod windows;

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
    /// Thread got woken up by a `futex_wake` call.
    WokenUp,
    /// Thread may be woken up by a `futex_wake` call, but it may also have been for other reasons.
    Unknown,
}

pub(crate) trait Futex {
    /// Park the current thread if `self` equals `compare`. Most implementations will only compare
    /// the 32 high-order bits.
    ///
    /// `timeout` is relative duration, not an absolute deadline.
    ///
    /// This function does not guard against spurious wakeups.
    fn futex_wait(&self, compare: i32, timeout: Option<Duration>) -> WakeupReason;

    /// Wake all threads waiting on `self`, and set `self` to `new`.
    ///
    /// Some implementations need to set `self` to another value before waking up threads, in order
    /// to detect spurious wakeups. Other implementations need to change `self` later, like NT Keyed
    /// Events for one needs to know the number of threads parked. So we make it up to the
    /// implementation to set set `self` to `new`.
    ///
    /// We don't support waking n out of m waiting threads. This gets into pretty advanced use cases,
    /// and it is not clear this can be supported cross-platform and without too much overhead.
    fn futex_wake(&self) -> usize;
}

//
// Implementation of the Waiters trait
//
const HAS_WAITERS: usize = 0x1 << UNCOMPARED_BITS;
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
            let compare = ((compare | HAS_WAITERS) >> UNCOMPARED_BITS) as u32 as i32;
            atomic_i32.futex_wait(compare, None);
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
            atomic_i32.futex_wake();
        }
    }
}

// Transmuting an `AtomicUsize` to an `AtomicI32` is a bit of a questionable operations. I believe
// it is safe because:
// * On 32 bit it is a no-op.
// * On 64 bit:
//   - The size of the `AtomicI32` is less then `AtomicUsize`;
//   - The alignment of the `AtomicUsize` is greater than or equal to `AtomicI32`;
//   - We take care of endianness by taking the porting of the `AtomicUsize` that contains the
//     non-reserved bits.
//   - There are no stores done on the resulting atomic, it is only passed to the kernel to do one
//     load for the comparison.
// * We don't need to support 16-bit pointers, as all the operating systems that offer futex-like
//   interfaces are 32-bit+.
#[cfg(any(
    target_pointer_width = "32",
    all(target_pointer_width = "64", target_endian = "big")
))]
unsafe fn get_i32_ref(ptr_sized: &AtomicUsize) -> &AtomicI32 {
    &*(ptr_sized as *const AtomicUsize as *const AtomicI32)
}
#[cfg(all(target_pointer_width = "64", target_endian = "little"))]
unsafe fn get_i32_ref(ptr_sized: &AtomicUsize) -> &AtomicI32 {
    &*((ptr_sized as *const AtomicUsize as *const AtomicI32).offset(1))
}

const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

//
// Implementation of the Parker trait
//
pub(crate) type Parker = AtomicI32;

// States for Parker
const NOT_PARKED: i32 = 0x0;
const PARKED: i32 = 0x1;
const NOTIFIED: i32 = 0x2;
const STATE_MASK: i32 = 0x3;

pub(crate) fn park(atomic: &AtomicI32, timeout: Option<Duration>) {
    let mut current = atomic.load(Ordering::SeqCst);
    loop {
        match current & STATE_MASK {
            // Good to go
            NOT_PARKED => {}
            // Some other thread unparked us even before we were able to park ourselves.
            NOTIFIED => break,
            // There is already some thread parked on this atomic; calling `park` on it is not
            // allowed by the API.
            PARKED | _ => panic!(),
        }

        let old = atomic.compare_and_swap(current, current | PARKED, Ordering::Relaxed);
        if old != current {
            // `self` was modified by some other thread, restart from the beginning.
            current = old;
            continue;
        }

        if timeout.is_some() {
            atomic.futex_wait(current | PARKED, timeout);
        } else {
            while current & STATE_MASK != NOTIFIED {
                atomic.futex_wait(current | PARKED, None);
                // Load `self` so the next iteration of this loop can make sure this wakeup was not
                // spurious, and otherwise park again.
                current = atomic.load(Ordering::Relaxed);
            }
        }
        break;
    }
    // Reset state to `NOT_PARKED`.
    &atomic.fetch_and(!STATE_MASK, Ordering::Relaxed);
}

pub(crate) fn unpark(atomic: &AtomicI32) {
    let current = atomic.load(Ordering::SeqCst);
    if atomic
        .compare_exchange(
            (current & !STATE_MASK) | PARKED,
            (current & !STATE_MASK) | NOTIFIED,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_err()
    {
        // We were unable to switch the state from `PARKED` to `NOTIFIED`. Either there is no
        // thread parked, or some other thread must be in the process of unparking it. In both cases
        // there is nothing for us to do.
        return;
    }
    atomic.futex_wake();
}
