use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_queue::ArrayQueue;
use lfqueue::{
    AllocBoundedQueue, UnboundedQueue, const_queue, ConstBoundedQueue
};
use std::collections::VecDeque;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Instant;

// NOTE: Our queue takes significantly longer to setup, so we need to make
// sure that does not leak into the benchmark. Special considerations were given
// to:
//
// https://github.com/bheisler/criterion.rs/issues/475

pub const PARAM_CONFIGS: &[(usize, usize)] = &[(1, 100), (10, 100), (100, 100), (100, 10000)];

fn run_benchmark_lscq(
    context: Arc<UnboundedQueue<usize>>,
    threads: usize,
    ops: usize,
)
{
    let barrier = Arc::new(Barrier::new(threads));

    // let barrier_finalized = Arc::new(Barrier::new(THREADS + 1));
    let mut handles = vec![];
    for _ in 0..threads {
        handles.push(std::thread::spawn({
            // let barrier_finalized = barrier_finalized.clone();
            let context = context.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                let mut full = context.full_handle();
                for i in 0..ops {
                    full.enqueue(i);
                    // enqueue(&context, i);
                    // context.lock().unwrap().push_back(std::hint::black_box(i));
                }
                for _ in 0..ops {
                    std::hint::black_box(full.dequeue());
                    // dequeue(&context);
                    // context.lock().unwrap().pop_front();
                }
                // println!("Hitting the barrier...");
                // barrier_finalized.wait();
                // println!("Hit the barrier...");
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    // barrier_finalized.wait();
}

fn run_multithread_benchmark<C: Send + Sync>(
    context: C,
    enqueue: fn(&C, usize),
    dequeue: fn(&C),
    threads: usize,
    ops: usize,
) where
    C: 'static + Clone,
{
    let barrier = Arc::new(Barrier::new(threads));

    // let barrier_finalized = Arc::new(Barrier::new(THREADS + 1));
    let mut handles = vec![];
    for _ in 0..threads {
        handles.push(std::thread::spawn({
            // let barrier_finalized = barrier_finalized.clone();
            let context = context.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                for i in 0..ops {
                    enqueue(&context, i);
                    // context.lock().unwrap().push_back(std::hint::black_box(i));
                }
                for _ in 0..ops {
                    dequeue(&context);
                    // context.lock().unwrap().pop_front();
                }
                // println!("Hitting the barrier...");
                // barrier_finalized.wait();
                // println!("Hit the barrier...");
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }
    // barrier_finalized.wait();
}

fn configure_benchmark_raw<R, P, C>(
    c: &mut Criterion,
    name: &str,
    patterns: &[(usize, usize)],
    mut routine: R,
    main_rt: fn(C, P, usize, usize),
    data: P,
) where
    R: FnMut() -> C + Copy,
    C: Send + Sync + 'static + Clone,
    P: Copy,
{
    for (threads, ops) in patterns {
        c.bench_function(
            &format!("sync {name} enqueue-dequeue | threads={threads}, ops={ops}"),
            |b| {
                b.iter_custom(move |iters| {
                    let context = routine();
                    let instant = Instant::now();

                    for _ in 0..iters {
                        main_rt(context.clone(), data, *threads, *ops);
                        // run_multithread_benchmark(context.clone(), enqueue, dequeue, *threads, *ops);
                    }

                    let elapsed = instant.elapsed();

                    drop(context);

                    elapsed
                });
            },
        );
    }
}

fn configure_benchmark<R, C>(
    c: &mut Criterion,
    name: &str,
    patterns: &[(usize, usize)],
    routine: R,
    enqueue: fn(&C, usize),
    dequeue: fn(&C),
) where
    R: FnMut() -> C + Copy,
    C: Send + Sync + 'static + Clone,
{
    configure_benchmark_raw(
        c,
        name,
        patterns,
        routine,
        |context, (enqueue, dequeue), threads, ops| {
            run_multithread_benchmark(context, enqueue, dequeue, threads, ops);
        },
        (enqueue, dequeue),
    );
    // for (threads, ops) in patterns {
    //     c.bench_function(
    //         &format!("{name} enqueue-dequeue | threads={threads}, ops={ops}"),
    //         |b| {
    //             b.iter_custom(move |iters| {

    //                  let context = routine();
    //                 let instant = Instant::now();

    //                 for _ in 0..iters {
    //                     run_multithread_benchmark(context.clone(), enqueue, dequeue, *threads, *ops);

    //                 }

    //                 let elapsed = instant.elapsed();

    //                 drop(context);

    //                 elapsed
    //             });
    //         },
    //     );
    // }
}

fn bench_lockfree_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "lockfree",
        PARAM_CONFIGS,
        || Arc::new(lockfree::queue::Queue::new()),
        |queue, item| queue.push(item),
        |queue| {
            std::hint::black_box(queue.pop());
        },
    );
}

fn bench_mutex_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "mutex",
        PARAM_CONFIGS,
        || Arc::new(Mutex::new(VecDeque::new())),
        |queue, item| queue.lock().unwrap().push_back(item),
        |queue| {
            std::hint::black_box(queue.lock().unwrap().pop_front());
        },
    );
}


fn bench_alloc_bounded_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "alloc-bounded-lfqueue",
        PARAM_CONFIGS,
        || Arc::new(AllocBoundedQueue::new(1024)),
        |queue, item| {
            let _ = queue.enqueue(item);
        },
        |queue| {
            std::hint::black_box(queue.dequeue());
        },
    );
}

fn bench_lscq_queue(c: &mut Criterion) {
    configure_benchmark_raw(
        c,
        "unbounded-lfqueue",
        PARAM_CONFIGS,
        || Arc::new(UnboundedQueue::<usize>::with_segment_size(1024)),
        |context, _, threads, ops| {
            run_benchmark_lscq(context, threads, ops);
        },
        || (),
    );

    //     configure_benchmark_raw(
    //         c,
    //         "unbounded-lfqueue",
    //         PARAM_CONFIGS,
    //         || Arc::new(UnboundedQueue::with_segment_size(2048)),
    //         |queue, item|  {
    //             let _ = queue.enqueue(item);
    //         },
    //         |queue| {  std::hint::black_box(queue.dequeue()); }
    //     );
    // }
}
fn bench_const_bounded_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "const-bounded-lfqueue",
        PARAM_CONFIGS,
        || Arc::new(const_queue!(usize; 32)),
        |queue, item| {
            let _ = queue.enqueue(item);
        },
        |queue| {
            std::hint::black_box(queue.dequeue());
        },
    );
}

fn bench_crossbeam_seg_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "crossbeam-seg-queue",
        PARAM_CONFIGS,
        || Arc::new(crossbeam_queue::SegQueue::new()),
        |queue, item| {
            queue.push(item);
        },
        |queue| {
            std::hint::black_box(queue.pop());
        },
    );
}

fn bench_crossbeam_array_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "crossbeam-array-queue",
        PARAM_CONFIGS,
        || Arc::new(ArrayQueue::new(32)),
        |queue, item| {
            let _ = queue.push(item);
        },
        |queue| {
            std::hint::black_box(queue.pop());
        },
    );
}

criterion_group!(
    queues,
    bench_const_bounded_queue,
    bench_alloc_bounded_queue,
    bench_lscq_queue,
    bench_mutex_queue,
    bench_lockfree_queue,
    bench_crossbeam_array_queue,
    bench_crossbeam_seg_queue
);
criterion_main!(queues);
