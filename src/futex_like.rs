use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::{Parker, Waiters, RESERVED_BITS, RESERVED_MASK};

pub(crate) trait FutexLike {
    // Park the current thread if `self` equals `compare`. Most implementations will only compare
    // the 32 high-order bits.
    //
    // `timeout` is relative duration, not an absolute deadline.
    //
    // This function does not guard against spurious wakeups.
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>);

    // Wake all threads waiting on `self`, and set `self` to `new`.
    //
    // Some implementations need to set `self` to another value before waking up threads, in order
    // to detect spurious wakeups. Other implementations need to change `self` later, like NT Keyed
    // Events for one needs to know the number of threads parked. So we make it up to the
    // implementation to set set `self` to `new`.
    //
    // We don't support waking n out of m waiting threads. This gets into pretty advanced use cases,
    // and it is not clear this can be supported cross-platform and without too much overhead.
    fn futex_wake(&self, new: usize);
}

// Layout of the atomic:
// FFFFFPPC_CCCCCCCC_CCCCCCCC_CCCCCCCC_[CCCCCCCC_CCCCCCCC_CCCCCCCC_CCCCCCCC]
//
// F: Free bits, available for user
// P: Parker state
// C: Counter for the number of waiting threads
//
// On several 64-bit systems the futex operation compares only 32 bits. We give it the 32 bits that
// contain the bits reserved for the user, and we must give it our parking state bits. That is why
// Parking state is the first to come after the Free bits.
//
// For Windows NT Keyed Events we need to keep track of the number of threads that should be waked.
// So all the remaining bits are used for the Counter.

// States for Parker
const NOT_PARKED: usize = 0x0 << (RESERVED_BITS - 2);
const PARKED: usize = 0x1 << (RESERVED_BITS - 2);
const NOTIFIED: usize = 0x2 << (RESERVED_BITS - 2);

const STATE_MASK: usize = 0x3 << (RESERVED_BITS - 2);
#[allow(unused)] // not used by all implementations
pub(crate) const COUNTER_MASK: usize = RESERVED_MASK ^ STATE_MASK;

impl Waiters for AtomicUsize {
    fn compare_and_wait(&self, compare: usize) {
        assert_eq!(compare & RESERVED_MASK, 0);
        loop {
            self.futex_wait(compare, None);
            if self.load(Ordering::SeqCst) != compare {
                break;
            }
        }
    }

    unsafe fn store_and_wake(&self, new: usize) {
        self.futex_wake(new);
    }
}

impl Parker for AtomicUsize {
    fn park(&self) {
        let mut current = self.load(Ordering::SeqCst);
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

            let old = self.compare_and_swap(current, current | PARKED, Ordering::Relaxed);
            if old != current {
                // `self` was modified by some other thread, restart from the beginning.
                current = old;
                continue;
            }

            while current & STATE_MASK != NOTIFIED {
                self.futex_wait(current | PARKED, None);
                // Load `self` so the next iteration of this loop can make sure this wakeup was not
                // spurious, and otherwise park again.
                current = self.load(Ordering::Relaxed);
            }
            break;
        }
        // Reset state to `NOT_PARKED`.
        &self.fetch_and(!STATE_MASK, Ordering::Relaxed);
    }

    fn park_timed(&self, _timeout: Duration) -> bool {
        unimplemented!();
    }

    unsafe fn unpark(&self) {
        let current = self.load(Ordering::SeqCst);
        if self
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
        self.futex_wake(NOTIFIED);
    }
}
