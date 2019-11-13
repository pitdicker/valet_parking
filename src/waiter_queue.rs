use core::sync::atomic::{AtomicUsize, Ordering};

use crate::{Parker, Waiters, FREE_BITS, RESERVED_MASK};

// Align so that the 5 lower bits are free for other uses.
#[repr(align(32))]
struct Waiter {
    parker: AtomicUsize,
    next: usize,
}

impl Waiters for AtomicUsize {
    unsafe fn compare_and_wait(&self, compare: usize) {
        let mut current = self.load(Ordering::Relaxed);
        loop {
            let pub_bits = current & !RESERVED_MASK;
            let next = (current & RESERVED_MASK) << FREE_BITS;
            if pub_bits != compare {
                break;
            }
            // Create a node for our current thread.
            let node = Waiter {
                parker: AtomicUsize::new(0),
                next: next,
            };
            let me = pub_bits | ((&node as *const Waiter as usize) >> FREE_BITS);

            // Try to slide in the node at the head of the linked list, making sure
            // that another thread didn't just replace the head of the linked list.
            let old = self.compare_and_swap(current, me, Ordering::Release);
            if old != current {
                current = old;
                continue;
            }

            // We have enqueued ourselves, now lets wait.
            // The parker will not park our thread if we got unparked just now.
            node.parker.park();
            current = self.load(Ordering::Relaxed);
        }
    }

    unsafe fn store_and_wake(&self, new: usize) {
        let queue = self.swap(new, Ordering::RelAcq);

        // Walk the entire linked list of waiters and wake them up (in lifo
        // order, last to register is first to wake up).
        let mut next = ((queue & RESERVED_MASK) << FREE_BITS) as *const Waiter;
        while !next.is_null() {
            let current = next;
            next = (*current).next as *const Waiter;
            (*current).parker.unpark();
        }
    }
}
