#![allow(non_camel_case_types)]

use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex::{Futex, WakeupReason};

// Fuchsia futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl Futex for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        let ptr = as_u32_pub(self) as *mut i32;
        let compare = (compare >> UNCOMPARED_BITS) as i32; // FIXME: is this correct?
        let deadline = convert_timeout(timeout);
        let r = unsafe { zx_futex_wait(ptr, compare, deadline) };
        match r {
            ZX_OK => WakeupReason::Unknown,
            ZX_ERR_BAD_STATE => WakeupReason::NoMatch,
            ZX_ERR_TIMED_OUT if deadline != ZX_TIME_INFINITE => WakeupReason::TimedOut,
            r => panic!("Undocumented return value {}.", r)
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        let ptr = as_u32_pub(self) as *mut i32;
        let wake_count = u32::max_value();
        let r = unsafe { zx_futex_wake(ptr, wake_count) };
        debug_assert!(r == ZX_OK);
        0 // FIXME: `zx_futex_wake` does not return the number of woken threads
    }
}

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
extern {
    fn zx_deadline_after(
        nanoseconds: zx_duration_t
        ) -> zx_time_t;

    fn zx_futex_wait(
        value_ptr: *mut zx_futex_t,
        current_value: zx_futex_t,
        deadline: zx_time_t
        ) -> zx_status_t;

    fn zx_futex_wake(
        value_ptr: *const zx_futex_t,
        count: u32
        ) -> zx_status_t;
}
