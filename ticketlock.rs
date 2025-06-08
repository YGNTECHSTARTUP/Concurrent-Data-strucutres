use std::{
    sync::{Arc, atomic::AtomicUsize},
    thread,
};

struct TicketLock {
    current: AtomicUsize,
    next: AtomicUsize,
}

impl TicketLock {
    pub fn new() -> Self {
        Self {
            current: AtomicUsize::new(0),
            next: AtomicUsize::new(0),
        }
    }
    pub fn lock(&self) -> usize {
        let ticket = self.next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        while ticket != self.current.load(std::sync::atomic::Ordering::Acquire) {
            std::hint::spin_loop();
        }
        ticket
    }
    pub fn unlock(&self, ticket: usize) {
        self.current
            .store(ticket.wrapping_add(1), std::sync::atomic::Ordering::Release);
    }
}

pub fn a() {
    let a = Arc::new(TicketLock::new());
    let b = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let a = Arc::clone(&a);
        let b = Arc::clone(&b);
        let hand = thread::spawn(move || {
            for _ in 0..100 {
                let k = a.lock();
                b.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                a.unlock(k);
            }
        });
        handles.push(hand);
    }
    for h in handles {
        h.join().unwrap();
    }
    println!("Expected:{:?}", 8 * 100);
    println!("Actual:{:?}", b.load(std::sync::atomic::Ordering::Relaxed));
}
