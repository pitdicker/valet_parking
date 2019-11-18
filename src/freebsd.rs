use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use libc;

use crate::as_u32_pub;
use crate::errno::errno;
use crate::futex_like::{FutexLike, WakeupReason};

// FreeBSD can take and compare an `usize` value when used with the `UMTX_OP_WAIT` and
// `UMTX_OP_WAKE` operations. But we want to be good citizens and use `UMTX_OP_WAIT_UINT_PRIVATE`
// and `UMTX_OP_WAKE_PRIVATE`, which allow the kernel to maintain a process-private queue of waiting
// threads. So we are going to use the same trick as for Linux futexes: pass a pointer to the
// 32 high-order bits.
// The compare value is still an usize, but the kernel seems to only compare the high-order part.
// In the same way the number of threads to wake is tricky: the value is an usize, but is does not
// accept values outside the i32 range.

impl FutexLike for AtomicUsize {
    #[inline]
    fn futex_wait(&self, compare: usize, timeout: Option<Duration>) -> WakeupReason {
        let ptr = as_u32_pub(self) as *mut _;
        let ts = convert_timeout(timeout);
        let ts_ptr = ts
            .as_ref()
            .map(|ts_ref| ts_ref as *const _ as *mut _)
            .unwrap_or(ptr::null_mut());
        let r = unsafe {
            umtx_op(
                ptr,
                UMTX_OP_WAIT_UINT_PRIVATE,
                compare as libc::c_long,
                ptr::null_mut(),
                ts_ptr,
            )
        };
        match r {
            0 => WakeupReason::Unknown, // Can be NoMatch, WokenUp and Spurious
            -1 => {
                match errno() {
                    libc::EINTR => WakeupReason::Interrupt,
                    libc::ETIMEDOUT if ts.is_some() => WakeupReason::TimedOut,
                    e => panic!("Undocumented return value -1 with errno {}.", e)
                }
            }
            r => panic!("Undocumented return value {}.", r)
        }
    }

    fn futex_wake(&self, new: usize) -> usize {
        self.store(new, Ordering::SeqCst);
        let ptr = as_u32_pub(self) as *mut _;
        let wake_count = libc::INT_MAX as libc::c_long;
        let r = unsafe {
            umtx_op(
                ptr,
                UMTX_OP_WAKE_PRIVATE,
                wake_count,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        assert!(r >= 0);
        r as usize
    }
}

const _UMTX_OP: i32 = 454;
const UMTX_OP_WAIT_UINT_PRIVATE: libc::c_int = 15;
const UMTX_OP_WAKE_PRIVATE: libc::c_int = 16;

unsafe fn umtx_op(
    obj: *mut libc::c_void,
    op: libc::c_int,
    val: libc::c_long,
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
        None => None,
    }
}
