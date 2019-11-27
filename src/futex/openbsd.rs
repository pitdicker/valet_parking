use core::cmp;
use core::ptr;
use core::sync::atomic::AtomicI32;
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};
use crate::utils::AtomicAsMutPtr;

impl Futex for AtomicI32 {
    type Integer = i32;

    #[inline]
    fn wait(&self, compare: Self::Integer, timeout: Option<Duration>) -> WakeupReason {
        let ptr = self.as_mut_ptr() as *mut u32;
        let ts = convert_timeout(timeout);
        let ts_ptr = ts
            .as_ref()
            .map(|ts_ref| ts_ref as *const libc::timespec)
            .unwrap_or(ptr::null());
        let r = unsafe { futex(ptr, FUTEX_WAIT, compare, ts_ptr, ptr::null_mut()) };
        match r {
            0 => WakeupReason::Unknown,
            libc::EAGAIN => WakeupReason::NoMatch,
            libc::EINTR | libc::ECANCELED => WakeupReason::Interrupt,
            libc::ETIMEDOUT if ts.is_some() => WakeupReason::TimedOut,
            r => {
                debug_assert!(false, "Unexpected return value of futex call: {}", r);
                WakeupReason::Unknown
            }
        }
    }

    #[inline]
    fn wake(&self) -> usize {
        let ptr = self.as_mut_ptr() as *mut u32;
        let wake_count = i32::max_value();
        let r = unsafe { futex(ptr, FUTEX_WAKE, wake_count, ptr::null(), ptr::null_mut()) };
        debug_assert!(r >= 0, "Unexpected return value of futex call: {}", r);
        cmp::max(r as usize, 0)
    }
}

const FUTEX_WAIT: libc::c_int = 0;
const FUTEX_WAKE: libc::c_int = 1;

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
