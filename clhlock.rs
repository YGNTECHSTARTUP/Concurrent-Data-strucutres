use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicPtr, AtomicUsize},
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use crossbeam::utils::CachePadded;

struct Node {
    locked: AtomicBool,
}

struct Clhlock {
    ptr: AtomicPtr<CachePadded<Node>>,
}

impl Node {
    fn new(lock: bool) -> *mut CachePadded<Node> {
        Box::into_raw(Box::new(CachePadded::new(Self {
            locked: AtomicBool::new(lock),
        })))
    }
}

struct Token(*const CachePadded<Node>);

impl Clhlock {
    pub fn new() -> Self {
        Self {
            ptr: AtomicPtr::new(Node::new(false)),
        }
    }
    pub fn lock(&self) -> Token {
        let node = Node::new(true);
        let prev = self.ptr.swap(node, std::sync::atomic::Ordering::Relaxed);
        while unsafe { (*prev).locked.load(std::sync::atomic::Ordering::Acquire) } {
            std::hint::spin_loop();
        }
        unsafe {
            drop(Box::from_raw(prev));
        }
        Token(node)
    }
    pub fn unlock(&self, token: Token) {
        unsafe {
            (*token.0)
                .locked
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
}

impl Drop for Clhlock {
    fn drop(&mut self) {
        let node = self.ptr.get_mut();
        unsafe {
            drop(Box::from_raw(node));
        }
    }
}

pub fn cllock() {
    let a = Arc::new(Clhlock::new());
    let counter = Arc::new(AtomicUsize::new(0));
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    for _ in 0..8 {
        let a = Arc::clone(&a);
        let counter = Arc::clone(&counter);
        let handle = thread::spawn(move || {
            for _ in 0..100 {
                let d = a.lock();
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                a.unlock(d);
            }
        });
        handles.push(handle);
    }
    let start = Instant::now();
    for h in handles {
        h.join().unwrap();
    }
    let duration = start.elapsed();
    println!("Expexted:{:?} completed at {:?}", 8 * 100, duration);
    println!(
        "Actual Value:{:?}",
        counter.load(std::sync::atomic::Ordering::Relaxed)
    );
}

