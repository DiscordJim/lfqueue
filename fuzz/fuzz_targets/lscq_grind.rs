#![no_main]

use arbitrary::*;
use lfqueue::{GrindInstr, UnboundedQueue, configure_grind, MockQueue};
use libfuzzer_sys::fuzz_target;
use std::collections::VecDeque;


fuzz_target!(|data: &[u8]| {
    let mut uns = Unstructured::new(data);

    let grind = configure_grind(&mut uns);

    let mut mock = MockQueue::new(grind.order);

    let mut queue = UnboundedQueue::<usize>::with_segment_size(grind.order);

    for operation in grind.instructions {
        match operation {
            GrindInstr::Dequeue => {
                assert_eq!(mock.dequeue(), queue.dequeue());
            }
            GrindInstr::Enqueue(item) => {
                queue.enqueue(item);
                mock.enqueue_boundless(item);
                // assert_eq!(mock.enqueue(item), queue.enqueue(item));
            }
        }
    }


    
});
