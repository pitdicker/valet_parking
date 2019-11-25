use core::mem;
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use crate::as_u32_pub;
use crate::futex_like::FutexLike;

use syscall::call;
use syscall::error::{Error, EAGAIN, EFAULT, EINTR, ETIMEDOUT};
use syscall::flag::{FUTEX_WAIT, FUTEX_WAKE};

// Redox futex takes an `i32` to compare if the thread should be parked.
// convert our reference to `AtomicUsize` to an `*const i32`, pointing to the part
// containing the non-reserved bits.
const UNCOMPARED_BITS: usize = 8 * (mem::size_of::<usize>() - mem::size_of::<u32>());

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, _timeout: Option<Duration>) {
        let ptr = as_u32_pub(self) as *mut i32;
        let compare = (compare >> UNCOMPARED_BITS) as u32 as i32;
        let r = unsafe { call::futex(ptr, FUTEX_WAIT, compare, 0, ptr::null_mut()) };
        match r {
            Ok(r) => debug_assert_eq!(r, 0),
            Err(Error { errno }) => {
                debug_assert!(errno == EINTR || errno == EAGAIN || errno == ETIMEDOUT);
            }
        }
    }

    fn futex_wake(&self, new: usize) {
        self.store(new, Ordering::SeqCst);
        let ptr = as_u32_pub(self) as *mut i32;
        let wake_count = i32::max_value();
        let r = unsafe { call::futex(ptr, FUTEX_WAKE, wake_count, 0, ptr::null_mut()) };
        match r {
            Ok(num_woken) => debug_assert!(num_woken <= wake_count as usize),
            Err(Error { errno }) => debug_assert_eq!(errno, EFAULT),
        }
    }
}
