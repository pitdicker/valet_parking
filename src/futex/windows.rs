use core::mem;
use core::sync::atomic::AtomicI32;
use core::time::Duration;

use winapi::shared::minwindef::{DWORD, FALSE, TRUE};
use winapi::shared::winerror::ERROR_TIMEOUT;
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::winbase::INFINITE;
use winapi::um::winnt::PVOID;

use crate::futex::{Futex, WakeupReason};
use crate::windows::{Backend, BACKEND};

impl Futex for AtomicI32 {
    fn wait(&self, compare: i32, timeout: Option<Duration>) -> WakeupReason {
        if let Backend::Wait(f) = BACKEND.get() {
            let address = self as *const _ as PVOID;
            let compare_address = &compare as *const _ as PVOID;
            let ms = convert_timeout_ms(timeout);
            let r = (f.WaitOnAddress)(address, compare_address, mem::size_of::<AtomicI32>(), ms);
            match r {
                TRUE => WakeupReason::Unknown, // Can be any reason except TimedOut
                FALSE | _ => match unsafe { GetLastError() } {
                    ERROR_TIMEOUT if ms != INFINITE => WakeupReason::TimedOut,
                    e => {
                        debug_assert!(false, "Unexpected error of WaitOnAddress call: {}", e);
                        WakeupReason::Unknown
                    }
                },
            }
        } else {
            unreachable!();
        }
    }

    fn wake(&self) -> usize {
        if let Backend::Wait(f) = BACKEND.get() {
            (f.WakeByAddressAll)(self as *const _ as PVOID);
            0 // `WakeByAddressAll` does not return the number of woken threads
        } else {
            unreachable!();
        }
    }
}

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
