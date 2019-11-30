use core::mem;
use core::sync::atomic::*;
use core::time::Duration;

use winapi::shared::minwindef::{DWORD, FALSE, TRUE};
use winapi::shared::winerror::ERROR_TIMEOUT;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::INFINITE;
use winapi::um::winnt::PVOID;

use crate::futex::{Futex, WakeupReason};
use crate::utils::AtomicAsMutPtr;
use crate::windows::{Backend, BACKEND};

macro_rules! imp_futex {
    ($atomic_type:ident, $int_type:ident) => {
        impl Futex for $atomic_type {
            type Integer = $int_type;

            fn wait(
                &self,
                mut compare: Self::Integer,
                timeout: Option<Duration>,
            ) -> Result<WakeupReason, ()> {
                if let Backend::Wait(f) = BACKEND.get() {
                    let address = self.as_mut_ptr() as PVOID;
                    let compare_address = &mut compare as *mut $int_type as PVOID;
                    let ms = convert_timeout_ms(timeout);
                    let r = (f.WaitOnAddress)(
                        address,
                        compare_address,
                        mem::size_of::<$int_type>(),
                        ms,
                    );
                    match r {
                        TRUE => Ok(WakeupReason::Unknown), // Can be any reason except TimedOut
                        FALSE | _ => match unsafe { GetLastError() } {
                            ERROR_TIMEOUT if ms != INFINITE => Ok(WakeupReason::TimedOut),
                            e => {
                                debug_assert!(
                                    false,
                                    "Unexpected error of WaitOnAddress call: {}",
                                    e
                                );
                                Ok(WakeupReason::Unknown)
                            }
                        },
                    }
                } else {
                    unreachable!();
                }
            }

            fn wake(&self) -> Result<usize, ()> {
                if let Backend::Wait(f) = BACKEND.get() {
                    let address = self.as_mut_ptr() as PVOID;
                    (f.WakeByAddressAll)(address);
                    Ok(0) // `WakeByAddressAll` does not return the number of woken threads
                } else {
                    unreachable!();
                }
            }
        }
    };
}
imp_futex!(AtomicUsize, usize);
imp_futex!(AtomicIsize, isize);
imp_futex!(AtomicU64, u64);
imp_futex!(AtomicI64, i64);
imp_futex!(AtomicU32, u32);
imp_futex!(AtomicI32, i32);
imp_futex!(AtomicU16, u16);
imp_futex!(AtomicI16, i16);
imp_futex!(AtomicU8, u8);
imp_futex!(AtomicI8, i8);

// Timeout in milliseconds, round nanosecond values up to milliseconds.
fn convert_timeout_ms(timeout: Option<Duration>) -> DWORD {
    match timeout {
        None => INFINITE,
        Some(duration) => duration
            .as_secs()
            .checked_mul(1000)
            .and_then(|x| x.checked_add((duration.subsec_nanos() as u64 + 999999) / 1000000))
            .map(|ms| {
                if ms > <DWORD>::max_value() as u64 {
                    INFINITE
                } else {
                    ms as DWORD
                }
            })
            .unwrap_or(INFINITE),
    }
}
