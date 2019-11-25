use core::cmp;
use core::mem;
use core::ptr;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::errno::errno;
use crate::futex::{Futex, WakeupReason};

// Linux futex takes an `i32` to compare if the thread should be parked.
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
            .map(|ts_ref| ts_ref as *const _)
            .unwrap_or(ptr::null());
        let r = unsafe {
            futex(
                ptr,
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                compare,
                ts_ptr,
                ptr::null_mut(),
                0,
            )
        };
        match r {
            0 => WakeupReason::Unknown,
            -1 => match errno() {
                libc::EAGAIN => WakeupReason::NoMatch,
                libc::EINTR => WakeupReason::Interrupt,
                libc::ETIMEDOUT if ts.is_some() => WakeupReason::TimedOut,
                e => {
                    debug_assert!(false, "Unexpected errno of futex_wait syscall: {}", e);
                    WakeupReason::Unknown
                }
            },
            r => {
                debug_assert!(
                    false,
                    "Unexpected return value of futex_wait syscall: {}",
                    r
                );
                WakeupReason::Unknown
            }
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        let ptr = as_u32_pub(self) as *mut i32;
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
        debug_assert!(
            r >= 0,
            "Unexpected return value of futex_wake syscall: {}",
            r
        );
        cmp::max(r as usize, 0)
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
