use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::{RESERVED_BITS, RESERVED_MASK};

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
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason;

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
// Implementation of the Waiter trait
//
const HAS_WAITERS: usize = 0x1;
pub(crate) fn compare_and_wait(atomic: &AtomicUsize, compare: usize) {
    loop {
        match atomic.compare_exchange_weak(compare, compare | HAS_WAITERS, Ordering::Relaxed, Ordering::Relaxed) {
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
        atomic.futex_wait(compare | HAS_WAITERS, None);
        if atomic.load(Ordering::Relaxed) != (compare | HAS_WAITERS) {
            break;
        }
    }
}

pub(crate) fn store_and_wake(atomic: &AtomicUsize, new: usize) {
    if atomic.swap(new, Ordering::SeqCst) & HAS_WAITERS == HAS_WAITERS {
        atomic.futex_wake();
    }
}

//
// Implementation of the Parker trait
//

// Layout of the atomic:
// FFFFFPP0_00000000_00000000_00000000_[00000000_00000000_00000000_00000000]
//
// F: Free bits, available for user
// P: Parker state
// 0: Unused bits
//
// On several 64-bit systems the futex operation compares only 32 bits. We give it the 32 bits that
// contain the bits reserved for the user, and we must give it our parking state bits. That is why
// Parking state is the first to come after the Free bits.

// States for Parker
const NOT_PARKED: usize = 0x0 << (RESERVED_BITS - 2);
const PARKED: usize = 0x1 << (RESERVED_BITS - 2);
const NOTIFIED: usize = 0x2 << (RESERVED_BITS - 2);

const STATE_MASK: usize = 0x3 << (RESERVED_BITS - 2);
#[allow(unused)] // not used by all implementations
pub(crate) const COUNTER_MASK: usize = RESERVED_MASK ^ STATE_MASK;

pub(crate) fn park(atomic: &AtomicUsize, timeout: Option<Duration>) {
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

pub(crate) fn unpark(atomic: &AtomicUsize) {
    let current = atomic.load(Ordering::SeqCst);
    if atomic
        .compare_exchange(
            current,
            (current & !STATE_MASK) | NOTIFIED,
            Ordering::Relaxed,
            Ordering::Relaxed,
        )
        .is_err()
    {
        // We were unable to switch the state to `NOTIFIED`; Some other thread must be in the
        // process of unparking it. Either way the parked thread is being waked, and there is
        // nothing for us to do.
        return;
    }
    atomic.futex_wake();
}
