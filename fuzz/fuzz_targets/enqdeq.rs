#![no_main]

use libfuzzer_sys::fuzz_target;
use libfuzzer_sys::arbitrary::Arbitrary;

use arbitrary::Unstructured;


#[derive(arbitrary::Arbitrary, Debug)]
pub struct LcsqParams {
    thresh: usize,
    values: Vec<usize>
}

#[inline(always)]
pub fn random_range(data: &mut Unstructured, min: usize, max: usize) -> usize {
    (((max - min) as f64) * (u64::arbitrary(data).unwrap() as f64 / usize::MAX as f64)) as usize + min
}


fuzz_target!(|data: &mut Unstructured| {

    
    let value = random_range(data, 3, 10);
    let lcsq = lfqueue::LcsqQueue::new(value);

    

    for value in &data.values {
        lcsq.enqueue(*value);
    }

    for i in 0..data.values.len() {
        assert_eq!(lcsq.dequeue(), Some(data.values[i]));
    }
});
