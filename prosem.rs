use std::{sync::atomic::AtomicI32, thread};
pub fn aw() {
    let a = AtomicI32::new(0);
    let b = AtomicI32::new(0);
    thread::scope(|s| {
        s.spawn(|| {
            println!("{:?}", a);
            a.load(std::sync::atomic::Ordering::Relaxed);

            println!("{:?}", a);
            a.store(1, std::sync::atomic::Ordering::Relaxed);

            println!("{:?}", a);
        });
        s.spawn(|| {
            println!("B{:?}", b);
            b.load(std::sync::atomic::Ordering::Relaxed);

            println!("B{:?}", b);
            b.store(1, std::sync::atomic::Ordering::Relaxed);

            println!("B{:?}", b);
        });
    });
    println!("{:?}{:?}", a, b);
}
