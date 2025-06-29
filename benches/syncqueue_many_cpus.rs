use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_queue::ArrayQueue;
use lfqueue::{AllocBoundedQueue, ConstBoundedQueue, UnboundedQueue, const_queue};
use many_cpus_benchmarking::{Payload, WorkDistribution, execute_runs};
use std::collections::VecDeque;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

/// Number of operations each worker performs in the benchmark
const OPERATIONS_PER_WORKER: usize = 100;

/// Payload for AllocBoundedQueue benchmarking
struct AllocBoundedQueuePayload {
    queue: Arc<AllocBoundedQueue<usize>>,
    operations: usize,
}

impl Payload for AllocBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Increase queue size to accommodate more operations from multiple workers
        let shared_queue = Arc::new(AllocBoundedQueue::new(2048));
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }
    fn process(&mut self) {
        let queue = &self.queue;

        // Pattern from original: enqueue all items, then dequeue all items
        // Enqueue phase
        for i in 0..self.operations {
            let _ = queue.enqueue(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.dequeue());
        }
    }
}

/// Payload for UnboundedQueue benchmarking (matches bench_lscq_queue)
struct UnboundedQueuePayload {
    queue: Arc<UnboundedQueue<usize>>,
    operations: usize,
}

impl Payload for UnboundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        let shared_queue = Arc::new(UnboundedQueue::with_segment_size(1024));
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }

    fn process(&mut self) {
        let queue = &self.queue;
        let mut full_handle = queue.full_handle();

        // Pattern from original run_benchmark_lscq: enqueue all, then dequeue all
        for i in 0..self.operations {
            full_handle.enqueue(i);
        }

        for _ in 0..self.operations {
            black_box(full_handle.dequeue());
        }
    }
}

/// Payload for ConstBoundedQueue benchmarking
struct ConstBoundedQueuePayload {
    queue: Arc<ConstBoundedQueue<usize, 512>>, // Updated to match const_queue!(usize; 256) -> 512 size
    operations: usize,
}

impl Payload for ConstBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        #[allow(clippy::unused_unit)]
        // Increase const queue size to accommodate more operations
        let shared_queue = Arc::new(const_queue!(usize; 256));
        
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }
    fn process(&mut self) {
        let queue = &self.queue;

        // Enqueue phase
        for i in 0..self.operations {
            let _ = queue.enqueue(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.dequeue());
        }
    }
}

/// Payload for Mutex<VecDeque> benchmarking
struct MutexQueuePayload {
    queue: Arc<Mutex<VecDeque<usize>>>,
    operations: usize,
}

impl Payload for MutexQueuePayload {
    fn new_pair() -> (Self, Self) {
        let shared_queue = Arc::new(Mutex::new(VecDeque::new()));
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }

    fn process(&mut self) {
        let queue = &self.queue;

        // Enqueue phase
        for i in 0..self.operations {
            queue.lock().unwrap().push_back(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.lock().unwrap().pop_front());
        }
    }
}

/// Payload for lockfree::queue::Queue benchmarking
struct LockfreeQueuePayload {
    queue: Arc<lockfree::queue::Queue<usize>>,
    operations: usize,
}

impl Payload for LockfreeQueuePayload {
    fn new_pair() -> (Self, Self) {
        let shared_queue = Arc::new(lockfree::queue::Queue::new());
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }

    fn process(&mut self) {
        let queue = &self.queue;

        // Enqueue phase
        for i in 0..self.operations {
            queue.push(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.pop());
        }
    }
}

/// Payload for crossbeam SegQueue benchmarking
struct CrossbeamSegQueuePayload {
    queue: Arc<crossbeam_queue::SegQueue<usize>>,
    operations: usize,
}

impl Payload for CrossbeamSegQueuePayload {
    fn new_pair() -> (Self, Self) {
        let shared_queue = Arc::new(crossbeam_queue::SegQueue::new());
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }
    fn process(&mut self) {
        let queue = &self.queue;

        // Enqueue phase
        for i in 0..self.operations {
            queue.push(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.pop());
        }
    }
}

/// Payload for crossbeam ArrayQueue benchmarking
struct CrossbeamArrayQueuePayload {
    queue: Arc<ArrayQueue<usize>>,
    operations: usize,
}

impl Payload for CrossbeamArrayQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Increase array queue size to accommodate more operations
        let shared_queue = Arc::new(ArrayQueue::new(256));
        (
            Self {
                queue: shared_queue.clone(),
                operations: OPERATIONS_PER_WORKER,
            },
            Self {
                queue: shared_queue,
                operations: OPERATIONS_PER_WORKER,
            },
        )
    }

    fn prepare(&mut self) {
        // Queue is already created and shared between workers
    }

    fn process(&mut self) {
        let queue = &self.queue;

        // Enqueue phase
        for i in 0..self.operations {
            let _ = queue.push(i);
        }

        // Dequeue phase
        for _ in 0..self.operations {
            black_box(queue.pop());
        }
    }
}

// Benchmark functions equivalent to the original syncqueue.rs

/// Equivalent to bench_alloc_bounded_queue
fn bench_alloc_bounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<AllocBoundedQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_lscq_queue (UnboundedQueue)
fn bench_unbounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<UnboundedQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_const_bounded_queue
fn bench_const_bounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<ConstBoundedQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_mutex_queue
fn bench_mutex_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<MutexQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_lockfree_queue
fn bench_lockfree_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<LockfreeQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_crossbeam_seg_queue
fn bench_crossbeam_seg_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamSegQueuePayload, 1>(c, WorkDistribution::all());
}

/// Equivalent to bench_crossbeam_array_queue
fn bench_crossbeam_array_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamArrayQueuePayload, 1>(c, WorkDistribution::all());
}

criterion_group!(
    syncqueue_many_cpus_benchmarks,
    // Main queue benchmarks (equivalent to the active ones in original)
    bench_alloc_bounded_queue_many_cpus,
    // All the commented-out benchmarks from original (now uncommented)
    bench_const_bounded_queue_many_cpus,
    bench_unbounded_queue_many_cpus,
    bench_mutex_queue_many_cpus,
    bench_lockfree_queue_many_cpus,
    bench_crossbeam_array_queue_many_cpus,
    bench_crossbeam_seg_queue_many_cpus,
);

criterion_main!(syncqueue_many_cpus_benchmarks);
