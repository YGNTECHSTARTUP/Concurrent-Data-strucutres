use std::{
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering::*},
    },
    thread::{self},
    time::Instant,
};

use crossbeam::utils::CachePadded;

struct Node {
    locked: AtomicBool,
    next: AtomicPtr<CachePadded<Node>>,
}

struct McsLock {
    tail: AtomicPtr<CachePadded<Node>>,
}

struct Token(*mut CachePadded<Node>);

impl Node {
    pub fn new(lock: bool) -> *mut CachePadded<Node> {
        Box::into_raw(Box::new(CachePadded::new(Node {
            locked: AtomicBool::new(lock),
            next: AtomicPtr::new(null_mut()),
        })))
    }
}

impl McsLock {
    pub fn new() -> Self {
        Self {
            tail: AtomicPtr::new(null_mut()),
        }
    }

    pub fn lock(&self) -> Token {
        let node = Node::new(true);
        let prev = self.tail.swap(node, Relaxed);
        if prev.is_null() {
            return Token(node);
        }

        unsafe {
            (*prev).next.store(node, Release);
            while (*node).locked.load(Acquire) {
                std::hint::spin_loop();
            }
        }

        Token(node)
    }

    pub fn unlock(&self, token: Token) {
        let node = token.0;
        let mut next = unsafe { (*node).next.load(Acquire) };
        if next.is_null() {
            if self
                .tail
                .compare_exchange(node, null_mut(), Release, Relaxed)
                .is_ok()
            {
                drop(unsafe { Box::from_raw(node) });
                return;
            }
            while {
                next = unsafe { (*node).next.load(Acquire) };
                next.is_null()
            } {
                std::thread::yield_now();
            }
        }

        unsafe {
            (*next).locked.store(false, Release);
            drop(Box::from_raw(node)); // node is now safe to drop
        }
    }
}

pub fn mcslock() {
    let lock = Arc::new(McsLock::new());
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    let start = Instant::now();

    for _ in 0..8 {
        let lock = Arc::clone(&lock);
        let counter = Arc::clone(&counter);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let token = lock.lock();
                counter.fetch_add(1, Relaxed);
                lock.unlock(token);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    let duration = start.elapsed();

    println!("Expected: {} completed at {:?}", 8 * 100, duration);
    println!("Actual: {}", counter.load(Relaxed));
}
