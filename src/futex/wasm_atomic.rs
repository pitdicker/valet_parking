//! Can currently (2019-11-20) be build using the following command:
//! ```
//! RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
//! cargo build --target wasm32-unknown-unknown -Z build-std --release
//! ```

use core::arch::wasm32;
use core::mem;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex::{Futex, WakeupReason};

const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl Futex for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        let ptr = as_u32_pub(self) as *mut i32;
        let compare = (compare >> UNCOMPARED_BITS) as i32;
        let timeout_ns = convert_timeout(timeout);
        let r = unsafe { wasm32::i32_atomic_wait(ptr, compare, timeout_ns) };
        match r {
            0 => WakeupReason::WokenUp,
            1 => WakeupReason::NoMatch,
            2 => WakeupReason::TimedOut,
            _ => {
                debug_assert!(false, "Unexpected return value of i32.atomic.wait: {}", r);
                WakeupReason::Unknown
            }
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        let ptr = as_u32_pub(self) as *mut i32;
        let r = unsafe { wasm32::atomic_notify(ptr, u32::max_value()) };
        r as usize
    }
}

fn convert_timeout(timeout: Option<Duration>) -> i64 {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > i64::max_value() as u64 {
                return -1;
            }
            (duration.as_secs() as i64)
                .checked_mul(1000_000_000)
                .and_then(|x| x.checked_add(duration.subsec_nanos() as i64))
                .unwrap_or(-1)
        }
        None => -1,
    }
}
