use core::cmp;
use core::sync::atomic::AtomicI32;
use core::time::Duration;

use crate::errno::errno;
use crate::futex::{Futex, WakeupReason};

impl Futex for AtomicI32 {
    #[inline]
    fn wait(&self, compare: i32, timeout: Option<Duration>) -> WakeupReason {
        let ptr = self as *const AtomicI32 as *const i32;
        let ts = convert_timeout_us(timeout);
        let r = unsafe { umtx_sleep(ptr, compare, ts) };
        match r {
            0 => WakeupReason::Unknown,
            -1 => match errno() {
                libc::EBUSY => WakeupReason::NoMatch,
                libc::EINTR => WakeupReason::Interrupt,
                libc::EWOULDBLOCK => WakeupReason::Unknown,
                e => {
                    debug_assert!(false, "Unexpected errno of umtx_sleep syscall: {}", e);
                    WakeupReason::Unknown
                }
            },
            r => {
                debug_assert!(
                    false,
                    "Unexpected return value of umtx_sleep syscall: {}",
                    r
                );
                WakeupReason::Unknown
            }
        }
    }

    #[inline]
    fn wake(&self) -> usize {
        let ptr = self as *const AtomicI32 as *const i32;
        let r = unsafe { umtx_wakeup(ptr, 0) };
        debug_assert!(
            r >= 0,
            "Unexpected return value of umtx_wakeup syscall: {}",
            r
        );
        cmp::max(r as usize, 0)
    }
}

extern "C" {
    fn umtx_sleep(
        uaddr: *const libc::c_int,
        val: libc::c_int,
        timeout: libc::c_int, // microseconds, 0 is indefinite
    ) -> libc::c_int;

    fn umtx_wakeup(
        uaddr: *const libc::c_int,
        count: libc::c_int, // 0 will wake up all
    ) -> libc::c_int;
}

// Timeout in microseconds, round nanosecond values up to microseconds.
fn convert_timeout_us(timeout: Option<Duration>) -> libc::c_int {
    match timeout {
        None => 0,
        Some(duration) => duration
            .as_secs()
            .checked_mul(1000_000)
            .and_then(|x| x.checked_add((duration.subsec_nanos() as u64 + 999) / 1000))
            .map(|ms| {
                if ms > libc::c_int::max_value() as u64 {
                    0
                } else {
                    ms as libc::c_int
                }
            })
            .unwrap_or(0),
    }
}
