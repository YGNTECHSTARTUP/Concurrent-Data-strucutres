use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;

static X: AtomicI32 = AtomicI32::new(0);
static Y: AtomicI32 = AtomicI32::new(0);

pub fn aa() {
    let a = thread::spawn(|| {
        let x = X.load(Ordering::Relaxed);
        Y.store(x, Ordering::Relaxed);
    });

    let b = thread::spawn(|| {
        let y = Y.load(Ordering::Relaxed);
        X.store(y, Ordering::Relaxed);
    });

    a.join().unwrap();
    b.join().unwrap();

    assert_eq!(X.load(Ordering::Relaxed), 0); // Might fail?
    assert_eq!(Y.load(Ordering::Relaxed), 0); // Might fail?
}
