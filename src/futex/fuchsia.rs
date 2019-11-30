#![allow(non_camel_case_types)]

use core::sync::atomic::{AtomicI32, AtomicU32};
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};
use crate::utils::AtomicAsMutPtr;

macro_rules! imp_futex {
    ($atomic_type:ident, $int_type:ident) => {
        impl Futex for $atomic_type {
            type Integer = $int_type;

            #[inline]
            fn wait(
                &self,
                compare: Self::Integer,
                timeout: Option<Duration>,
            ) -> Result<WakeupReason, ()> {
                let ptr = self.as_mut_ptr() as *mut zx_futex_t;
                let deadline = convert_timeout(timeout);
                let r = unsafe { zx_futex_wait(ptr, compare as zx_futex_t, deadline) };
                match r {
                    ZX_OK => Ok(WakeupReason::Unknown),
                    ZX_ERR_BAD_STATE => Ok(WakeupReason::NoMatch),
                    ZX_ERR_TIMED_OUT if deadline != ZX_TIME_INFINITE => Ok(WakeupReason::TimedOut),
                    r => {
                        debug_assert!(false, "Unexpected return value of zx_futex_wait: {}", r);
                        Ok(WakeupReason::Unknown)
                    }
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut i32;
                let wake_count = u32::max_value();
                let r = unsafe { zx_futex_wake(ptr, wake_count) };
                debug_assert!(
                    r == ZX_OK,
                    "Unexpected return value of zx_futex_wake: {}",
                    r
                );
                Ok(0) // `zx_futex_wake` does not return the number of woken threads
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

fn convert_timeout(timeout: Option<Duration>) -> zx_time_t {
    match timeout {
        Some(duration) => {
            if duration.as_nanos() > zx_duration_t::max_value() as u128 {
                ZX_TIME_INFINITE
            } else {
                unsafe { zx_deadline_after(duration.as_nanos() as zx_duration_t) }
            }
        }
        None => ZX_TIME_INFINITE,
    }
}

// It would be better if we could depend on the `fuchsia-zircon-sys` crate.
// But it contains a bug in its signature of `zx_futex_wait`, and the repository seems gone.
type zx_futex_t = i32;
type zx_status_t = i32;
type zx_duration_t = u64;
type zx_time_t = u64;

const ZX_OK: zx_status_t = 0;
const ZX_ERR_BAD_STATE: zx_status_t = -20;
const ZX_ERR_TIMED_OUT: zx_status_t = -21;
const ZX_TIME_INFINITE: zx_time_t = u64::max_value();

#[link(name = "zircon")]
extern "C" {
    fn zx_deadline_after(nanoseconds: zx_duration_t) -> zx_time_t;

    fn zx_futex_wait(
        value_ptr: *mut zx_futex_t,
        current_value: zx_futex_t,
        deadline: zx_time_t,
    ) -> zx_status_t;

    fn zx_futex_wake(value_ptr: *const zx_futex_t, count: u32) -> zx_status_t;
}
