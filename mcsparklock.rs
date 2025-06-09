use std::{
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering::*},
        Arc,
    },
    thread::{self, Thread},
};

use crossbeam::utils::CachePadded;

struct McsParkLock {
    tail: AtomicPtr<CachePadded<Node>>,
}

struct Node {
    thread: Arc<Thread>,
    next: AtomicPtr<CachePadded<Node>>,
    locked: AtomicBool,
}

impl Node {
    pub fn new(locked: bool) -> *mut CachePadded<Self> {
        Box::into_raw(Box::new(CachePadded::new(Self {
            thread: Arc::new(thread::current()),
            next: AtomicPtr::new(null_mut()),
            locked: AtomicBool::new(locked),
        })))
    }
}

struct Token(*mut CachePadded<Node>);

impl McsParkLock {
    pub fn new() -> McsParkLock {
        McsParkLock {
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
        }

        while unsafe { (*node).locked.load(Acquire) } {
            thread::park();
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
                unsafe {
                    drop(Box::from_raw(node));
                }
                return;
            }

            while {
                next = unsafe { (*node).next.load(Acquire) };
                next.is_null()
            } {}
        }
        unsafe {
            let next_ref = &*next;
            let t = next_ref.thread.clone();
            next_ref.locked.store(false, Release);
            t.unpark();
            drop(Box::from_raw(node));
        }
    }
}

pub fn mcsparklock() {
    let lock = Arc::new(McsParkLock::new());
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    for _ in 0..8 {
        let lock = Arc::clone(&lock);
        let counter = Arc::clone(&counter);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                let token = lock.lock();
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                lock.unlock(token);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    println!("Expected: {}", 8 * 100);
    println!(
        "Actual: {}",
        counter.load(std::sync::atomic::Ordering::Relaxed)
    );
}
