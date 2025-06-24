#![no_main]

use arbitrary::*;
use lfqueue::{GrindInstr, MockQueue, LcsqQueue, configure_grind};
use libfuzzer_sys::fuzz_target;
use std::collections::VecDeque;
use std::sync::Arc;

#[inline(always)]
pub fn random_range(data: &mut Unstructured, min: usize, max: usize) -> usize {
    (((max - min) as f64) * (u64::arbitrary(data).unwrap() as f64 / usize::MAX as f64)) as usize
        + min
}

fuzz_target!(|data: &[u8]| {
    let mut uns = Unstructured::new(data);

    let grind = configure_grind(&mut uns);
    let mut queue = Arc::new(LcsqQueue::<usize>::new(grind.order));

    let mut handles = vec![];

    for i in 0..random_range(&mut uns, 1, 150) {
        handles.push(std::thread::spawn({
            let queue = queue.clone();
            // let mock = mock.clone();
            let instructions = grind.instructions.clone();
            move || {
                for operation in instructions {
                    match operation {
                        GrindInstr::Dequeue => {
                            queue.dequeue();
                        }
                        GrindInstr::Enqueue(item) => {
                            queue.enqueue(item);
                        }
                    }
                }
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
});
