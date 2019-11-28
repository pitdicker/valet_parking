use core::cmp;
use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};
use crate::utils::{errno, AtomicAsMutPtr};

macro_rules! imp_futex {
    ($atomic_type:ident, $int_type:ident) => {
        impl Futex for $atomic_type {
            type Integer = $int_type;

            #[inline]
            fn wait(&self, compare: Self::Integer, timeout: Option<Duration>) -> WakeupReason {
                let ptr = self.as_mut_ptr() as *mut libc::c_int;
                let ts = convert_timeout_us(timeout);
                let r = unsafe { umtx_sleep(ptr, compare as libc::c_int, ts) };
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
                let ptr = self.as_mut_ptr() as *mut libc::c_int;
                let r = unsafe { umtx_wakeup(ptr, 0) };
                debug_assert!(
                    r >= 0,
                    "Unexpected return value of umtx_wakeup syscall: {}",
                    r
                );
                cmp::max(r as usize, 0)
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

extern "C" {
    // Note: our function signature does not match the one from the man page, which says that `ptr`
    // can be `*const`. Yet at the same time it says:
    // "WARNING! In order to properly interlock against fork(), this function will do an atomic
    // read-modify-write on the underlying memory by atomically adding the value 0 to it."
    // So let's make the functions take a `*mut` pointer like on all other operating systems.
    fn umtx_sleep(
        ptr: *mut libc::c_int,
        value: libc::c_int,
        timeout: libc::c_int, // microseconds, 0 is indefinite
    ) -> libc::c_int;

    fn umtx_wakeup(
        ptr: *mut libc::c_int,
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
