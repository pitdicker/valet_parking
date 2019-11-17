use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex_like::FutexLike;

// OpenBSD futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) {
        let ptr = as_u32_pub(self) as *mut u32;
        let compare = (compare >> UNCOMPARED_BITS) as libc::c_int;
        let ts = convert_timeout(timeout);
        let ts_ptr = ts
            .as_ref()
            .map(|ts_ref| ts_ref as *const _)
            .unwrap_or(ptr::null());
        let r = unsafe {
            futex(
                ptr,
                FUTEX_WAIT,
                compare,
                ts_ptr,
                ptr::null_mut(),
            )
        };
        debug_assert!(r == 0 || r == -1);
/*
        if r == -1 {
            debug_assert!(
                errno() == libc::EINTR
                    || errno() == libc::EAGAIN
                    || (timeout.is_some() && errno() == libc::ETIMEDOUT)
            );
        }
*/
    }

    fn futex_wake(&self, new: usize) {
        self.store(new, Ordering::SeqCst);
        let ptr = as_u32_pub(self) as *mut u32;
        let wake_count = i32::max_value();
        let r = unsafe {
            futex(
                ptr,
                FUTEX_WAKE,
                wake_count,
                ptr::null(),
                ptr::null_mut(),
            )
        };
        debug_assert!((r >= 0 && r <= wake_count as libc::c_int) || r == -1);
/*
        if r == -1 {
            debug_assert_eq!(errno(), libc::EFAULT);
        }
*/
    }
}

const FUTEX_WAIT: libc::c_int = 0;
const FUTEX_WAKE: libc::c_int = 1;

extern {
    fn futex(
        uaddr: *mut u32,
        futex_op: libc::c_int,
        val: libc::c_int,
        timeout: *const libc::timespec,
        uaddr2: *mut u32,
        ) -> libc::c_int;
}

fn convert_timeout(timeout: Option<Duration>) -> Option<libc::timespec> {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > libc::time_t::max_value() as u64 {
                return None;
            }
            Some(libc::timespec {
                tv_sec: duration.as_secs() as libc::time_t,
                tv_nsec: duration.subsec_nanos() as libc::c_long,
            })
        }
        None => None,
    }
}
