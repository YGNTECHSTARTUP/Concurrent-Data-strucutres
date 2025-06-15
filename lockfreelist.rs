use std::mem;

use crossbeam_epoch::{Atomic, Guard, Owned, Shared};

pub struct Node<K, V> {
    key: K,
    value: V,
    next: Atomic<Node<K, V>>,
}
pub struct List<K, V> {
    head: Atomic<Node<K, V>>,
}
unsafe impl<K, V> Send for List<K, V>
where
    K: Send,
    V: Send,
{
}
unsafe impl<K, V> Sync for List<K, V>
where
    K: Sync,
    V: Sync,
{
}

impl<K, V> Default for List<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K, V> Node<K, V> {
    pub fn new(key: K, value: V) -> Self {
        Self {
            next: Atomic::null(),
            key,
            value,
        }
    }
    pub fn into_value(self) -> V {
        self.value
    }
}

impl<K, V> List<K, V>
where
    K: Ord,
{
    pub fn new() -> Self {
        List {
            head: Atomic::null(),
        }
    }
}

impl<K, V> Drop for List<K, V> {
    fn drop(&mut self) {
        let mut o_curr = mem::take(&mut self.head);
        while let Some(curr) = unsafe { o_curr.try_into_owned() }.map(|s| s.into_box()) {
            o_curr = curr.next
        }
    }
}
struct Cursor<'g, K, V> {
    prev: &'g Atomic<Node<K, V>>,
    curr: Shared<'g, Node<K, V>>,
}

impl<K, V> Clone for Cursor<'_, K, V> {
    fn clone(&self) -> Self {
        Self {
            prev: self.prev,
            curr: self.curr,
        }
    }
}

impl<'g, K, V> Cursor<'g, K, V>
where
    K: Ord,
{
    pub fn new(prev: &'g Atomic<Node<K, V>>, curr: Shared<'g, Node<K, V>>) -> Self {
        Self {
            prev,
            curr: curr.with_tag(0),
        }
    }

    pub fn curr(&self) -> Shared<'g, Node<K, V>> {
        self.curr()
    }
}

impl<'g, K, V> Cursor<'g, K, V>
where
    K: Ord,
{
    #[inline]
    pub fn find_h(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        let mut prev_next = self.curr;
        let found = loop {
            let Some(curr) = (unsafe { self.curr.as_ref() }) else {
                return Ok(false);
            };

            let next = curr.next.load(std::sync::atomic::Ordering::Acquire, guard);
            while next.tag() != 0 {
                self.curr = next;
                continue;
            }
            match curr.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    prev_next = next;
                    self.prev = &curr.next;
                    self.curr = next;
                }
                std::cmp::Ordering::Equal => break true,
                std::cmp::Ordering::Greater => break false,
            }
        };
        if prev_next == self.curr {
            return Ok(found);
        } else {
            self.prev
                .compare_exchange(
                    prev_next,
                    self.curr,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                )
                .map_err(|_| ())?;
            let mut node = prev_next;
            while node.with_tag(0) != self.curr {
                let next = unsafe { node.deref() }
                    .next
                    .load(std::sync::atomic::Ordering::Relaxed, guard);
                unsafe { guard.defer_destroy(node) };
                node = next;
            }
            Ok(found)
        }
    }

    pub fn find_hm(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        loop {
            debug_assert_eq!(self.curr.tag(), 0);
            let Some(curr) = (unsafe { self.curr.as_ref() }) else {
                break Ok(false);
            };
            let mut next = curr.next.load(std::sync::atomic::Ordering::Acquire, guard);
            if next.tag() != 0 {
                next = next.with_tag(0);
                self.prev
                    .compare_exchange(
                        self.curr,
                        next,
                        std::sync::atomic::Ordering::Release,
                        std::sync::atomic::Ordering::Relaxed,
                        guard,
                    )
                    .map_err(|_| ())?;
                unsafe { guard.defer_destroy(self.curr) };
                self.curr = next;
                continue;
            }
            match curr.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    self.prev = &curr.next;
                    self.curr = curr.next.load(std::sync::atomic::Ordering::Acquire, guard)
                }
                std::cmp::Ordering::Equal => return Ok(true),
                std::cmp::Ordering::Greater => return Ok(false),
            }
        }
    }

    #[inline]
    pub fn find_hms(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        Ok(loop {
            let a = unsafe { self.curr.as_ref() };
            let a = match a {
                Some(curr) => curr,
                None => break false,
            };
            match a.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    self.prev = &a.next;
                    self.curr = a.next.load(std::sync::atomic::Ordering::Acquire, guard)
                }
                std::cmp::Ordering::Equal => {
                    break a
                        .next
                        .load(std::sync::atomic::Ordering::Release, guard)
                        .tag()
                        == 0;
                }
                std::cmp::Ordering::Greater => break false,
            }
        })
    }

    #[inline]
    pub fn lookup(&self) -> &'g V {
        &unsafe { self.curr.as_ref() }.unwrap().value
    }
    #[inline]
    pub fn insert(
        &self,
        mut node: Owned<Node<K, V>>,
        guard: &'g Guard,
    ) -> Result<(), Owned<Node<K, V>>> {
        node.next = self.curr.into();
        let next = unsafe { self.curr.as_ref() }.unwrap().next;
        match self.prev.compare_exchange(
            self.curr,
            next,
            std::sync::atomic::Ordering::Release,
            std::sync::atomic::Ordering::Relaxed,
            guard,
        ) {
            Ok(k) => {
                self.curr = node;
                Ok(())
            }
            Err(e) => return e.new,
        }
    }
    #[inline]
    pub fn delete(&mut self, guard: &'g Guard) -> Result<&'g V, ()> {
        let curr_node = unsafe { self.curr.as_ref() }.unwrap();
        let next = curr_node
            .next
            .fetch_or(1, std::sync::atomic::Ordering::AcqRel, guard);
        if next.tag() == 1 {
            return Err(());
        } else {
            if self
                .prev
                .compare_exchange(
                    self.curr,
                    next,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                )
                .is_ok()
            {
                unsafe {
                    guard.defer_destroy(self.curr);
                }
            }
            self.curr = next;
            Ok(&curr_node.value)
        }
    }
    pub fn h(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        let mut prev_next = self.curr;
        let found = loop {
            let Some(curr) = (unsafe { self.curr.as_ref() }) else {
                break false;
            };
            let next = curr.next.load(std::sync::atomic::Ordering::Acquire, guard);
            if next.tag() != 0 {
                self.curr = next;
                continue;
            }
            match curr.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    prev_next = next;
                    self.prev = &curr.next;
                    self.curr = next;
                }
                std::cmp::Ordering::Equal => break true,
                std::cmp::Ordering::Greater => break false,
            }
        };
        if self.curr == prev_next {
            return Ok(found);
        } else {
            self.prev
                .compare_exchange(
                    self.curr,
                    next,
                    std::sync::atomic::Ordering::Release,
                    std::sync::atomic::Ordering::Relaxed,
                    guard,
                )
                .map_err(|_| ())?;
        }
    }

    pub fn hm(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        loop {
            debug_assert_eq!(self.curr.tag(), 0);
            let Some(curr) = (unsafe { self.curr.as_ref() }) else {
                break Ok(false);
            };
            let mut next = curr.next.load(std::sync::atomic::Ordering::Acquire, guard);
            if next.tag() != 0 {
                next = next.with_tag(0);
                self.prev
                    .compare_exchange(
                        self.curr,
                        next,
                        std::sync::atomic::Ordering::Release,
                        std::sync::atomic::Ordering::Relaxed,
                        guard,
                    )
                    .map_err(|_| ())?;
                unsafe {
                    guard.defer_destroy(self.curr);
                };
                self.curr = next;
                continue;
            }
            match curr.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    self.prev = &curr.next;
                    self.curr = next;
                }
                std::cmp::Ordering::Equal => break Ok(true),
                std::cmp::Ordering::Greater => break Ok(false),
            }
        }
    }

    pub fn hms(&mut self, key: &K, guard: &'g Guard) -> Result<bool, ()> {
        Ok(loop {
            let Some(curr) = (unsafe { self.curr.as_ref() }) else {
                break false;
            };
            let next = curr.next.load(std::sync::atomic::Ordering::Acquire, guard);
            match curr.key.cmp(key) {
                std::cmp::Ordering::Less => {
                    self.prev = &curr.next;
                    self.curr = next;
                }
                std::cmp::Ordering::Equal => {
                    break curr
                        .next
                        .load(std::sync::atomic::Ordering::Acquire, guard)
                        .tag()
                        == 0;
                }
                std::cmp::Ordering::Greater => break false,
            }
        })
    }
}
