use std::{mem, sync::Arc, thread};

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
        &mut self,
        mut node: Owned<Node<K, V>>,
        guard: &'g Guard,
    ) -> Result<(), Owned<Node<K, V>>> {
        node.next = self.curr.into();
        match self.prev.compare_exchange(
            self.curr,
            node,
            std::sync::atomic::Ordering::Release,
            std::sync::atomic::Ordering::Relaxed,
            guard,
        ) {
            Ok(node) => {
                self.curr = node;
                Ok(())
            }
            Err(e) => Err(e.new),
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

impl<K, V> List<K, V> {
    pub fn new() -> Self {
        List {
            head: Atomic::null(),
        }
    }
}
impl<K, V> List<K, V>
where
    K: Ord,
{
    #[inline]
    pub fn head<'g>(&'g self, guard: &'g Guard) -> Cursor<'g, K, V> {
        Cursor::new(
            &self.head,
            self.head.load(std::sync::atomic::Ordering::Acquire, guard),
        )
    }
    #[inline]
    fn find<'g, F>(&'g self, key: &K, find: &F, guard: &'g Guard) -> (bool, Cursor<'g, K, V>)
    where
        F: Fn(&mut Cursor<'g, K, V>, &K, &'g Guard) -> Result<bool, ()>,
    {
        loop {
            let mut cursor = self.head(guard);
            if let Ok(r) = find(&mut cursor, key, guard) {
                return (r, cursor);
            }
        }
    }
    #[inline]
    fn lookup<'g, F>(&'g self, key: &K, find: F, guard: &'g Guard) -> Option<&'g V>
    where
        F: Fn(&mut Cursor<'g, K, V>, &K, &'g Guard) -> Result<bool, ()>,
    {
        let (found, cursor) = self.find(key, &find, guard);
        if found { Some(cursor.lookup()) } else { None }
    }
    #[inline]
    fn insert<'g, F>(&'g self, key: K, value: V, find: F, guard: &'g Guard) -> bool
    where
        F: Fn(&mut Cursor<'g, K, V>, &K, &'g Guard) -> Result<bool, ()>,
    {
        let mut node = Owned::new(Node::new(key, value));
        loop {
            let (found, mut cursor) = self.find(&node.key, &find, guard);
            if found {
                return false;
            }
            match cursor.insert(node, guard) {
                Ok(()) => return true,
                Err(n) => node = n,
            }
        }
    }

    #[inline]
    fn delete<'g, F>(&'g self, key: &K, find: F, guard: &'g Guard) -> Option<&'g V>
    where
        F: Fn(&mut Cursor<'g, K, V>, &K, &'g Guard) -> Result<bool, ()>,
    {
        loop {
            let (found, mut cursor) = self.find(key, &find, guard);
            if !found {
                return None;
            }
            if let Ok(value) = cursor.delete(guard) {
                return Some(value);
            }
        }
    }
    pub fn harris_lookup<'g>(&'g self, key: &K, guard: &'g Guard) -> Option<&'g V> {
        self.lookup(key, Cursor::find_h, guard)
    }

    /// Insert the value with the Harris strategy.
    pub fn harris_insert<'g>(&'g self, key: K, value: V, guard: &'g Guard) -> bool {
        self.insert(key, value, Cursor::find_h, guard)
    }

    /// Attempts to delete the value with the Harris strategy.
    pub fn harris_delete<'g>(&'g self, key: &K, guard: &'g Guard) -> Option<&'g V> {
        self.delete(key, Cursor::find_h, guard)
    }

    /// Lookups the value at `key` with the Harris-Michael strategy.
    pub fn harris_michael_lookup<'g>(&'g self, key: &K, guard: &'g Guard) -> Option<&'g V> {
        self.lookup(key, Cursor::find_hm, guard)
    }

    /// Insert a `key`-`value`` pair with the Harris-Michael strategy.
    pub fn harris_michael_insert(&self, key: K, value: V, guard: &Guard) -> bool {
        self.insert(key, value, Cursor::find_hm, guard)
    }

    /// Delete the value at `key` with the Harris-Michael strategy.
    pub fn harris_michael_delete<'g>(&'g self, key: &K, guard: &'g Guard) -> Option<&'g V> {
        self.delete(key, Cursor::find_hm, guard)
    }

    /// Lookups the value at `key` with the Harris-Herlihy-Shavit strategy.
    pub fn harris_herlihy_shavit_lookup<'g>(&'g self, key: &K, guard: &'g Guard) -> Option<&'g V> {
        self.lookup(key, Cursor::find_hms, guard)
    }
}

pub fn lc() {
    // Use Arc to share the list among threads
    // Number of threads
    let list = Arc::new(List::<i32, String>::new());
    let thread_count = 4;
    use crossbeam_epoch as epoch;
    // Insert some elements concurrently
    let mut handles = vec![];
    for i in 0..thread_count {
        let list = Arc::clone(&list);
        handles.push(thread::spawn(move || {
            let guard = &epoch::pin(); // Hazard pointer guard

            for j in 0..5 {
                let key = i * 10 + j;
                let value = format!("val-{}", key);
                let inserted = list.harris_michael_insert(key, value, guard);
                println!("[Thread {i}] Inserted key {key}: {inserted}");
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    println!("\n--- Lookup Phase ---");
    // Look up some keys
    {
        let guard = &epoch::pin();
        for key in [3, 12, 21, 30, 33] {
            match list.harris_michael_lookup(&key, guard) {
                Some(v) => println!("Lookup key {key}: Found value '{}'", v),
                None => println!("Lookup key {key}: Not found"),
            }
        }
    }

    println!("\n--- Deletion Phase ---");
    // Delete a few keys
    {
        let guard = &epoch::pin();
        for key in [12, 21] {
            match list.harris_michael_delete(&key, guard) {
                Some(v) => println!("Deleted key {key}: value '{}'", v),
                None => println!("Delete key {key}: Not found"),
            }
        }
    }

    println!("\n--- Post-Deletion Lookup ---");
    // Re-lookup deleted keys
    {
        let guard = &epoch::pin();
        for key in [12, 21] {
            match list.harris_michael_lookup(&key, guard) {
                Some(v) => println!("Lookup key {key}: Found value '{}'", v),
                None => println!("Lookup key {key}: Not found"),
            }
        }
    }

    println!("\nDone.");
}
