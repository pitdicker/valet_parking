use core::cmp;
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};
use crate::utils::{errno, AtomicAsMutPtr};

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
                    .map(|ts_ref| ts_ref as *const _)
                    .unwrap_or(ptr::null());
                let r = unsafe {
                    futex(
                        ptr,
                        libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                        expected as i32,
                        ts_ptr,
                        ptr::null_mut(),
                        0,
                    )
                };
                match r {
                    0 => Ok(WakeupReason::Unknown),
                    -1 => match errno() {
                        libc::EAGAIN => Ok(WakeupReason::NoMatch),
                        libc::EINTR => Ok(WakeupReason::Interrupt),
                        libc::ETIMEDOUT if ts.is_some() => Ok(WakeupReason::TimedOut),
                        e => {
                            debug_assert!(false, "Unexpected errno of futex syscall: {}", e);
                            Ok(WakeupReason::Unknown)
                        }
                    },
                    r => {
                        debug_assert!(false, "Unexpected return value of futex syscall: {}", r);
                        Ok(WakeupReason::Unknown)
                    }
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut i32;
                let wake_count = i32::max_value();
                let r = unsafe {
                    futex(
                        ptr,
                        libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                        wake_count,
                        ptr::null(),
                        ptr::null_mut(),
                        0,
                    )
                };
                debug_assert!(r >= 0, "Unexpected return value of futex syscall: {}", r);
                Ok(cmp::max(r as usize, 0))
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

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

// x32 Linux uses a non-standard type for tv_nsec in timespec.
// See https://sourceware.org/bugzilla/show_bug.cgi?id=16437
#[cfg(all(target_arch = "x86_64", target_pointer_width = "32"))]
#[allow(non_camel_case_types)]
type tv_nsec_t = i64;
#[cfg(not(all(target_arch = "x86_64", target_pointer_width = "32")))]
#[allow(non_camel_case_types)]
type tv_nsec_t = libc::c_long;

fn convert_timeout(timeout: Option<Duration>) -> Option<libc::timespec> {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > libc::time_t::max_value() as u64 {
                return None;
            }
            Some(libc::timespec {
                tv_sec: duration.as_secs() as libc::time_t,
                tv_nsec: duration.subsec_nanos() as tv_nsec_t,
            })
        }
        None => None,
    }
}
