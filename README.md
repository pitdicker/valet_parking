# Overview

`valet_boy` can park your car (thread) and is happy to provide its service no matter the brand
(operating system).

`valet_boy` provides a cross-platform abstraction over thread parking. The goal is to provide an
abstraction with little overhead, which is `no_std`-compatible and works without allocations.

Two patterns:
- `Waiters`: multiple threads can wait on a single `AtomicUsize` until one thread wakes them all up at once.
- `Parker`: One thread parkes itself, and multiple threads or a timeout are able to wake it up.
