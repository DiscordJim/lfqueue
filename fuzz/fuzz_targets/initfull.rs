#![no_main]

use libfuzzer_sys::fuzz_target;
use arbitrary::{Arbitrary, Unstructured};
use lfqueue::ScqRing;

#[inline(always)]
pub fn random_range(data: &mut Unstructured, min: usize, max: usize) -> usize {
    (((max - min) as f64) * (u64::arbitrary(data).unwrap() as f64 / usize::MAX as f64)) as usize + min
}




fuzz_target!(|data: &[u8]| {
    let mut unstruc = Unstructured::new(data);

    

    let order = random_range(&mut unstruc, 0, 10);
    let size = 1 << order;

    let manual = ScqRing::<false>::new(order);
    let auto = ScqRing::<false>::new_full(order);
    assert_eq!(manual.capacity(), size);
    assert_eq!(auto.capacity(), size);

    for i in 0..size {
        manual.enqueue(i).unwrap();
    }

    assert_eq!(manual, auto);

    for i in 0..size {
        assert_eq!(auto.dequeue(), Some(i));
        auto.enqueue(i).unwrap();
    }

    for i in 0..size {
        assert_eq!(auto.dequeue(), Some(i), "Auto: {auto:?}, Manual: {manual:?}");
    }
});
