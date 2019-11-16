#![allow(non_upper_case_globals)]

use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex_like::{FutexLike, ThreadCount};

/*
#[link(name = "libsystem_kernel")]
extern {
    // Only 32 bits of `addr` and `value` are used for comparison.
    // `timeout` is specified in microseconds, with 0 for infinite.
    fn __ulock_wait(operation: u32, addr: *mut libc::c_void, value: u64, timeout: u32) -> libc::c_int;
    fn __ulock_wake(operation: u32, addr: *mut libc::c_void, wake_value: u64) -> libc::c_int;
}
*/

const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) {
        let ptr = as_u32_pub(self) as *mut _;
        let compare = (compare >> UNCOMPARED_BITS) as u64;
        let timeout_us = convert_timeout_us(timeout);
        let r = unsafe { ulock_wait(UL_COMPARE_AND_WAIT, ptr, compare, timeout_us) };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
//            debug_assert!(
//                errno() == libc::EINTR
//                    || errno() == libc::EAGAIN
//                    || (timeout_us != 0 && errno() == libc::ETIMEDOUT)
//            );
        }
    }

    fn futex_wake(&self, _count: ThreadCount) {
        let ptr = as_u32_pub(self) as *mut _;
        // WARNING: we always wake all threads.
        // To wake only one, we have to use `ULF_WAKE_THREAD` and specify a thread name.
        let _r = unsafe { ulock_wake(UL_COMPARE_AND_WAIT | ULF_WAKE_ALL, ptr, 0) };
    }
}

const UL_COMPARE_AND_WAIT: u32 = 1;
const ULF_WAKE_ALL: u32 = 0x100;
const SYS_ulock_wait: libc::c_int = 515;
const SYS_ulock_wake: libc::c_int = 516;

// Only 32 bits of `addr` and `value` are used for comparison.
// `timeout` is specified in microseconds, with 0 for infinite.
unsafe fn ulock_wait(operation: u32, addr: *mut libc::c_void, value: u64, timeout: u32) -> libc::c_int {
    libc::syscall(SYS_ulock_wait, operation, addr, value, timeout)
}

// Wake_value is used to specify the thread to wake, used in combination with `ULF_WAKE_THREAD`.
unsafe fn ulock_wake(operation: u32, addr: *mut libc::c_void, wake_value: u64) -> libc::c_int {
    libc::syscall(SYS_ulock_wake, operation, addr, wake_value)
}

// Timeout in microseconds, round nanosecond values up to microseconds.
fn convert_timeout_us(timeout: Option<Duration>) -> u32 {
    match timeout {
        None => 0,
        Some(duration) => duration
            .as_secs()
            .checked_mul(1000_000)
            .and_then(|x| x.checked_add((duration.subsec_nanos() as u64 + 999) / 1000))
            .map(|ms| {
                if ms > u32::max_value() as u64 {
                    0
                } else {
                    ms as u32
                }
            })
            .unwrap_or(0),
    }
}
