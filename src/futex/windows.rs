#![allow(non_snake_case)]

use core::cell::Cell;
use core::mem;
use core::ptr;
use core::sync::atomic::{spin_loop_hint, AtomicUsize};
use core::time::Duration;

use winapi::shared::basetsd::SIZE_T;
use winapi::shared::minwindef::{BOOL, DWORD, TRUE as BOOL_TRUE, FALSE as BOOL_FALSE, ULONG};
use winapi::shared::winerror::ERROR_TIMEOUT;
use winapi::shared::ntdef::{FALSE as BOOLEAN_FALSE, NTSTATUS};
use winapi::shared::ntstatus::{STATUS_ALERTED, STATUS_SUCCESS, STATUS_TIMEOUT, STATUS_USER_APC};
use winapi::um::errhandlingapi::GetLastError;
use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
use winapi::um::winbase::INFINITE;
use winapi::um::winnt::{
    ACCESS_MASK, BOOLEAN, GENERIC_READ, GENERIC_WRITE, HANDLE, LPCSTR, PHANDLE, PVOID,
};

use crate::futex::{Futex, COUNTER_MASK, WakeupReason};

impl Futex for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        if let Wait(f) = BACKEND.get() {
            let address = self as *const _ as PVOID;
            let compare_address = &compare as *const _ as PVOID;
            let ms = convert_timeout_ms(timeout);
            let r =
                (f.WaitOnAddress)(address, compare_address, mem::size_of::<AtomicUsize>(), ms);
            match r {
                BOOL_TRUE => WakeupReason::Unknown, // Can be any reason except TimedOut
                BOOL_FALSE | _ => {
                    match unsafe { GetLastError() } {
                        ERROR_TIMEOUT if ms != INFINITE => WakeupReason::TimedOut,
                        e => {
                            debug_assert!(false, "Unexpected error of WaitOnAddress call: {}", e);
                            WakeupReason::Unknown
                        }
                    }
                }
            }
        } else {
            unreachable!();
        }
    }

    #[inline]
    fn futex_wake(&self) -> usize {
        if let Wait(f) = BACKEND.get() {
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
