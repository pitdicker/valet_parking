use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use std::os::fortanix_sgx::thread;
use std::os::fortanix_sgx::usercalls;
use std::os::fortanix_sgx::usercalls::raw::{Tcs, EV_UNPARK, WAIT_INDEFINITE};

use crate::waiter_queue;
use crate::{FREE_BITS, RESERVED_MASK};

pub(crate) use waiter_queue::{compare_and_wait, store_and_wake};

#[repr(align(64))]
pub struct TcsParker {
    tcs: Tcs,
}

// # State table (of the reserved bits):
//
// PTR_BITS | NOTIFY_BIT | Description
//     0    |     0      | Thread is not parked, and also not just woken up.
// ---------+------------+------------------------------------------------------------------
//   some   |     0      | Thread is parked. If the parked thread sees this state on wakeup,
//          |            | the wakeup must be spurious and it should park itself again.
// ---------+------------+------------------------------------------------------------------
//   some   |     1      | Thread is still parked, but some thread is in the process of
//          |            | waking it up.
// ---------+------------+------------------------------------------------------------------
//     0    |     1      | Thread got woken up by another thread.
// ---------+------------+------------------------------------------------------------------

const NOTIFY_BIT: usize = 1;
const PTR_BITS: usize = RESERVED_MASK ^ NOTIFY_BIT;

// Returns false if the wakeup was because of the timeout, or spurious.
pub(crate) fn park(atomic: &AtomicUsize, timeout: Option<Duration>) {
    if timeout.is_some() {
        panic!("Timeouts for usercalls::wait are supported in Fortanix SGX");
    }
    let parker = TcsParker {
        tcs: thread::current(),
    };
    let ptr = (&parker as *const TcsParker as usize) >> FREE_BITS;

    let mut current = atomic.load(Ordering::SeqCst);
    loop {
        if current & RESERVED_MASK != 0 {
            // See https://github.com/fortanix/rust-sgx/issues/31
            panic!("Tried to call park on an atomic while another thread is already parked on it");
        }
        match atomic.compare_exchange_weak(
            current,
            current | ptr,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(x) => current = x,
        }
    }
    loop {
        let r = usercalls::wait(EV_UNPARK, WAIT_INDEFINITE);
        if let Err(e) = r {
            debug_assert!(false, "Unexpected return value of usercalls::wait: {}", e);
        }
        if atomic.load(Ordering::Relaxed) & RESERVED_MASK == NOTIFY_BIT {
            break;
        }
    }
}

pub(crate) unsafe fn unpark(atomic: &AtomicUsize) {
    let old = atomic.fetch_or(NOTIFY_BIT, Ordering::SeqCst);
    match (old & PTR_BITS, old & NOTIFY_BIT == NOTIFY_BIT) {
        (_, true) => {
            // Some other thread must be in the process of unparking the suspended thread.
            // There is nothing for us to do.
            return;
        }
        (0, false) => {
            // There is no thread to wake up, maybe it didn't even get to parking itself yet.
            return;
        }
        (_, false) => {} // Good to go.
    }

    // The parked thread will not return from `self.park` while `NOTIFY_BIT` and a pointer is
    // set, so we can safely access data on its stack through the pointer encoded in `self`.
    let ptr = ((old & PTR_BITS) << FREE_BITS) as *const TcsParker;
    let target_tcs = (*ptr).tcs;

    // Remove the pointer bits, from now on the `TcsParker` may get freed (if the thread wakes up
    // spuriously).
    &atomic.fetch_and(!PTR_BITS, Ordering::Relaxed);
    let r = usercalls::send(EV_UNPARK, Some(target_tcs));
    if let Err(e) = r {
        debug_assert!(false, "Unexpected return value of usercalls::send: {}", e);
    }
}
