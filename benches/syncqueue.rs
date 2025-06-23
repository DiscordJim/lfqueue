use criterion::{BatchSize, BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use lfqueue::LcsqQueue;
use lockfree::queue::Queue;
use std::collections::VecDeque;
use std::sync::{Arc, Barrier, Mutex};

// ===== Benchmarks =====

pub const THREADS: usize = 5;
pub const THREAD_RUNS: usize = 10;

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
) where
    C: 'static,
{
    let barrier = Arc::new(Barrier::new(THREADS));

    // let barrier_finalized = Arc::new(Barrier::new(THREADS + 1));
    let mut handles = vec![];
    for _ in 0..THREADS {
        handles.push(std::thread::spawn({
            // let barrier_finalized = barrier_finalized.clone();
            let context = context.clone();
            let barrier = barrier.clone();
            move || {
                barrier.wait();
                for i in 0..THREAD_RUNS {
                    enqueue(&context, i);
                    // context.lock().unwrap().push_back(std::hint::black_box(i));
                }
                for _ in 0..THREAD_RUNS {
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


fn bench_mutex_queue(c: &mut Criterion) {
    // let q = Arc::new(Mutex::new(VecDeque::new()));
    c.bench_function("mutex enqueue-dequeue", |b| {
        b.iter_batched(
            || Arc::new(Mutex::new(VecDeque::new())),
            |context| {
                run_multithread_benchmark(
                    context,
                    |queue, item| queue.lock().unwrap().push_back(item),
                    |queue| {
                        std::hint::black_box(queue.lock().unwrap().pop_front());
                    },
                )
            },
            BatchSize::NumBatches(1),
        );
    });
}

fn bench_lscq_queue(c: &mut Criterion) {
    // let q = Arc::new(Mutex::new(VecDeque::new()));
    c.bench_function("lcsq multithread enqueue-dequeue", |b| {
        b.iter_batched(
            || Arc::new(LcsqQueue::new(3)),
            |context| {
                run_multithread_benchmark(
                    context,
                    |queue, item| {
                        // println!("Started queue...");
                         queue.enqueue(item);
                        //  println!("Done queue...");
                    },
                    |queue| {
                        std::hint::black_box(queue.dequeue());
                    },
                )
            },
            BatchSize::NumBatches(1),
        );
    });
}

criterion_group!(queues, bench_lscq_queue);
criterion_main!(queues);
