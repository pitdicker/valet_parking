use core::ptr;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use libc;

use crate::futex_like::{FutexLike, ThreadCount};

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, _ts: Option<Duration>) {
        let ptr = self as *const AtomicUsize as *mut usize;
        let r = unsafe { umtx_op(ptr, UMTX_OP_WAIT, compare, ptr::null_mut(), ptr::null_mut()) };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
//            debug_assert!(errno() == libc::EINTR));
        }
    }

    fn futex_wake(&self, count: ThreadCount) {
        let ptr = self as *const AtomicUsize as *mut usize;
        let max_threads_to_wake = match count {
            ThreadCount::One => 1,
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
