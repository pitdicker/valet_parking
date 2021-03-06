//! Use the undocumented `ulock_wait` and `ulock_wake` syscalls that are available since
//! MacOS 10.12 Sierra (Darwin 16.0).
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
                let ptr = self.as_mut_ptr() as *mut libc::c_void;
                let expected = expected as u32 as u64;
                let timeout_us = convert_timeout_us(timeout);
                let r = unsafe { ulock_wait(UL_COMPARE_AND_WAIT, ptr, expected, timeout_us) };
                if r >= 0 {
                    // r is the number of threads waiting.
                    Ok(WakeupReason::Unknown)
                } else if r == -1 {
                    match errno() {
                        libc::EINTR => Ok(WakeupReason::Interrupt),
                        libc::ETIMEDOUT if timeout_us != 0 => Ok(WakeupReason::TimedOut),
                        e => {
                            debug_assert!(false, "Unexpected errno of ulock_wait syscall: {}", e);
                            Ok(WakeupReason::Unknown)
                        }
                    }
                } else {
                    debug_assert!(
                        false,
                        "Unexpected return value of ulock_wait syscall: {}",
                        r
                    );
                    Ok(WakeupReason::Unknown)
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut libc::c_void;
                let r = unsafe { ulock_wake(UL_COMPARE_AND_WAIT | ULF_WAKE_ALL, ptr, 0) };
                // Apparently the return value -1 with ENOENT means there were no threads waiting.
                // Libdispatch considers it a success, so lets do the same.
                if !(r == 0 || (r == -1 && errno() == libc::ENOENT)) {
                    debug_assert!(
                        r >= 0,
                        "Unexpected return value of ulock_wake syscall: {}; errno: {}",
                        r,
                        errno()
                    );
                }
                Ok(0) // `ulock_wake` does not return the number of woken threads.
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

const UL_COMPARE_AND_WAIT: u32 = 1;
const ULF_WAKE_ALL: u32 = 0x100;
#[allow(non_upper_case_globals)]
const SYS_ulock_wait: libc::c_int = 515;
#[allow(non_upper_case_globals)]
const SYS_ulock_wake: libc::c_int = 516;

// Only 32 bits of `addr` and `value` are used for comparison.
// `timeout` is specified in microseconds, with 0 for infinite.
unsafe fn ulock_wait(
    operation: u32,
    addr: *mut libc::c_void,
    value: u64,
    timeout: u32,
) -> libc::c_int {
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
