use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

use syscall::call;
use syscall::data::TimeSpec;
use syscall::error::{Error, EAGAIN, EINTR, ETIMEDOUT};
use syscall::flag::{FUTEX_WAIT, FUTEX_WAKE};

use crate::futex::{Futex, WakeupReason};
use crate::utils::AtomicAsMutPtr;

macro_rules! imp_futex {
    ($atomic_type:ident, $int_type:ident) => {
        impl Futex for $atomic_type {
            type Integer = $int_type;

            #[inline]
            fn wait(
                &self,
                expected: Self::Integer,
                timeout: Option<Duration>,
            ) -> Result<WakeupReason, ()> {
                let ptr = self.as_mut_ptr() as *mut i32;
                let ts = convert_timeout(timeout);
                let ts_ptr = ts
                    .as_ref()
                    .map(|ts_ref| ts_ref as *const TimeSpec)
                    .unwrap_or(ptr::null());
                let r = unsafe {
                    call::futex(
                        ptr,
                        FUTEX_WAIT,
                        expected as i32,
                        ts_ptr as usize,
                        ptr::null_mut(),
                    )
                };
                match r {
                    Ok(r) => {
                        debug_assert_eq!(r, 0);
                        Ok(WakeupReason::Unknown)
                    }
                    Err(Error { errno }) => match errno {
                        EAGAIN => Ok(WakeupReason::NoMatch),
                        EINTR => Ok(WakeupReason::Interrupt),
                        ETIMEDOUT if ts.is_some() => Ok(WakeupReason::TimedOut),
                        e => {
                            debug_assert!(false, "Unexpected error of futex syscall: {}", e);
                            Ok(WakeupReason::Unknown)
                        }
                    },
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut i32;
                let wake_count = i32::max_value();
                let r = unsafe { call::futex(ptr, FUTEX_WAKE, wake_count, 0, ptr::null_mut()) };
                match r {
                    Ok(num_woken) => Ok(num_woken),
                    Err(Error { errno }) => {
                        debug_assert!(false, "Unexpected error of futex syscall: {}", errno);
                        Ok(0)
                    }
                }
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

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
