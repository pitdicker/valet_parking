use core::sync::atomic::Ordering::{Relaxed, Release};
use core::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use core::time::Duration;

use crate::RESERVED_MASK;
use crate::futex::*;

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
