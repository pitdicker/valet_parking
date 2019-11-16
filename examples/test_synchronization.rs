/// Test if the OnceCell properly synchronizes.
/// Needs to be run in release mode.
///
/// We create a `Vec` with `N_ROUNDS` of `OnceCell`s. All threads will walk the `Vec`, and race to
/// be the first one to initialize a cell.
/// Every thread adds the results of the cells it sees to an accumulator, which is compared at the
/// end.
/// All threads should end up with the same result.
use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicUsize, Ordering};

use valet_parking::{Waiters, RESERVED_BITS, RESERVED_MASK};

const N_THREADS: usize = 32;
const N_ROUNDS: usize = 1_000_000;

static CELLS: OnceCell<Vec<OnceCell<usize>>> = OnceCell::new();
static RESULT: OnceCell<usize> = OnceCell::new();

fn main() {
    println!("Started test");
    let start = std::time::Instant::now();
    CELLS.get_or_init(vec![OnceCell::new(); N_ROUNDS]);
    let threads = (0..N_THREADS)
        .map(|i| std::thread::spawn(move || thread_main(i)))
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
    println!("{:?}", start.elapsed());
    println!("No races detected");
}

fn thread_main(i: usize) {
    let cells = CELLS.get().unwrap();
    let mut accum = 0;
    for cell in cells.iter() {
        let &value = cell.get_or_init(i);
        accum += value;
    }
    assert_eq!(RESULT.get_or_init(accum), &accum);
}

const INCOMPLETE: usize = 0 << RESERVED_BITS;
const RUNNING: usize = 1 << RESERVED_BITS;
const COMPLETE: usize = 2 << RESERVED_BITS;

struct OnceCell<T> {
    state: AtomicUsize,
    value: UnsafeCell<Option<T>>,
}

impl<T> OnceCell<T> {
    const fn new() -> OnceCell<T> {
        OnceCell {
            state: AtomicUsize::new(INCOMPLETE),
            value: UnsafeCell::new(None),
        }
    }

    fn get(&self) -> Option<&T> {
        if self.state.load(Ordering::SeqCst) == COMPLETE {
            unsafe { &*self.value.get() }.as_ref()
        } else {
            None
        }
    }

    fn get_or_init(&self, value: T) -> &T {
        if let Some(val) = self.get() {
            return val;
        }
        self.init(value)
    }

    fn init(&self, value: T) -> &T {
        let mut state = self.state.load(Ordering::SeqCst);
        loop {
            match state {
                COMPLETE => break,
                INCOMPLETE => {
                    let old = self
                        .state
                        .compare_and_swap(state, RUNNING, Ordering::SeqCst);
                    if old != state {
                        state = old;
                        continue;
                    }

                    unsafe { self.value.get().write(Some(value)) };
                    unsafe { self.state.store_and_wake(COMPLETE) }
                    assert!(self.state.load(Ordering::SeqCst) & !RESERVED_MASK == COMPLETE);
                    break;
                }
                _ => {
                    assert!(state & !RESERVED_MASK == RUNNING);
                    self.state.compare_and_wait(RUNNING);
                    state = self.state.load(Ordering::SeqCst);
                }
            }
        }
        self.get().unwrap()
    }
}

impl<T: Clone> Clone for OnceCell<T> {
    fn clone(&self) -> OnceCell<T> {
        let res = OnceCell::new();
        if let Some(value) = self.get() {
            let _ = res.get_or_init(value.clone());
        }
        res
    }
}

unsafe impl<T: Sync + Send> Sync for OnceCell<T> {}
unsafe impl<T: Send> Send for OnceCell<T> {}
