use core::ptr;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use libc;

use crate::futex_like::{FutexLike, ThreadCount};

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) {
        let ptr = self as *const AtomicUsize as *mut usize;
        let ts = convert_timeout(timeout);
        let ts_ptr = ts.as_ref().map(|ts_ref| ts_ref as *const _ as *mut _).unwrap_or(ptr::null_mut());
        let r = unsafe { umtx_op(ptr, UMTX_OP_WAIT, compare, ptr::null_mut(), ts_ptr) };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
//            debug_assert!(errno() == libc::EINTR));
        }
    }

    fn futex_wake(&self, count: ThreadCount) {
        let ptr = self as *const AtomicUsize as *mut usize;
        let max_threads_to_wake = match count {
            ThreadCount::Some(n) => n as usize,
            ThreadCount::All => usize::max_value(),
        };
        let r = unsafe {
            umtx_op(
                ptr,
                UMTX_OP_WAKE,
                max_threads_to_wake,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
//            debug_assert!(errno() == libc::EINTR));
        }
    }
}


const _UMTX_OP: i32 = 454;
const UMTX_OP_WAIT: libc::c_int = 2;
const UMTX_OP_WAKE: libc::c_int = 3;

unsafe fn umtx_op(
    obj: *mut usize, // actually *mut libc::c_void
    op: libc::c_int,
    val: usize, // actually libc::c_ulong
    uaddr: *mut libc::c_void,
    uaddr2: *mut libc::c_void, // *mut timespec or *mut _umtx_time
) -> libc::c_int {
    libc::syscall(_UMTX_OP, obj, op, val, uaddr, uaddr2)
}

fn convert_timeout(timeout: Option<Duration>) -> Option<libc::timespec> {
    match timeout {
        Some(duration) => {
            if duration.as_secs() > libc::time_t::max_value() as u64 {
                return None;
            }
            Some(libc::timespec {
                tv_sec: duration.as_secs() as libc::time_t,
                tv_nsec: duration.subsec_nanos() as libc::c_long,
            })
        }
        None => None
    }
}
