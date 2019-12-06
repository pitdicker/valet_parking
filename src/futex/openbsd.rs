use core::cmp;
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

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
                let ptr = self.as_mut_ptr() as *mut u32;
                let ts = convert_timeout(timeout);
                let ts_ptr = ts
                    .as_ref()
                    .map(|ts_ref| ts_ref as *const libc::timespec)
                    .unwrap_or(ptr::null());
                let r = unsafe {
                    futex(
                        ptr,
                        FUTEX_WAIT | FUTEX_PRIVATE_FLAG,
                        expected as libc::c_int,
                        ts_ptr,
                        ptr::null_mut(),
                    )
                };
                match r {
                    0 => Ok(WakeupReason::Unknown),
                    libc::EAGAIN => Ok(WakeupReason::NoMatch),
                    libc::EINTR | libc::ECANCELED => Ok(WakeupReason::Interrupt),
                    libc::ETIMEDOUT if ts.is_some() => Ok(WakeupReason::TimedOut),
                    r => {
                        debug_assert!(false, "Unexpected return value of futex call: {}", r);
                        Ok(WakeupReason::Unknown)
                    }
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut u32;
                let wake_count = i32::max_value();
                let r = unsafe { futex(ptr, FUTEX_WAKE | FUTEX_PRIVATE_FLAG, wake_count, ptr::null(), ptr::null_mut()) };
                debug_assert!(r >= 0, "Unexpected return value of futex call: {}", r);
                Ok(cmp::max(r as usize, 0))
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

const FUTEX_WAIT: libc::c_int = 0;
const FUTEX_WAKE: libc::c_int = 1;
const FUTEX_PRIVATE_FLAG: libc::c_int = 128;

extern "C" {
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
