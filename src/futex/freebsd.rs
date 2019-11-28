use core::cmp;
use core::ptr;
use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};
use crate::utils::{errno, AtomicAsMutPtr};

// FreeBSD can take and compare an `usize` value when used with the `UMTX_OP_WAIT` and
// `UMTX_OP_WAKE` operations. But we want to be good citizens and use `UMTX_OP_WAIT_UINT_PRIVATE`
// and `UMTX_OP_WAKE_PRIVATE`, which allow the kernel to maintain a process-private queue of waiting
// threads. This has the nice side effect that it takes a operates on an i32 instead, which makes it
// the similar to futex implementations on other platforms.

macro_rules! imp_futex {
    ($atomic_type:ident, $int_type:ident) => {
        impl Futex for $atomic_type {
            type Integer = $int_type;

            #[inline]
            fn wait(&self, compare: Self::Integer, timeout: Option<Duration>) -> WakeupReason {
                let ptr = self.as_mut_ptr() as *mut libc::c_void;
                let mut ts = convert_timeout(timeout);
                let ts_ptr = ts
                    .as_mut()
                    .map(|ts_ref| ts_ref as *mut libc::timespec as *mut libc::c_void)
                    .unwrap_or(ptr::null_mut());
                let r = unsafe {
                    umtx_op(
                        ptr,
                        UMTX_OP_WAIT_UINT_PRIVATE,
                        compare as i32 as libc::c_long,
                        ptr::null_mut(),
                        ts_ptr,
                    )
                };
                match r {
                    0 => WakeupReason::Unknown, // Can be NoMatch, WokenUp and Spurious
                    -1 => match errno() {
                        libc::EINTR => WakeupReason::Interrupt,
                        libc::ETIMEDOUT if ts.is_some() => WakeupReason::TimedOut,
                        e => {
                            debug_assert!(false, "Unexpected errno of umtx_op syscall: {}", e);
                            WakeupReason::Unknown
                        }
                    },
                    r => {
                        debug_assert!(false, "Unexpected return value of umtx_op syscall: {}", r);
                        WakeupReason::Unknown
                    }
                }
            }

            #[inline]
            fn wake(&self) -> usize {
                let ptr = self.as_mut_ptr() as *mut libc::c_void;
                let wake_count = libc::INT_MAX as libc::c_long;
                let r = unsafe {
                    umtx_op(
                        ptr,
                        UMTX_OP_WAKE_PRIVATE,
                        wake_count,
                        ptr::null_mut(),
                        ptr::null_mut(),
                    )
                };
                debug_assert!(r >= 0, "Unexpected return value of umtx_op syscall: {}", r);
                cmp::max(r as usize, 0)
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

const _UMTX_OP: i32 = 454;
const UMTX_OP_WAIT_UINT_PRIVATE: libc::c_int = 15;
const UMTX_OP_WAKE_PRIVATE: libc::c_int = 16;

unsafe fn umtx_op(
    obj: *mut libc::c_void,
    op: libc::c_int,
    val: libc::c_long,
    uaddr: *mut libc::c_void,
    uaddr2: *mut libc::c_void, // *mut timespec or *mut _umtx_time
) -> libc::c_int {
    libc::syscall(_UMTX_OP, obj, op, val, uaddr, uaddr2)
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
