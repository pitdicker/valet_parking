#![allow(non_upper_case_globals)]

use core::mem;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::as_u32_pub;
use crate::errno::errno;
use crate::futex_like::{FutexLike, WakeupReason};

const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        let ptr = as_u32_pub(self) as *mut _;
        let compare = (compare >> UNCOMPARED_BITS) as u64;
        let timeout_us = convert_timeout_us(timeout);
        let r = unsafe { ulock_wait(UL_COMPARE_AND_WAIT, ptr, compare, timeout_us) };
        if r >= 0 {
            // r is the number of threads waiting.
            WakeupReason::Unknown
        } else if r == -1 {
            match errno() {
                libc::EINTR => WakeupReason::Interrupt,
                libc::ETIMEDOUT if timeout_us != 0 => WakeupReason::TimedOut,
                e => panic!("Undocumented return value -1 with errno {}.", e)
            }
        } else {
            panic!("Undocumented return value {}.", r)
        }
    }


    fn futex_wake(&self, new: usize) -> usize {
        self.store(new, Ordering::SeqCst);
        let ptr = as_u32_pub(self) as *mut _;
        let r = unsafe { ulock_wake(UL_COMPARE_AND_WAIT | ULF_WAKE_ALL, ptr, 0) };
        if r == 0 || (r == -1 && errno() == libc::ENOENT) {
            // Apparently ENOENT means there were no threads waiting.
            // Libdispatch considers it a success, so lets do the same.
            return 0; // `ulock_wake` does not return the number of woken threads.
        }
        panic!("Undocumented return value {} with errno {}.", r, errno());
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
// Operation must be the same as that one used for `ulock_wait` (`UL_COMPARE_AND_WAIT`), combined
// with a flag: 0 to wake one thread, `ULF_WAKE_ALL` to wake all waiters.
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
