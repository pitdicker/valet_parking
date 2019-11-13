use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::{Parker, FREE_BITS, RESERVED_MASK};

// `UnsafeCell` because Posix needs mutable references to these types.
#[repr(align(64))]
pub struct PosixParker {
    mutex: UnsafeCell<libc::pthread_mutex_t>,
    condvar: UnsafeCell<libc::pthread_cond_t>,
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

impl Parker for AtomicUsize {
    fn park(&self) {
        park(self, None);
    }

    fn park_timed(&self, timeout: Duration) -> bool {
        let ts = convert_timeout(timeout);
        park(self, ts)
    }

    unsafe fn unpark(&self) {
        let old = self.fetch_or(NOTIFY_BIT, Ordering::SeqCst);
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
        let ptr = ((old & PTR_BITS) << FREE_BITS) as *const PosixParker;

        // Lock a mutex, set the signal, and release the mutex.
        // The parked thread will be woken up after releasing the mutex.
        // While holding the lock also clear the pointer part of `self`, so the unparked thread
        // knows this is not a spurious wakeup (that just happened to happen while we already set
        // the `NOTIFY_BIT` and were about to wake it up).
        let r = libc::pthread_mutex_lock((*ptr).mutex.get());
        debug_assert_eq!(r, 0);
        self.fetch_and(!PTR_BITS, Ordering::SeqCst);
        let r = libc::pthread_cond_signal((*ptr).condvar.get());
        debug_assert_eq!(r, 0);
        let r = libc::pthread_mutex_unlock((*ptr).mutex.get());
        debug_assert_eq!(r, 0);
    }
}

// Returns false if the wakeup was because of the timeout, or spurious.
fn park(atomic: &AtomicUsize, timeout: Option<libc::timespec>) -> bool {
    let parker = PosixParker {
        mutex: UnsafeCell::new(libc::PTHREAD_MUTEX_INITIALIZER),
        condvar: UnsafeCell::new(libc::PTHREAD_COND_INITIALIZER),
    };
    let ptr = (&parker as *const PosixParker as usize) >> FREE_BITS;

    unsafe {
        // Lock the mutex before making a pointer to `parker` available to other threads.
        let r = libc::pthread_mutex_lock(parker.mutex.get());
        debug_assert_eq!(r, 0);

        let mut current = atomic.load(Ordering::SeqCst);
        let mut res = true;
        loop {
            // If the old state had its `NOTIFY_BIT` set, some other thread unparked us even
            // before we were able to park ourselves. Then stop trying to park ourselves and
            // clean up.
            if current & RESERVED_MASK == NOTIFY_BIT {
                break;
            }

            let old = atomic.compare_and_swap(current, current | ptr, Ordering::SeqCst);
            if old != current {
                current = old;
                continue;
            }

            if let Some(ts) = timeout {
                res = condvar_wait_timed(atomic, &parker, &ts);
            } else {
                condvar_wait(atomic, &parker);
            }
            break;
        }

        // Done, clean up.
        let r = libc::pthread_mutex_unlock(parker.mutex.get());
        debug_assert_eq!(r, 0);
        let r = libc::pthread_mutex_destroy(parker.mutex.get());
        debug_assert_eq!(r, 0);
        let r = libc::pthread_cond_destroy(parker.condvar.get());
        debug_assert_eq!(r, 0);
        atomic.fetch_and(!NOTIFY_BIT, Ordering::SeqCst);
        res
    }
}

fn condvar_wait(atomic: &AtomicUsize, parker: &PosixParker) {
    unsafe {
        loop {
            // Wait on a signal through the condvar; mutex gets released
            let r = libc::pthread_cond_wait(parker.condvar.get(), parker.mutex.get());
            // We got woken up; mutex is locked again.
            debug_assert_eq!(r, 0);
            // Make sure this wakeup was not spurious, otherwise park again.
            // The pointer must be gone, and the `NOTIFY_BIT` must be set.
            if atomic.load(Ordering::SeqCst) & RESERVED_MASK == NOTIFY_BIT {
                break;
            }
        }
    }
}

fn condvar_wait_timed(atomic: &AtomicUsize, parker: &PosixParker, ts: &libc::timespec) -> bool {
    unsafe {
        // Wait on a signal through the condvar; mutex gets released
        let r = libc::pthread_cond_timedwait(parker.condvar.get(), parker.mutex.get(), ts);
        // We got woken up; mutex is locked again.
        debug_assert_eq!(r, 0);
        let current = atomic.load(Ordering::SeqCst);
        if current & NOTIFY_BIT != NOTIFY_BIT {
            // If this wakeup was not caused by another thread waking us, but was spurious or
            // because the timeout expired.
            loop {
                // Try to set the state to not parked (and not notified).
                let old =
                    atomic.compare_and_swap(current, current & !RESERVED_MASK, Ordering::SeqCst);
                if old == current {
                    return true;
                } else if old & NOTIFY_BIT == NOTIFY_BIT {
                    // Some other thread just happened to try waking us right now, while we were
                    // already woken up by the timeout. It is now blocked on our mutex. We have
                    // to let it take the mutex and wake us, otherwise it will read through a
                    // dangling pointer when we return.
                    let r = libc::pthread_cond_wait(parker.condvar.get(), parker.mutex.get());
                    debug_assert_eq!(r, 0);
                    debug_assert_eq!(atomic.load(Ordering::SeqCst) & RESERVED_MASK, NOTIFY_BIT);
                    return false;
                }
            }
        }
    }
    return true;
}

// x32 Linux uses a non-standard type for tv_nsec in timespec.
// See https://sourceware.org/bugzilla/show_bug.cgi?id=16437
#[cfg(all(target_arch = "x86_64", target_pointer_width = "32"))]
#[allow(non_camel_case_types)]
type tv_nsec_t = i64;
#[cfg(not(all(target_arch = "x86_64", target_pointer_width = "32")))]
#[allow(non_camel_case_types)]
type tv_nsec_t = libc::c_long;

fn convert_timeout(timeout: Duration) -> Option<libc::timespec> {
    if timeout.as_secs() > libc::time_t::max_value() as u64 {
        return None;
    }
    Some(libc::timespec {
        tv_sec: timeout.as_secs() as libc::time_t,
        tv_nsec: timeout.subsec_nanos() as tv_nsec_t,
    })
}
