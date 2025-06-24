use std::sync::{Arc, Barrier};

#[derive(Debug)]
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


fn run(data: LcsqParams) {
    let order = remap((data.thresh as f32) / u32::MAX as f32,3, 10);
    // let order = 3;
    println!("order: {order}");

    let lcsq = Arc::new(lfqueue::ScqQueue::new(order));

    // printl
    let values = data.values.into_iter().map(|v| remap((v as f32) / (u32::MAX as f32), 0, (1 << (order + 1)) - 1)).collect::<Vec<_>>();
    // println!("Values: {values:?}");


    // std::process::abort();

    // std::thread::spawn({
    //     let values = values.clone();
    //     move || {
    //     std::thread::sleep(std::time::Duration::from_secs(5));
    //     println!("Aborting process.");
    //     println!("Order: {order}");
    //     println!("Values: {values:?}");
    //     std::process::abort();
    // }
    // });

    // let values = data.values.into_iter().map(|v| remap((v as f32) / (u32::MAX as f32), 0, 1 << (order + 1))).collect::<Vec<_>>();

    let barrier = Arc::new(Barrier::new(data.threads as usize));

    let mut handles = vec![];
    for _ in 0..data.threads {
        handles.push(std::thread::spawn({

            let barrier = barrier.clone();
            let lcsq = lcsq.clone();
            let values = values.clone();
            move || {
                
                for v in &values {
                    let _ = lcsq.enqueue(*v);
                }
                for v in &values {
                    lcsq.dequeue();
                }
                // println!("Done...");

            }

        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
}


pub fn main() {
    run(  LcsqParams {
            thresh: 724249391,
            threads: 255,
            values: vec![
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                3147800479,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642559,
                3149642683,
                3149642683,
                3149642683,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                1069267899,
                3149642683,
                2678038459,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                4294942623,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149642683,
                3149610939,
                3149642683,
                3149642683,
                2678045627,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                2678038431,
                3147800479,
                3149642683,
                3149642683,
                741129147,
            ],
        });
}