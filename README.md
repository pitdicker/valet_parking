# <img src="logo.svg" alt="" height="28px"> Valet parking

[![Build Status]][travis-ci.org] [![Latest Version]][crates.io] [![Documentation]][docs.rs] ![rustc] ![License]

`valet_parking` provides the service of parking your threads.

This library provides a cross-platform abstraction for thread parking which only needs one `AtomicUsize`
to operate on, and is `no_std`-compatible.

## Overview

Two patterns:
- `Waiters`: multiple threads can wait on a single `AtomicUsize` until one thread wakes them all up at once.
- `Parker`: One thread parkes itself, and multiple threads or a timeout are able to wake it up.

`valet` has two primary goals:
- Offer a cross-platform abstraction for thread parking.
- Be usable in other libraries that offer `no_std` support, even without `alloc`.

Some systems provide a way to park a thread while requiring nothing more than a single atomic. Other systems need multiple variables. The generic Posix implementation is such an example, it requires a condvar with mutex. `valet` is able to offer an API that requires only an `AtomicUsize` thanks to one key observation: we can use the stack of a parked thread to store the extra fields.

This does however takes some extra care on the side of `valet` to ensure no thread does reads from the stack of a thread while it is getting unparked.

## Platform interfaces

| OS               | interface               | notes
|------------------|-------------------------|------------------------------------------------------
| Linux, Android   | futex[¹] [²]            |
| Windows 8+       | WaitOnAddress[³]        |
| Windows XP+      | NT Keyed Events[⁴] [⁵]  | we keep a count of the waiting threads
| FreeBSD          | umutex[⁶]               |
| OpenBSD          | futex[⁷]                | (WIP)
| Posix-compatible | condition variable[⁸]   | we keep a queue of waiting threads
| Generic fallback | spin loop               | (WIP)
| Fuchsia OS       | futex[⁹]                | (untested)
| Redox            | futex[¹⁰]               | (untested)
| CloudABI         | lock[¹¹]                | (WIP), uses some bits of the atomic
| Fortanix SGX     | wait[¹²]                | (WIP), we keep a queue of waiting threads
| WASM atomics     | i32.atomic.wait[¹³]     | (WIP)
| MacOS 10.12+     | ulock_wait/ulock_wake   | (WIP)


The goal to provide an API that can be used without allocations has a big impact on the design of `valet`. Take the generic Posix implementation as an example. It requires a condvar with a mutex for thread parking. If `valet` were to provide some `ThreadParker` type containing fields for these two, you would have to store it in some place in memory that is accessable to both threads. This would typically be an `Arc`, or some other structure requring an allocation.

Instead `valet` provides an API that only requires you to pass around a reference to an `AtomicUsize`.
In the case were multiple variables are required


## Minimum Rust version

The current minimum required Rust version is 1.33.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.


[Build Status]: https://travis-ci.org/pitdicker/valet_parking.svg?branch=master
[travis-ci.org]: https://travis-ci.org/pitdicker/valet_parking
[Latest version]: https://img.shields.io/crates/v/rand.svg
[crates.io]: https://crates.io/crates/valet_parking
[Documentation]: https://docs.rs/valet_parking/badge.svg
[docs.rs]: https://docs.rs/valet_parking
[rustc]: https://img.shields.io/badge/rustc-1.33+-blue.svg
[License]: https://img.shields.io/crates/l/valet_parking.svg

[¹]: http://man7.org/linux/man-pages/man2/futex.2.html
[²]: https://www.akkadia.org/drepper/futex.pdf
[³]: https://docs.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitonaddress
[⁴]: http://locklessinc.com/articles/keyed_events/
[⁵]: http://joeduffyblog.com/2006/11/28/windows-keyed-events-critical-sections-and-new-vista-synchronization-features/
[⁶]: https://www.freebsd.org/cgi/man.cgi?query=_umtx_op
[⁷]: https://man.openbsd.org/futex
[⁸]: https://pubs.opengroup.org/onlinepubs/7908799/xsh/pthread_cond_wait.html
[⁹]: https://fuchsia.dev/fuchsia-src/reference/syscalls/futex_wait
[¹⁰]: https://doc.redox-os.org/kernel/kernel/syscall/futex/index.html
[¹¹]: https://nuxi.nl/blog/2016/06/22/cloudabi-futexes.html
[¹²]: https://docs.rs/fortanix-sgx-abi/0.3.3/fortanix_sgx_abi/struct.Usercalls.html#tcs-event-queues
[¹³]: https://github.com/WebAssembly/threads/blob/master/proposals/threads/Overview.md
