// use std::{
//     sync::{
//         Arc,
//         atomic::{AtomicBool, AtomicPtr, fence},
//     },
//     thread,
// };

// struct LinkedList {
//     head: CachePadded<AtomicPtr<u32>>,
//     tail: CachePadded<AtomicPtr<u32>>,
// }

// use crossbeam::{
//     channel::{bounded, unbounded},
//     utils::CachePadded,
// };

// pub fn cs() {
//     let (s, r) = unbounded();
//     let issent = Arc::new(AtomicBool::new(false));
//     for i in 0..100 {
//         let se = s.clone();
//         let r = r.clone();
//         let issent = Arc::clone(&issent);
//         thread::scope(|s| {
//             let sente = Arc::clone(&issent);
//             s.spawn(move || {
//                 let ka = issent.load(std::sync::atomic::Ordering::Acquire);
//                 if ka {
//                     std::hint::spin_loop();
//                 }
//                 se.send(i).unwrap();
//                 issent.store(true, std::sync::atomic::Ordering::Release);
//             });
//             s.spawn(move || {
//                 let ke = sente.load(std::sync::atomic::Ordering::Acquire);
//                 if !ke {
//                     std::hint::spin_loop();
//                 }
//                 let k = r.recv().unwrap();
//                 sente.store(false, std::sync::atomic::Ordering::Release);
//                 println!("{:?}", k);
//             });
//         })
//     }
// }
