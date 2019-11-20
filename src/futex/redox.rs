use core::mem;
use core::ptr;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex::{Futex, WakeupReason};

use syscall::call;
use syscall::data::TimeSpec;
use syscall::error::{Error, EAGAIN, EINTR, ETIMEDOUT};
use syscall::flag::{FUTEX_WAIT, FUTEX_WAKE};

// Redox futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl Futex for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        let ptr = as_u32_pub(self) as *mut i32;
        let compare = (compare >> UNCOMPARED_BITS) as u32 as i32;
        let ts = convert_timeout(timeout);
        let ts_ptr = ts
            .as_ref()
            .map(|ts_ref| ts_ref as *const _ as *mut _)
            .unwrap_or(ptr::null_mut());
        let r = unsafe { call::futex(ptr, FUTEX_WAIT, compare, 0, ts_ptr) };
        match r {
            Ok(r) => {
                debug_assert_eq!(r, 0);
                WakeupReason::Unknown
            }
            Err(Error { errno }) => match errno {
                EAGAIN => WakeupReason::NoMatch,
                EINTR => WakeupReason::Interrupt,
                ETIMEDOUT if ts.is_some() => WakeupReason::TimedOut,
                e => {
                    debug_assert!(false, "Unexpected error of futex syscall: {}", e);
                    WakeupReason::Unknown
                }
            },
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        let ptr = as_u32_pub(self) as *mut i32;
        let wake_count = i32::max_value();
        let r = unsafe { call::futex(ptr, FUTEX_WAKE, wake_count, 0, ptr::null_mut()) };
        match r {
            Ok(num_woken) => num_woken,
            Err(Error { errno }) => {
                debug_assert!(false, "Unexpected error of futex syscall: {}", errno);
                0
            }
        }
    }
}

fn convert_timeout(timeout: Option<Duration>) -> Option<TimeSpec> {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > i64::max_value() as u64 {
                return None;
            }
            Some(TimeSpec {
                tv_sec: duration.as_secs() as i64,
                tv_nsec: duration.subsec_nanos() as i32,
            })
        }
        None => None,
    }
}
