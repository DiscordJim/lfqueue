#![no_main]

use arbitrary::*;
use lfqueue::{GrindInstr, const_queue, GrindConfiguration, ConstBoundedQueue, configure_grind, MockQueue};
use libfuzzer_sys::fuzz_target;
use std::collections::VecDeque;

#[inline(always)]
pub fn random_range(data: &mut Unstructured, min: usize, max: usize) -> usize {
    (((max - min) as f64) * (u64::arbitrary(data).unwrap() as f64 / usize::MAX as f64)) as usize
        + min
}

fn fuzz_queue<const N: usize>(grind: &GrindConfiguration, queue: ConstBoundedQueue<usize, N>) {
    let mock = MockQueue::new(N >> 1);
    for operation in &grind.instructions {
        match operation {
            GrindInstr::Dequeue => {
                assert_eq!(mock.dequeue(), queue.dequeue());
            }
            GrindInstr::Enqueue(item) => {
                assert_eq!(mock.enqueue(*item), queue.enqueue(*item));
            }
        }
    }
}


macro_rules! generate_switch {
    ($switch:expr, $grind:expr, $( $size:expr),* ) => {
        match $switch {
            $(
                $size => fuzz_queue($grind, const_queue!(usize; 1 << $size)),

            )*
            x => panic!("{x} is not in range.")
        }
    };
}


fuzz_target!(|data: &[u8]| {
    let mut uns = Unstructured::new(data);

    let switch = random_range(&mut uns, 1, 6);
    let grind = configure_grind(&mut uns);


    generate_switch!(switch, &grind, 1, 2, 3, 4, 5, 6);


    

    // let grind = configure_grind(&mut uns);

    // let mut mock = MockQueue::new(grind.order);

    // let mut queue = AllocBoundedQueue::new(grind.order);

    
});
