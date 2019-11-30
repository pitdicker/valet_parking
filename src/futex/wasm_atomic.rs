//! Can currently (2019-11-20) be build using the following command:
//! ```
//! RUSTFLAGS='-C target-feature=+atomics,+bulk-memory' \
//! cargo build --target wasm32-unknown-unknown -Z build-std --release
//! ```

use core::arch::wasm32;
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
                let ptr = self.as_mut_ptr() as *mut i32;
                let timeout_ns = convert_timeout(timeout);
                let r = unsafe { wasm32::i32_atomic_wait(ptr, compare as i32, timeout_ns) };
                match r {
                    0 => Ok(WakeupReason::WokenUp),
                    1 => Ok(WakeupReason::NoMatch),
                    2 => Ok(WakeupReason::TimedOut),
                    _ => {
                        debug_assert!(false, "Unexpected return value of i32.atomic.wait: {}", r);
                        Ok(WakeupReason::Unknown)
                    }
                }
            }

            #[inline]
            fn wake(&self) -> Result<usize, ()> {
                let ptr = self.as_mut_ptr() as *mut i32;
                let r = unsafe { wasm32::atomic_notify(ptr, u32::max_value()) };
                Ok(r as usize)
            }
        }
    };
}
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);

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
