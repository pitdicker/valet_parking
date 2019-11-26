#[cfg(all(
    any(
        target_os = "android",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "fuchsia",
        target_os = "linux",
        target_os = "openbsd",
        target_os = "redox",
        all(target_arch = "wasm32", target_feature = "atomics")
    ),
    not(feature = "fallback")
))]
#[path = "futex.rs"]
mod sys;

#[cfg(windows)]
#[path = "windows.rs"]
mod sys;

#[cfg(all(any(target_os = "macos", target_os = "ios"), not(feature = "fallback")))]
#[path = "darwin.rs"]
mod sys;

#[cfg(all(
    unix,
    any(
        not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "ios",
            target_os = "macos",
            target_os = "openbsd",
            target_os = "redox"
        )),
        feature = "fallback"
    )
))]
#[path = "posix.rs"]
mod sys;

#[cfg(target_vendor = "fortanix")]
#[path = "fortanix.rs"]
mod sys;

#[cfg(any(all(
    unix,
    any(
        not(any(
            target_os = "android",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "fuchsia",
            target_os = "linux",
            target_os = "ios",
            target_os = "macos",
            target_os = "openbsd",
            target_os = "redox"
        )),
        feature = "fallback"
    )
),
fortanix))
]
mod waiter_queue;

pub(crate) use sys::{compare_and_wait, store_and_wake, Parker, park, unpark};
