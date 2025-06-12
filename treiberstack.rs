use std::{
    mem::{self, MaybeUninit},
    ptr,
    sync::{atomic::Ordering, Arc}, thread::{self, JoinHandle},
};

use crossbeam_epoch::{Atomic, Owned, Shared};

pub struct Node<T> {
    data: MaybeUninit<T>,
    next: *const Node<T>,
}

pub struct Stack<T> {
    head: Atomic<Node<T>>,
}

pub fn tr( ) {
    let a = Arc::new(Stack::new());
    let mut handles:Vec<JoinHandle<()>> = Vec::new();
    for i in 0..1000 {
        let a = Arc::clone(&a);
        let handle = thread::spawn(move||{
            a.push(10);
        });
        handles.push(handle);
    }
    
    for i in 0..10000 {
        let a = Arc::clone(&a);
        let handle = thread::spawn(move||{
            a.pop();
        });
        handles.push(handle);
    }
    for h in handles {
        h.join().unwrap();
    }
    println!("Stack:{:?}",a.is_empty())
}

unsafe impl<T> Send for Stack<T> where T: Send {}
unsafe impl<T> Sync for Stack<T> where T: Send {}

impl<T> Stack<T> {
    pub fn new() -> Stack<T> {
        Stack {
            head: Atomic::null(),
        }
    }
    pub fn push(&self, t: T) {
        let mut node = Owned::new(Node {
            data: MaybeUninit::new(t),
            next: ptr::null(),
        });
        let guard = unsafe { crossbeam_epoch::unprotected() };
        let mut top = self.head.load(Ordering::Relaxed, &guard);
        loop {
            node.next = top.as_raw();
            match self.head.compare_exchange(
                top,
                node,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(_) => break,
                Err(e) => {
                    top = e.current;
                    node = e.new;
                }
            }
        }
    }
    pub fn pop(&self) -> Option<T> {
        let mut guard = crossbeam_epoch::pin();
        loop {
            let top = self.head.load(Ordering::Acquire, &guard);
            let t = unsafe { top.as_ref()? };
            let next = Shared::from(t.next);
            if self
                .head
                .compare_exchange(top, next, Ordering::Release, Ordering::Relaxed, &guard)
                .is_ok()
            {
                let res = unsafe { t.data.assume_init_read() };
                unsafe { guard.defer_destroy(top) };
                return Some(res);
            }
            guard.repin();
        }
    }

    pub fn is_empty(&self) -> bool {
        let guard = crossbeam_epoch::pin();
        self.head.load(Ordering::Acquire, &guard).is_null()
    }
}

impl<T> Drop for Stack<T> {
    fn drop(&mut self) {
        let mut curr = mem::take(&mut self.head);
        while let Some(c) = unsafe {curr.try_into_owned()}.map(|o|{o.into_box()}) {
            drop(unsafe {c.data.assume_init()});
            curr = c.next.into();
        }
    }
}




