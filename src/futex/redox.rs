use core::ptr;
use core::sync::atomic::AtomicI32;
use core::time::Duration;

use syscall::call;
use syscall::data::TimeSpec;
use syscall::error::{Error, EAGAIN, EINTR, ETIMEDOUT};
use syscall::flag::{FUTEX_WAIT, FUTEX_WAKE};

use crate::futex::{Futex, WakeupReason};
use crate::utils::AtomicAsMutPtr;

impl Futex for AtomicI32 {
    type Integer = i32;

    #[inline]
    fn wait(&self, compare: Self::Integer, timeout: Option<Duration>) -> WakeupReason {
        let ptr = self.as_mut_ptr() as *mut i32;
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
    fn wake(&self) -> usize {
        let ptr = self.as_mut_ptr() as *mut i32;
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
