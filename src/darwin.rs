use core::ptr;
use core::str;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::{futex, posix};

const TRUE: usize = 0;
const FALSE: usize = 1;
const UNINITIALIZED: usize = 2;

pub(crate) static HAS_ULOCK: AtomicUsize = AtomicUsize::new(UNINITIALIZED);

//
// Implementation of the Waiters trait
//
pub(crate) fn compare_and_wait(atomic: &AtomicUsize, compare: usize) {
    if has_ulock() {
        get_os_release();
        futex::compare_and_wait(atomic, compare)
    } else {
        posix::compare_and_wait(atomic, compare)
    }
}

pub(crate) unsafe fn store_and_wake(atomic: &AtomicUsize, new: usize) {
    if has_ulock() {
        futex::store_and_wake(atomic, new)
    } else {
        posix::store_and_wake(atomic, new)
    }
}

//
// Implementation of the Parker trait
//
pub(crate) fn park(atomic: &AtomicUsize, timeout: Option<Duration>) {
    if has_ulock() {
        futex::park(atomic, timeout)
    } else {
        posix::park(atomic, timeout)
    }
}

pub(crate) unsafe fn unpark(atomic: &AtomicUsize) {
    if has_ulock() {
        futex::unpark(atomic)
    } else {
        posix::unpark(atomic)
    }
}

fn has_ulock() -> bool {
    match HAS_ULOCK.load(Ordering::Relaxed) {
        TRUE => true,
        FALSE => false,
        UNINITIALIZED | _ => {
            let release = get_os_release();
            if release.0 >= 16 {
                HAS_ULOCK.store(TRUE, Ordering::Relaxed);
                true
            } else {
                HAS_ULOCK.store(FALSE, Ordering::Relaxed);
                false
            }
        }
    }
}

fn get_os_release() -> (u16, u16, u16) {
    let mut mib = [libc::CTL_KERN, libc::KERN_OSRELEASE];
    let mut buf = [0u8; 20];
    let mut len = buf.len();
    let ret = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as libc::c_uint,
            buf.as_mut_ptr() as *mut _,
            &mut len,
            ptr::null_mut(),
            0,
        )
    };
    if ret == -1 {
        panic!("kern.osrelease sysctl failed");
    }
    let mut len = 0;
    for c in buf.iter() {
        len += 1;
        if *c == 0 {
            break;
        }
    }
    let mut versions = [0u16; 3];
    let release = str::from_utf8(&buf[0..len]).unwrap();
    for (v, s) in versions.iter_mut().zip(release.split('.')) {
        *v = s.parse().unwrap_or(0);
    }
    (versions[0], versions[1], versions[2])
}
