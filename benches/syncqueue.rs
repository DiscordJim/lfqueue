use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lfqueue::{LcsqQueue, ScqQueue};
use lockfree::queue::Queue;
use std::collections::VecDeque;
use std::sync::{Arc, Barrier, Mutex};
use std::time::Instant;

// NOTE: Our queue takes significantly longer to setup, so we need to make
// sure that does not leak into the benchmark. Special considerations were given
// to:
//
// https://github.com/bheisler/criterion.rs/issues/475

// ===== Benchmarks =====

pub const PARAM_CONFIGS: &[(usize, usize)] = &[(1, 100), (10, 100), (100, 100), (100, 10000)];

// pub const THREADS: usize = 100;
// pub const THREAD_RUNS: usize = 10000;

// async fn do_crossbeam_queue() {
//     // let q = Arc::new(Scq::new());

//     let q = Arc::new(SegQueue::new());
//     let b = Arc::new(Barrier::new(THREADS));
//     for _ in 0..THREADS {
//         tokio::spawn({
//             let q = q.clone();
//             let b = b.clone();
//             async move {
//                 b.wait().await;
//                 for i in 0..THREAD_RUNS {
//                     q.push(black_box(i));
//                 }
//                 for _ in 0..THREAD_RUNS {
//                     let _ = q.pop();
//                 }
//             }
//         });
//     }

//     // run_test_harness(
//     //     1000,
//     //     q,
//     //     async |i| {
//     //         q.enqueue(i);
//     //     },
//     //     async || q.dequeue().await,
//     // )
//     // .await;
// }

// async fn do_scq_bench() {
//     // let q = Arc::new(Scq::new());

//     let q: Arc<Queue<usize>> = Arc::new(Queue::new());
//     let b = Arc::new(Barrier::new(THREADS));
//     for _ in 0..THREADS {
//         tokio::spawn({
//             let q = q.clone();
//             let b = b.clone();
//             async move {
//                 b.wait().await;
//                 for i in 0..THREAD_RUNS {
//                     q.push(black_box(i));
//                 }
//                 for _ in 0..THREAD_RUNS {
//                     let _ = q.pop();
//                 }
//             }
//         });
//     }

//     // run_test_harness(
//     //     1000,
//     //     q,
//     //     async |i| {
//     //         q.enqueue(i);
//     //     },
//     //     async || q.dequeue().await,
//     // )
//     // .await;
// }

// async fn do_true_scq_bench(q: Arc<ScqQueue<usize>>) {
//     // let q = Arc::new(Scq::new());

//     let b = Arc::new(Barrier::new(THREADS));
//     for _ in 0..THREADS {
//         tokio::spawn({
//             let q = q.clone();
//             let b = b.clone();
//             async move {
//                 b.wait().await;
//                 for i in 0..THREAD_RUNS {
//                     q.enqueue(black_box(i)).unwrap();
//                 }
//                 for _ in 0..THREAD_RUNS {
//                     let _ = q.dequeue();
//                 }
//             }
//         });
//     }

//     // run_test_harness(
//     //     1000,
//     //     q,
//     //     async |i| {
//     //         q.enqueue(i);
//     //     },
//     //     async || q.dequeue().await,
//     // )
//     // .await;
// }

fn run_multithread_benchmark<C: Send + Sync>(
    context: Arc<C>,
    enqueue: fn(&Arc<C>, usize),
    dequeue: fn(&Arc<C>),
    threads: usize,
    ops: usize,
) where
    C: 'static,
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

// fn get_runtime() -> Runtime {
//     Builder::new_multi_thread().build().unwrap()
// }

// fn bench_seg_queue(c: &mut Criterion) {
//     c.bench_function("seg enqueue-dequeue", |b| {
//         b.iter(|| );
//         b.to_async(get_runtime()).iter(|| do_crossbeam_queue());
//     });
// }

// fn bench_scq_queue(c: &mut Criterion) {
//     c.bench_function("scq enqueue-dequeue", |b| {
//         b.to_async(get_runtime()).iter_batched(|| {
//             let q: Arc<ScqQueue<usize>> = Arc::new(ScqQueue::new(16));
//             q
//         }, |q| {
//             do_true_scq_bench(q)
//         }, criterion::BatchSize::NumBatches(1));
//     });
// }

// fn configure_multithread_benchmark(
//     c: &mut Criterion,
//     name: &str,
//     enqueue: fn(&C, usize),
//     dequeue: fn(&C)
// ) {

// }

fn configure_benchmark<R, C>(
    c: &mut Criterion,
    name: &str,
    patterns: &[(usize, usize)],
    mut routine: R,
    enqueue: fn(&Arc<C>, usize),
    dequeue: fn(&Arc<C>),
) where
    R: FnMut() -> C + Copy,
    C: Send + Sync + 'static,
{
    for (threads, ops) in patterns {
        c.bench_function(
            &format!("{name} enqueue-dequeue | threads={threads}, ops={ops}"),
            |b| {
                b.iter_custom(move |iters| {

                     let context = Arc::new(routine());

                    //  println!("iters: {iters}");
                    let instant = Instant::now();

                    for _ in 0..iters {
                        // println!("iters: {iters}");
                        run_multithread_benchmark(context.clone(), enqueue, dequeue, *threads, *ops);

                    }
                   
                    

                    
                    let elapsed = instant.elapsed();

                    drop(context);

                    elapsed
                });
                // b.iter_batched(
                //     || Arc::new(routine()),
                //     |context| run_multithread_benchmark(context, enqueue, dequeue, *threads, *ops),
                //     BatchSize::NumBatches(1),
                // );
            },
        );
    }
}

fn bench_lockfree_queue(c: &mut Criterion) {
    // let q = Arc::new(Mutex::new(VecDeque::new()));
    configure_benchmark(
        c,
        "lockfree",
        PARAM_CONFIGS,
        || lockfree::queue::Queue::new(),
        |queue, item| queue.push(item),
        |queue| {  std::hint::black_box(queue.pop()); }
    );

    // c.bench_function("lockfree enqueue-dequeue", |b| {
    //     b.iter_batched(
    //         || Arc::new(lockfree::queue::Queue::new()),
    //         |context| {
    //             run_multithread_benchmark(
    //                 context,
    //                 |queue, item| queue.push(item),
    //                 |queue| {
    //                     std::hint::black_box(queue.pop());
    //                 },
    //             )
    //         },
    //         BatchSize::NumBatches(1),
    //     );
    // });
    
}

fn bench_mutex_queue(c: &mut Criterion) {
    // let q = Arc::new(Mutex::new(VecDeque::new()));
    configure_benchmark(
        c,
        "mutex",
        PARAM_CONFIGS,
        || Mutex::new(VecDeque::new()),
        |queue, item| queue.lock().unwrap().push_back(item),
        |queue| {  std::hint::black_box(queue.lock().unwrap().pop_front()); }
    );
}

fn bench_lscq_queue(c: &mut Criterion) {

    configure_benchmark(
        c,
        "scqring",
        PARAM_CONFIGS,
        || ScqQueue::new(4),
        |queue, item|  {
            let _ = queue.enqueue(item);
        },
        |queue| {  std::hint::black_box(queue.dequeue()); }
    );
}

criterion_group!(
    queues,
    bench_lscq_queue,
    // bench_mutex_queue,
    // bench_lockfree_queue
);
criterion_main!(queues);
