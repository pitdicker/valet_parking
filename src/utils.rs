#![allow(unused)]
use core::cell::UnsafeCell;
use core::sync::atomic::*;

// Copied from `libstd/sys/unix/os.rs`.
#[cfg(unix)]
extern "C" {
    #[cfg(not(target_os = "dragonfly"))]
    #[cfg_attr(
        any(
            target_os = "linux",
            target_os = "emscripten",
            target_os = "fuchsia",
            target_os = "l4re"
        ),
        link_name = "__errno_location"
    )]
    #[cfg_attr(
        any(
            target_os = "netbsd",
            target_os = "openbsd",
            target_os = "android",
            target_os = "redox",
            target_env = "newlib"
        ),
        link_name = "__errno"
    )]
    #[cfg_attr(target_os = "solaris", link_name = "___errno")]
    #[cfg_attr(
        any(target_os = "macos", target_os = "ios", target_os = "freebsd"),
        link_name = "__error"
    )]
    #[cfg_attr(target_os = "haiku", link_name = "_errnop")]
    fn errno_location() -> *mut libc::c_int;
}

#[cfg(all(unix, not(target_os = "dragonfly")))]
pub(crate) fn errno() -> i32 {
    unsafe { (*errno_location()) as i32 }
}

#[cfg(target_os = "dragonfly")]
pub(crate) fn errno() -> i32 {
    extern "C" {
        #[thread_local]
        static errno: libc::c_int;
    }

    unsafe { errno as i32 }
}

pub(crate) trait AtomicAsMutPtr {
    type Integer;

    fn as_mut_ptr(&self) -> *mut Self::Integer;
}

macro_rules! imp_as_mut_ptr {
    ($atomic_type:ident, $int_type:ident) => {
        impl AtomicAsMutPtr for $atomic_type {
            type Integer = $int_type;

            fn as_mut_ptr(&self) -> *mut Self::Integer {
                unsafe { (&*(self as *const $atomic_type as *const UnsafeCell<$int_type>)).get() }
            }
        }
    };
}
imp_as_mut_ptr!(AtomicUsize, usize);
imp_as_mut_ptr!(AtomicIsize, isize);
imp_as_mut_ptr!(AtomicU64, u64);
imp_as_mut_ptr!(AtomicI64, i64);
imp_as_mut_ptr!(AtomicU32, u32);
imp_as_mut_ptr!(AtomicI32, i32);
imp_as_mut_ptr!(AtomicU16, u16);
imp_as_mut_ptr!(AtomicI16, i16);
imp_as_mut_ptr!(AtomicU8, u8);
imp_as_mut_ptr!(AtomicI8, i8);
imp_as_mut_ptr!(AtomicBool, bool);
