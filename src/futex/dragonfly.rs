use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use libc;

use crate::as_u32_pub;
use crate::errno::errno;
use crate::futex::{Futex, WakeupReason};

const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl Futex for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) {
        let ptr = as_u32_pub(self) as *mut _;
        let compare = (compare >> UNCOMPARED_BITS) as libc::c_int;
        let ts = convert_timeout_us(timeout);
        let r = unsafe {
            umtx_sleep(
                ptr,
                compare,
                ts,
            )
        };
        match r {
            0 => WakeupReason::Unknown,
            -1 => {
                match errno() {
                    libc::EBUSY => WakeupReason::NoMatch,
                    libc::EINTR => WakeupReason::Interrupt,
                    libc::EWOULDBLOCK => WakeupReason::Unknown,
                    e => panic!("Undocumented return value -1 with errno {}.", e)
                }
            }
            r => panic!("Undocumented return value {}.", r)
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        let ptr = as_u32_pub(self) as *mut _;
        let r = unsafe { umtx_wakeup(ptr, 0) };
        assert!(r >= 0);
        r as usize
    }
}

extern {
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
