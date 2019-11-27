//! Can currently (2019-11-20) be build using the following command:
//! ```
//! RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
//! cargo build --target wasm32-unknown-unknown -Z build-std --release
//! ```

use core::arch::wasm32;
use core::sync::atomic::AtomicI32;
use core::time::Duration;

use crate::futex::{Futex, WakeupReason};

impl Futex for AtomicI32 {
    type Integer = i32;

    #[inline]
    fn wait(&self, compare: Self::Integer, timeout: Option<Duration>) -> WakeupReason {
        let ptr = self as *const AtomicI32 as *mut i32;
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
    fn wake(&self) -> usize {
        let ptr = self as *const AtomicI32 as *mut i32;
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
