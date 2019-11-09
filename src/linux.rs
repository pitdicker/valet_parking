use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::{as_u32_pub, Parker, Waiters, RESERVED_MASK};

// Linux futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl Waiters for AtomicUsize {
    unsafe fn wait<P>(&self, should_park: P)
    where
        P: Fn(usize) -> bool,
    {
        let ptr = as_u32_pub(self) as *mut i32;
        loop {
            let current = self.load(Ordering::SeqCst);
            if !should_park(current & !RESERVED_MASK) {
                break;
            }
            let compare = (current >> UNCOMPARED_BITS) as u32;
            futex_wait(ptr, compare as i32, None);
        }
    }

    unsafe fn store_and_wake(&self, new: usize) {
        self.store(new, Ordering::SeqCst);
        let ptr = as_u32_pub(self) as *mut i32;
        futex_wake(ptr, i32::max_value());
    }
}

// States
const NOT_PARKED: usize = 0 << UNCOMPARED_BITS;
const PARKED: usize = 1 << UNCOMPARED_BITS;
const NOTIFIED: usize = 2 << UNCOMPARED_BITS;
const STATE_MASK: usize = 3 << UNCOMPARED_BITS;

impl Parker for AtomicUsize {
    fn park(&self) {
        let ptr = as_u32_pub(self) as *mut i32;
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
                let compare = ((current | PARKED) >> UNCOMPARED_BITS) as u32;
                futex_wait(ptr, compare as i32, None);
                // Load `self` so the next iteration of this loop can make sure this wakeup was not
                // spurious, and otherwise park again.
                current = self.load(Ordering::Relaxed);
            }
            break;
        }
        // Reset state to `NOT_PARKED`.
        self.fetch_and(!STATE_MASK, Ordering::Relaxed);
    }

    fn park_timed(&self, _timeout: Duration) -> bool {
        unimplemented!();
    }
    /*
        fn park_timed(&self, timeout: Duration) -> bool {
            let ptr = as_u32_pub(self) as *mut i32;
            let ts = libc::timespec {
                tv_sec: timeout.as_secs() as libc::time_t,
                tv_nsec: timeout.subsec_nanos() as tv_nsec_t,
            };
            loop {
                let current = self.load(Ordering::Relaxed);
                if !should_park(current & !RESERVED_MASK) {
                    break;
                }
                let compare = (current >> (8 * (mem::size_of::<usize>() - mem::size_of::<u32>()))) as u32;
                futex_wait(ptr, compare as i32, Some(ts));
            }
        }
    */
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
        let ptr = as_u32_pub(self) as *mut i32;
        futex_wake(ptr, 1);
    }
}

fn errno() -> libc::c_int {
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location()
    }
    #[cfg(target_os = "android")]
    unsafe {
        *libc::__errno()
    }
}

unsafe fn futex(
    uaddr: *mut libc::c_int,
    futex_op: libc::c_int,
    val: libc::c_int,
    timeout: *const libc::timespec,
    uaddr2: *mut libc::c_void,
    val3: libc::c_int,
) -> libc::c_long {
    libc::syscall(libc::SYS_futex, uaddr, futex_op, val, timeout, uaddr2, val3)
}

#[inline]
fn futex_wait(atomic: *mut i32, current: i32, ts: Option<libc::timespec>) {
    let ts_ptr = ts
        .as_ref()
        .map(|ts_ref| ts_ref as *const _)
        .unwrap_or(ptr::null());
    let r = unsafe {
        futex(
            atomic,
            libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
            current,
            ts_ptr,
            ptr::null_mut(),
            0,
        )
    };
    debug_assert!(r == 0 || r == -1);
    if r == -1 {
        debug_assert!(
            errno() == libc::EINTR
                || errno() == libc::EAGAIN
                || (ts.is_some() && errno() == libc::ETIMEDOUT)
        );
    }
}

fn futex_wake(atomic: *mut i32, max_threads_to_wake: i32) {
    let r = unsafe {
        futex(
            atomic,
            libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
            max_threads_to_wake,
            ptr::null(),
            ptr::null_mut(),
            0,
        )
    };
    debug_assert!((r >= 0 && r <= max_threads_to_wake as i64) || r == -1);
    if r == -1 {
        debug_assert_eq!(errno(), libc::EFAULT);
    }
}
