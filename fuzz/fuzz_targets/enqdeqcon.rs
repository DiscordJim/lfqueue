#![no_main]

use libfuzzer_sys::fuzz_target;
use libfuzzer_sys::arbitrary::Arbitrary;
use std::sync::{Barrier, Arc};

#[derive(arbitrary::Arbitrary, Debug, Clone)]
pub struct LcsqParams {
    thresh: u32,
    threads: u8,
    values: Vec<u32>
}

#[inline(always)]
fn rescale_f32(value: f32) -> f32 {
   ((value / f32::MAX) + 1.0) / 2.0
}

#[inline(always)]
fn remap(value: f32, min: usize, max: usize) -> usize {
    (((max - min) as f32) * value) as usize + min
}



fuzz_target!(|data: LcsqParams| {

  
    let order = remap((data.thresh as f32) / u32::MAX as f32, 3, 10);


    let lcsq = Arc::new(lfqueue::ScqRing::new(order));

    let values = data.values.clone().into_iter().map(|v| remap((v as f32) / (u32::MAX as f32), 0, (1 << (order + 1)) - 1)).collect::<Vec<_>>();


    
    // std::thread::spawn({
    //     let values = values.clone();
    //     let data = data.clone();
    //     move || {
    //         std::thread::sleep(std::time::Duration::from_secs(360));
    //         println!("Aborting process.");
    //         println!("Data: {:?}", data);
    //         std::process::abort();
    //     }
    // });

    // let values = data.values.into_iter().map(|v| remap((v as f32) / (u32::MAX as f32), 0, 1 << (order + 1))).collect::<Vec<_>>();

    // let barrier = Arc::new(Barrier::new(data.threads as usize));

    let mut handles = vec![];
    for _ in 0..data.threads {
        handles.push(std::thread::spawn({

            // let barrier = barrier.clone();
            let lcsq = lcsq.clone();
            let values = values.clone();
            move || {
                
                for v in &values {
                    let _ = lcsq.enqueue(*v);
                }

                for v in &values {
                    lcsq.dequeue();
                }

            }

        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    // for value in &data.values {
    //     lcsq.enqueue(*value);
    // }

    // for i in 0..data.values.len() {
    //     assert_eq!(lcsq.dequeue(), Some(data.values[i]));
    // }
});
