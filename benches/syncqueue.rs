use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use lfqueue::{AllocBoundedQueue, BoundedQueue, UnboundedQueue};
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
                    let instant = Instant::now();

                    for _ in 0..iters {
                        run_multithread_benchmark(context.clone(), enqueue, dequeue, *threads, *ops);

                    }
                   
                    

                    
                    let elapsed = instant.elapsed();

                    drop(context);

                    elapsed
                });
            },
        );
    }
}

fn bench_lockfree_queue(c: &mut Criterion) {
    configure_benchmark(
        c,
        "lockfree",
        PARAM_CONFIGS,
        || lockfree::queue::Queue::new(),
        |queue, item| queue.push(item),
        |queue| {  std::hint::black_box(queue.pop()); }
    );

}

fn bench_mutex_queue(c: &mut Criterion) {
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
        || AllocBoundedQueue::new(4),
        |queue, item|  {
            let _ = queue.enqueue(item);
        },
        |queue| {  std::hint::black_box(queue.dequeue()); }
    );
}

criterion_group!(
    queues,
    bench_lscq_queue,
    bench_mutex_queue,
    bench_lockfree_queue
);
criterion_main!(queues);
