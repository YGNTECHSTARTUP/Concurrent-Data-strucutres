use std::{
    mem::{self, MaybeUninit},
    sync::Arc,
    thread::{self, JoinHandle},
};

use crossbeam::utils::CachePadded;
use crossbeam_epoch::{Atomic, Guard, Owned, Shared, pin};

pub struct Queue<T> {
    head: CachePadded<Atomic<Node<T>>>,
    tail: CachePadded<Atomic<Node<T>>>,
}

pub struct Node<T> {
    data: MaybeUninit<T>,
    next: Atomic<Node<T>>,
}

unsafe impl<T> Sync for Queue<T> where T: Send {}
unsafe impl<T> Send for Queue<T> where T: Send {}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Queue<T> {
    pub fn new() -> Self {
        let sentinel = Box::into_raw(Box::new(Node {
            data: MaybeUninit::uninit(),
            next: Atomic::null(),
        }))
        .cast_const();
        Self {
            head: CachePadded::new(sentinel.into()),
            tail: CachePadded::new(sentinel.into()),
        }
    }
    pub fn push(&self, t: T, guard: &mut Guard) {
        let mut node = Owned::new(Node {
            data: MaybeUninit::new(t),
            next: Atomic::null(),
        });
        loop {
            let tail = self.tail.load(std::sync::atomic::Ordering::Acquire, guard);
            let tail_ref = unsafe { tail.deref() };
            let next = tail_ref
                .next
                .load(std::sync::atomic::Ordering::Acquire, guard);
            if !next.is_null() {
                let _ = self.tail.compare_exchange(
                    tail,
                    next,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                );
                continue;
            }
            match tail_ref.next.compare_exchange(
                Shared::null(),
                node,
                std::sync::atomic::Ordering::Release,
                std::sync::atomic::Ordering::Relaxed,
                guard,
            ) {
                Ok(new) => {
                    let _ = self.tail.compare_exchange(
                        tail,
                        new,
                        std::sync::atomic::Ordering::Release,
                        std::sync::atomic::Ordering::Acquire,
                        guard,
                    );
                    break;
                }
                Err(e) => node = e.new,
            }
            guard.repin();
        }
    }
    pub fn pop(&self, guard: &mut Guard) -> Option<T> {
        loop {
            let head = self.head.load(std::sync::atomic::Ordering::Acquire, guard);
            let next = unsafe { head.as_ref()? }
                .next
                .load(std::sync::atomic::Ordering::Acquire, &guard);
            let next_ref = unsafe { next.as_ref()? };
            let tail = self.tail.load(std::sync::atomic::Ordering::Acquire, guard);
            if tail == head {
                let _ = self.tail.compare_exchange(
                    tail,
                    next,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                );
            }
            if self
                .head
                .compare_exchange(
                    head,
                    next,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                )
                .is_ok()
            {
                let result = unsafe { next_ref.data.assume_init_read() };
                unsafe {
                    guard.defer_destroy(head);
                };
                return Some(result);
            }
            guard.repin();
        }
    }
}

impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        let sentinel = mem::take(&mut *self.head);
        let mut o_curr = unsafe { sentinel.into_owned() }.into_box().next;
        while let Some(curr) = unsafe { o_curr.try_into_owned() }.map(|s| s.into_box()) {
            drop(unsafe { curr.data.assume_init() });
            o_curr = curr.next;
        }
    }
}

pub fn q() {
    let a: Arc<Queue<u32>> = Arc::new(Queue::default());
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    for i in 0..100 {
        let a = Arc::clone(&a);
        handles.push(thread::spawn(move || {
            let mut guard = pin();
            for j in 0..100 {
                a.push(i * 100 + j, &mut guard);
            }
        }));
    }

    for _ in 0..5 {
        let a = Arc::clone(&a);
        handles.push(thread::spawn(move || {
            let mut guard = pin();
            let mut local_count = 0;
            while local_count < 200 {
                if let Some(val) = a.pop(&mut guard) {
                    println!("Popped:{}", val);
                    local_count += 1;
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}
