#![allow(non_snake_case)]

use core::cell::Cell;
use core::mem;
use core::ptr;
use core::sync::atomic::{spin_loop_hint, AtomicUsize, Ordering};
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

use crate::futex_like::{FutexLike, COUNTER_MASK, WakeupReason};

impl FutexLike for AtomicUsize {
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        match BACKEND.get() {
            Backend::Wait(f) => {
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
                            r => panic!("Undocumented return value {}.", r),
                        }
                    }
                }
            }
            Backend::Keyed(f) => {
                // Register the number of threads waiting. In theory we should be careful not to
                // overflow out of our counter bits. But it is impossible to have so many
                // threads waiting that it doesn't fit in 2^25 on 32-bit and 2^57 on 64-bit
                // (there would not be enough memory to hold their stacks).
                let mut current = self.load(Ordering::Relaxed);
                loop {
                    if current & !COUNTER_MASK != compare {
                        return WakeupReason::NoMatch;
                    }
                    let old = self.compare_and_swap(current, current + 1, Ordering::Relaxed);
                    if old == current {
                        break;
                    }
                    current = old;
                }
                // We need to provide some unique key to wait on. The least significant bit must be
                // zero, it is appearently used as a flag bit. The address of `self` seems like a
                // good candidate.
                let key = self as *const AtomicUsize as PVOID;
                let nt_timeout = convert_timeout_100ns(timeout);
                let timeout_ptr = nt_timeout
                    .map(|t_ref| t_ref as *mut _)
                    .unwrap_or(ptr::null_mut());
                let r = (f.NtWaitForKeyedEvent)(f.handle, key, BOOLEAN_FALSE, timeout_ptr);
                // `NtWaitForKeyedEvent` is an undocumented API where we don't known the possible
                // return values, but they are probably similar to `NtWaitForSingleObject`.
                match r {
                    STATUS_SUCCESS => WakeupReason::Unknown,
                    STATUS_TIMEOUT if nt_timeout.is_some() => WakeupReason::TimedOut,
                    STATUS_ALERTED |
                    STATUS_USER_APC => WakeupReason::Interrupt,
                    r => panic!("Undocumented return value {}.", r)
                }
            }
            Backend::None => unreachable!(),
        }
    }

    fn futex_wake(&self, new: usize) -> usize {
        let current = self.swap(new, Ordering::SeqCst);
        match BACKEND.get() {
            Backend::Wait(f) => {
                (f.WakeByAddressAll)(self as *const _ as PVOID);
                0 // `WakeByAddressAll` does not return the number of woken threads
            }
            Backend::Keyed(f) => {
                let wake_count = current & COUNTER_MASK;
                // Recreate the key; the address of self.
                let key = self as *const AtomicUsize as PVOID;
                // With every event we wake one thread. If we would try to wake a thread that is not
                // parked we will block indefinitely.
                for _ in 0..wake_count {
                    (f.NtReleaseKeyedEvent)(f.handle, key, 0, ptr::null_mut());
                };
                wake_count
            }
            Backend::None => unreachable!(),
        }
    }
}

// Backend states
const READY: usize = 0;
const INITIALIZING: usize = 1;
const EMPTY: usize = 2;

struct BackendStatic {
    status: AtomicUsize,
    backend: Cell<Backend>,
}
static BACKEND: BackendStatic = BackendStatic::new();

impl BackendStatic {
    const fn new() -> Self {
        BackendStatic {
            status: AtomicUsize::new(EMPTY),
            backend: Cell::new(Backend::None),
        }
    }

    fn get(&self) -> Backend {
        if self.status.load(Ordering::Acquire) == READY {
            return self.backend.get();
        }
        self.init()
    }

    #[inline(never)]
    fn init(&self) -> Backend {
        let mut status = self
            .status
            .compare_and_swap(EMPTY, INITIALIZING, Ordering::Acquire);
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
            self.status.store(READY, Ordering::Release);
            return backend;
        }
        while status != READY {
            // The one place were we can't really do better than a spin loop is while another
            // thread is loading the functions that provide parking primitives ;-).
            spin_loop_hint();
            status = self.status.load(Ordering::Acquire);
        }
        self.backend.get()
    }
}

unsafe impl Sync for BackendStatic {}

#[derive(Clone, Copy)]
enum Backend {
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
struct WaitAddress {
    WaitOnAddress: extern "system" fn(
        Address: PVOID,
        CompareAddress: PVOID,
        AddressSize: SIZE_T,
        dwMilliseconds: DWORD,
    ) -> BOOL,
    WakeByAddressAll: extern "system" fn(Address: PVOID),
}

#[derive(Clone, Copy)]
struct KeyedEvent {
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
        let status = NtCreateKeyedEvent(
            &mut handle,
            GENERIC_READ | GENERIC_WRITE,
            ptr::null_mut(),
            0,
        );
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

// `NtWaitForKeyedEvent` allows a thread to go to sleep, waiting on the event signaled by
// `NtReleaseKeyedEvent`. The major different between this API and the Futex API is that there is no
// comparison value that is checked as the thread goes to sleep. Instead the `NtReleaseKeyedEvent`
// function blocks, waiting for a thread to wake if there is none. (Compared to the Futex wake
// function which will immediately return.)
//
// Thus to use this API we need to keep track of how many waiters there are to prevent the release
// function from hanging.
//
// http://joeduffyblog.com/2006/11/28/windows-keyed-events-critical-sections-and-new-vista-synchronization-features/
// http://locklessinc.com/articles/keyed_events/

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
