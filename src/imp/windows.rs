//! The `WaitOnAddress` / `WakeByAddress*` functions provide a convenient futex-like interface,
//! but they are only available since Windows 8. They are implemented in the `futex` module.
//!
//! On earlier Windows versions we fall back on the undocumented NT Keyed Events API. By using the
//! address of the atomic as the key to wait on, we can get something with looks a lot like a futex.
//!
//! There is an important difference:
//! Before the thread goes to sleep it does not check a comparison value. Instead the
//! `NtReleaseKeyedEvent` function blocks, waiting for a thread to wake if there is none. (Compared
//! to the Futex wake function which will immediately return.)
//!
//! With every release event one thread is waked. Thus we need to keep track of how many waiters
//! there are in order to wake them all, and to prevent the release function from hanging
//! indefinitely.
#![allow(non_snake_case)]

use core::cell::Cell;
use core::mem;
use core::ptr;
use core::sync::atomic::Ordering::{Acquire, Relaxed, Release};
use core::sync::atomic::{spin_loop_hint, AtomicI32, AtomicUsize};
use core::time::Duration;

use winapi::shared::basetsd::SIZE_T;
use winapi::shared::minwindef::{BOOL, DWORD, ULONG};
use winapi::shared::ntdef::{FALSE, NTSTATUS};
use winapi::shared::ntstatus::{STATUS_ALERTED, STATUS_SUCCESS, STATUS_TIMEOUT, STATUS_USER_APC};
use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
use winapi::um::winnt::{ACCESS_MASK, BOOLEAN, EVENT_ALL_ACCESS, HANDLE, LPCSTR, PHANDLE, PVOID};

use crate::futex::{self, WakeupReason};
use crate::RESERVED_MASK;

//
// Implementation of the Waiters trait
//
pub(crate) fn compare_and_wait(atomic: &AtomicUsize, compare: usize) {
    match BACKEND.get() {
        Backend::Wait(_) => futex::compare_and_wait(atomic, compare),
        Backend::Keyed(_) => {
            // Register the number of threads waiting. In theory we should be careful not to
            // overflow out of our counter bits. But it is impossible to have so many
            // threads waiting that it doesn't fit in 2^27 on 32-bit and 2^59 on 64-bit
            // (there would not be enough memory to hold their stacks).
            let mut current = atomic.load(Relaxed);
            loop {
                if current & !RESERVED_MASK != compare {
                    return;
                }
                match atomic.compare_exchange_weak(current, current + 1, Relaxed, Relaxed) {
                    Ok(_) => break,
                    Err(x) => current = x,
                }
            }
            // If a spurious wakeup happens right after a thread stores a new value in `atomic`
            // but before it can send the signal, we have no way to detect it is spurious.
            // If we then would not be waiting when the real signal is send, the sending thread
            // may hang indefinitely.
            // There is no way to prevent this race, but as an extra protection we check the return
            // value and repark when the wakeup is definitely not due to the event.
            let key = atomic as *const AtomicUsize as PVOID;
            loop {
                if let WakeupReason::Unknown = wait_for_keyed_event(key, None) {
                    break;
                }
            }
            debug_assert!(atomic.load(Relaxed) & !RESERVED_MASK != compare);
        }
        Backend::None => unreachable!(),
    }
}

pub(crate) fn store_and_wake(atomic: &AtomicUsize, new: usize) {
    match BACKEND.get() {
        Backend::Wait(_) => futex::store_and_wake(atomic, new),
        Backend::Keyed(_) => {
            let wake_count = atomic.swap(new, Release) & RESERVED_MASK;
            let key = atomic as *const AtomicUsize as PVOID;
            release_keyed_events(key, wake_count);
        }
        Backend::None => unreachable!(),
    }
}

//
// Implementation of the Parker trait
//
pub(crate) type Parker = AtomicI32;

const NOT_PARKED: i32 = 0x0;
const PARKED: i32 = 0x1;
const NOTIFIED: i32 = 0x2;

pub(crate) fn park(atomic: &AtomicI32, timeout: Option<Duration>) {
    match BACKEND.get() {
        Backend::Wait(_) => futex::park(atomic, timeout),
        Backend::Keyed(_) => {
            match atomic.compare_exchange(NOT_PARKED, PARKED, Release, Relaxed) {
                Ok(_) => {}
                Err(NOTIFIED) => {
                    atomic.store(NOT_PARKED, Relaxed);
                    return;
                }
                Err(_) => panic!(
                    "Tried to call park on an atomic while \
                     another thread is already parked on it"
                ),
            };
            let key = atomic as *const AtomicI32 as PVOID;
            loop {
                let r = wait_for_keyed_event(key, timeout);
                if timeout.is_some() {
                    // We don't guarantee there are no spurious wakeups when there was a timeout
                    // supplied.
                    atomic.store(NOT_PARKED, Relaxed);
                    break;
                }
                if let WakeupReason::Unknown = r {
                    // The wakeup was not caused by an alert ot timeout, we know (almost) for sure
                    // if the status is set to NOTIFIED. But this remains inherently racy, see
                    // the `compare_and_wait` implementation.
                    if atomic
                        .compare_exchange(NOTIFIED, NOT_PARKED, Relaxed, Relaxed)
                        .is_ok()
                    {
                        break;
                    }
                }
            }
        }
        Backend::None => unreachable!(),
    }
}

pub(crate) fn unpark(atomic: &AtomicI32) {
    match BACKEND.get() {
        Backend::Wait(_) => futex::unpark(atomic),
        Backend::Keyed(_) => {
            if atomic.swap(NOTIFIED, Release) == PARKED {
                let key = atomic as *const AtomicI32 as PVOID;
                release_keyed_events(key, 1);
            }
        }
        Backend::None => unreachable!(),
    }
}

fn wait_for_keyed_event(key: PVOID, timeout: Option<Duration>) -> WakeupReason {
    if let Backend::Keyed(f) = BACKEND.get() {
        let nt_timeout = convert_timeout_100ns(timeout);
        let timeout_ptr = nt_timeout
            .map(|t_ref| t_ref as *mut _)
            .unwrap_or(ptr::null_mut());
        let r = (f.NtWaitForKeyedEvent)(f.handle, key, FALSE, timeout_ptr);
        // `NtWaitForKeyedEvent` is an undocumented API where we don't known the possible
        // return values, but they are most likely similar to `NtWaitForSingleObject`.
        match r {
            STATUS_SUCCESS => WakeupReason::Unknown,
            STATUS_TIMEOUT if nt_timeout.is_some() => WakeupReason::TimedOut,
            STATUS_ALERTED | STATUS_USER_APC => WakeupReason::Interrupt,
            r => {
                debug_assert!(
                    false,
                    "Unexpected return value of NtWaitForKeyedEvent: {}",
                    r
                );
                WakeupReason::Unknown
            }
        }
    } else {
        unreachable!();
    }
}

fn release_keyed_events(key: PVOID, wake_count: usize) {
    if let Backend::Keyed(f) = BACKEND.get() {
        for _ in 0..wake_count {
            (f.NtReleaseKeyedEvent)(f.handle, key, 0, ptr::null_mut());
        }
    } else {
        unreachable!();
    }
}

// NT uses a timeout in units of 100ns, where positive values are absolute and negative values are
// relative.
fn convert_timeout_100ns(timeout: Option<Duration>) -> Option<LARGE_INTEGER> {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > i64::max_value() as u64 {
                return None;
            }
            // Checked operations that return `None` on overflow.
            // Round nanosecond values up to 100 ns.
            (duration.as_secs() as i64)
                .checked_mul(-10_000_000)
                .and_then(|x| x.checked_sub((duration.subsec_nanos() as i64 + 99) / 100))
        }
        None => None,
    }
}

// Backend states
const READY: usize = 0;
const INITIALIZING: usize = 1;
const EMPTY: usize = 2;

pub(crate) struct BackendStatic {
    status: AtomicUsize,
    backend: Cell<Backend>,
}
pub(crate) static BACKEND: BackendStatic = BackendStatic::new();

impl BackendStatic {
    const fn new() -> Self {
        BackendStatic {
            status: AtomicUsize::new(EMPTY),
            backend: Cell::new(Backend::None),
        }
    }

    pub(crate) fn get(&self) -> Backend {
        if self.status.load(Acquire) == READY {
            return self.backend.get();
        }
        self.init()
    }

    #[inline(never)]
    fn init(&self) -> Backend {
        let mut status = self.status.compare_and_swap(EMPTY, INITIALIZING, Acquire);
        if status == EMPTY {
            let backend = if let Some(res) = ProbeWaitAddress() {
                Backend::Wait(res)
            } else if let Some(res) = ProbeKeyedEvent() {
                Backend::Keyed(res)
            } else {
                panic!(
                    "failed to load both NT Keyed Events (WinXP+) and \
                     WaitOnAddress/WakeByAddress (Win8+)"
                );
            };
            self.backend.set(backend);
            self.status.store(READY, Release);
            return backend;
        }
        while status != READY {
            // The one place were we can't really do better than a spin loop is while another
            // thread is loading the functions that provide parking primitives ;-).
            spin_loop_hint();
            status = self.status.load(Acquire);
        }
        self.backend.get()
    }
}

unsafe impl Sync for BackendStatic {}

#[derive(Clone, Copy)]
pub(crate) enum Backend {
    None,
    Wait(WaitAddress),
    Keyed(KeyedEvent),
}

// LARGE_INTEGER in WinAPI is a struct instead of integer, and not ergonomic to use.
#[allow(non_camel_case_types)]
type LARGE_INTEGER = i64;
#[allow(non_camel_case_types)]
type PLARGE_INTEGER = *mut LARGE_INTEGER;

#[derive(Clone, Copy)]
pub(crate) struct WaitAddress {
    pub(crate) WaitOnAddress: extern "system" fn(
        Address: PVOID,
        CompareAddress: PVOID,
        AddressSize: SIZE_T,
        dwMilliseconds: DWORD,
    ) -> BOOL,
    pub(crate) WakeByAddressAll: extern "system" fn(Address: PVOID),
}

#[derive(Clone, Copy)]
pub(crate) struct KeyedEvent {
    handle: HANDLE, // The global keyed event handle.
    NtReleaseKeyedEvent: extern "system" fn(
        EventHandle: HANDLE,
        Key: PVOID,
        Alertable: BOOLEAN,
        Timeout: PLARGE_INTEGER,
    ) -> NTSTATUS,
    NtWaitForKeyedEvent: extern "system" fn(
        EventHandle: HANDLE,
        Key: PVOID,
        Alertable: BOOLEAN,
        Timeout: PLARGE_INTEGER,
    ) -> NTSTATUS,
}

#[cfg(not(feature = "fallback"))]
fn ProbeWaitAddress() -> Option<WaitAddress> {
    unsafe {
        // MSDN claims that that WaitOnAddress and WakeByAddressAll are
        // located in kernel32.dll, but they aren't...
        let synch_dll = GetModuleHandleA(b"api-ms-win-core-synch-l1-2-0.dll\0".as_ptr() as LPCSTR);
        if synch_dll.is_null() {
            return None;
        }

        let WaitOnAddress = GetProcAddress(synch_dll, b"WaitOnAddress\0".as_ptr() as LPCSTR);
        if WaitOnAddress.is_null() {
            return None;
        }
        let WakeByAddressAll = GetProcAddress(synch_dll, b"WakeByAddressAll\0".as_ptr() as LPCSTR);
        if WakeByAddressAll.is_null() {
            return None;
        }

        Some(WaitAddress {
            WaitOnAddress: mem::transmute(WaitOnAddress),
            WakeByAddressAll: mem::transmute(WakeByAddressAll),
        })
    }
}

#[cfg(feature = "fallback")]
fn ProbeWaitAddress() -> Option<WaitAddress> {
    None
}

fn ProbeKeyedEvent() -> Option<KeyedEvent> {
    unsafe {
        let ntdll = GetModuleHandleA(b"ntdll.dll\0".as_ptr() as LPCSTR);
        if ntdll.is_null() {
            return None;
        }

        let NtCreateKeyedEvent = GetProcAddress(ntdll, b"NtCreateKeyedEvent\0".as_ptr() as LPCSTR);
        if NtCreateKeyedEvent.is_null() {
            return None;
        }
        let NtWaitForKeyedEvent =
            GetProcAddress(ntdll, b"NtWaitForKeyedEvent\0".as_ptr() as LPCSTR);
        if NtWaitForKeyedEvent.is_null() {
            return None;
        }
        let NtReleaseKeyedEvent =
            GetProcAddress(ntdll, b"NtReleaseKeyedEvent\0".as_ptr() as LPCSTR);
        if NtReleaseKeyedEvent.is_null() {
            return None;
        }

        let NtCreateKeyedEvent: extern "system" fn(
            KeyedEventHandle: PHANDLE,
            DesiredAccess: ACCESS_MASK,
            ObjectAttributes: PVOID,
            Flags: ULONG,
        ) -> NTSTATUS = mem::transmute(NtCreateKeyedEvent);
        let mut handle: HANDLE = ptr::null_mut();
        let status = NtCreateKeyedEvent(&mut handle, EVENT_ALL_ACCESS, ptr::null_mut(), 0);
        if status != STATUS_SUCCESS {
            return None;
        }

        Some(KeyedEvent {
            handle: handle,
            NtReleaseKeyedEvent: mem::transmute(NtReleaseKeyedEvent),
            NtWaitForKeyedEvent: mem::transmute(NtWaitForKeyedEvent),
        })
    }
}
