use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_queue::ArrayQueue;
use lfqueue::{AllocBoundedQueue, ConstBoundedQueue, UnboundedQueue, const_queue};
use many_cpus_benchmarking::{execute_runs, Payload, WorkDistribution};
use std::collections::VecDeque;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

/// Payload for AllocBoundedQueue benchmarking
struct AllocBoundedQueuePayload {
    queue: Arc<AllocBoundedQueue<usize>>,
    operations: usize,
}

impl Payload for AllocBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        let shared_queue = Arc::new(AllocBoundedQueue::new(1024));
        (
            Self {
                queue: shared_queue.clone(),
                operations: 10, // Default small operations
            },
            Self {
                queue: shared_queue,
                operations: 10,
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
            let mut attempts = 0;
            while queue.enqueue(i).is_err() && attempts < 50 {
                std::thread::yield_now();
                attempts += 1;
            }
        }
        
        // Dequeue phase
        for _ in 0..self.operations {
            let mut attempts = 0;
            while queue.dequeue().is_none() && attempts < 50 {
                std::thread::yield_now();
                attempts += 1;
            }
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
                operations: 10,
            },
            Self {
                queue: shared_queue,
                operations: 10,
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
    queue: Arc<ConstBoundedQueue<usize, 64>>, // Using 64 to match const_queue!(usize; 32) -> 64 size
    operations: usize,
}

impl Payload for ConstBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        #[allow(clippy::unused_unit)]
        let shared_queue = Arc::new(const_queue!(usize; 32));
        
        (
            Self {
                queue: shared_queue.clone(),
                operations: 5, // Smaller for const queue
            },
            Self {
                queue: shared_queue,
                operations: 5,
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
            let mut attempts = 0;
            while queue.enqueue(i).is_err() && attempts < 20 {
                std::thread::yield_now();
                attempts += 1;
            }
        }
        
        // Dequeue phase
        for _ in 0..self.operations {
            let mut attempts = 0;
            while queue.dequeue().is_none() && attempts < 20 {
                std::thread::yield_now();
                attempts += 1;
            }
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
                operations: 10,
            },
            Self {
                queue: shared_queue,
                operations: 10,
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
                operations: 10,
            },
            Self {
                queue: shared_queue,
                operations: 10,
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
                operations: 10,
            },
            Self {
                queue: shared_queue,
                operations: 10,
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
        let shared_queue = Arc::new(ArrayQueue::new(32));
        (
            Self {
                queue: shared_queue.clone(),
                operations: 5, // Smaller for bounded array queue
            },
            Self {
                queue: shared_queue,
                operations: 5,
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
            let mut attempts = 0;
            while queue.push(i).is_err() && attempts < 20 {
                std::thread::yield_now();
                attempts += 1;
            }
        }
        
        // Dequeue phase
        for _ in 0..self.operations {
            let mut attempts = 0;
            while queue.pop().is_none() && attempts < 20 {
                std::thread::yield_now();
                attempts += 1;
            }
        }
    }
}

// Benchmark functions equivalent to the original syncqueue.rs

/// Equivalent to bench_alloc_bounded_queue
fn bench_alloc_bounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<AllocBoundedQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_lscq_queue (UnboundedQueue)
fn bench_unbounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<UnboundedQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_const_bounded_queue
fn bench_const_bounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<ConstBoundedQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_mutex_queue
fn bench_mutex_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<MutexQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_lockfree_queue
fn bench_lockfree_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<LockfreeQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_crossbeam_seg_queue
fn bench_crossbeam_seg_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamSegQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Equivalent to bench_crossbeam_array_queue
fn bench_crossbeam_array_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamArrayQueuePayload, 1>(
        c,
        WorkDistribution::all_with_unique_processors_without_self()
    );
}

/// Memory region focused benchmarks - tests the most important memory effects
fn bench_memory_region_comparison(c: &mut Criterion) {
    // Compare key memory region effects for the main queue types
    execute_runs::<AllocBoundedQueuePayload, 1>(c, &[
        WorkDistribution::PinnedMemoryRegionPairs,   // Cross-memory-region
        WorkDistribution::PinnedSameMemoryRegion,    // Same memory region
    ]);
    
    execute_runs::<UnboundedQueuePayload, 1>(c, &[
        WorkDistribution::PinnedMemoryRegionPairs,
        WorkDistribution::PinnedSameMemoryRegion,
    ]);
}

/// High-throughput tests with payload multipliers
fn bench_high_throughput_many_cpus(c: &mut Criterion) {
    // Test with higher payload multipliers to reduce harness overhead
    execute_runs::<AllocBoundedQueuePayload, 4>(
        c,
        &[WorkDistribution::PinnedMemoryRegionPairs]
    );
    
    execute_runs::<UnboundedQueuePayload, 4>(
        c,
        &[WorkDistribution::PinnedMemoryRegionPairs]
    );
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
    
    // Additional memory-region focused tests
    bench_memory_region_comparison,
    bench_high_throughput_many_cpus,
);

criterion_main!(syncqueue_many_cpus_benchmarks);
