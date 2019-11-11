use core::mem;
use core::ptr;
use core::sync::atomic::AtomicUsize;
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex_like::{FutexLike, ThreadCount};

// Linux futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) {
        let ptr = as_u32_pub(self) as *mut i32;
        let compare = (compare >> UNCOMPARED_BITS) as u32 as i32;
        let r = unsafe {
            futex(
                ptr,
                libc::FUTEX_WAIT | libc::FUTEX_PRIVATE_FLAG,
                compare,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        debug_assert!(r == 0 || r == -1);
        if r == -1 {
            debug_assert!(
                errno() == libc::EINTR
                    || errno() == libc::EAGAIN
                    || (timeout.is_some() && errno() == libc::ETIMEDOUT)
            );
        }
    }

    fn futex_wake(&self, count: ThreadCount) {
        let ptr = as_u32_pub(self) as *mut i32;
        let max_threads_to_wake = match count {
            ThreadCount::One => 1,
            ThreadCount::All => i32::max_value(),
        };
        let r = unsafe {
            futex(
                ptr,
                libc::FUTEX_WAKE | libc::FUTEX_PRIVATE_FLAG,
                max_threads_to_wake as i32,
                ptr::null(),
                ptr::null_mut(),
                0,
            )
        };
        debug_assert!((r >= 0 && r <= max_threads_to_wake as i64) || r == -1);
        if r == -1 {
            debug_assert_eq!(errno(), libc::EFAULT);
        }
    }
}

fn errno() -> libc::c_int {
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location()
    }
    #[cfg(target_os = "android")]
    unsafe {
        *libc::__errno()
    }
}

unsafe fn futex(
    uaddr: *mut libc::c_int,
    futex_op: libc::c_int,
    val: libc::c_int,
    timeout: *const libc::timespec,
    uaddr2: *mut libc::c_void,
    val3: libc::c_int,
) -> libc::c_long {
    libc::syscall(libc::SYS_futex, uaddr, futex_op, val, timeout, uaddr2, val3)
}
