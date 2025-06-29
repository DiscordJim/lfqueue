use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_queue::ArrayQueue;
use lfqueue::{AllocBoundedQueue, ConstBoundedQueue, UnboundedQueue, const_queue};
use many_cpus_benchmarking::{execute_runs, Payload, WorkDistribution};
use std::collections::VecDeque;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

// Configuration patterns matching the original PARAM_CONFIGS
// We scale down the operations significantly for the many_cpus_benchmarking framework
const SMALL_CONFIG: (usize, usize) = (1, 10);     // Original: (1, 100)
const MEDIUM_CONFIG: (usize, usize) = (10, 10);   // Original: (10, 100)
const LARGE_CONFIG: (usize, usize) = (100, 10);   // Original: (100, 100)
const XLARGE_CONFIG: (usize, usize) = (100, 100); // Original: (100, 10000)

/// Generic payload for benchmarking different queue types
/// Each worker creates its own queue and performs enqueue-dequeue cycles
/// This pattern matches the original syncqueue.rs behavior where each thread
/// does enqueue operations followed by dequeue operations
#[derive(Debug)]
struct QueuePayload<Q> {
    queue: Option<Arc<Q>>,
    operations: usize,
    queue_factory: fn() -> Q,
}

impl<Q> QueuePayload<Q> {
    fn new_with_factory(operations: usize, factory: fn() -> Q) -> (Self, Self) {
        (
            Self {
                queue: None,
                operations,
                queue_factory: factory,
            },
            Self {
                queue: None,
                operations,
                queue_factory: factory,
            },
        )
    }
}

/// Payload for AllocBoundedQueue benchmarking
struct AllocBoundedQueuePayload {
    queue: Option<Arc<AllocBoundedQueue<usize>>>,
    operations: usize,
    queue_size: usize,
}

impl Payload for AllocBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 10, // Default small operations
                queue_size: 1024,
            },
            Self {
                queue: None,
                operations: 10,
                queue_size: 1024,
            },
        )
    }

    fn prepare(&mut self) {
        // Each worker creates its own queue in its memory region
        self.queue = Some(Arc::new(AllocBoundedQueue::new(self.queue_size)));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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
    queue: Option<Arc<UnboundedQueue<usize>>>,
    operations: usize,
    segment_size: usize,
}

impl Payload for UnboundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 10,
                segment_size: 1024,
            },
            Self {
                queue: None,
                operations: 10,
                segment_size: 1024,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(UnboundedQueue::with_segment_size(self.segment_size)));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
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
    queue: Option<Arc<ConstBoundedQueue<usize, 64>>>, // Using 64 to match const_queue!(usize; 32) -> 64 size
    operations: usize,
}

impl Payload for ConstBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 5, // Smaller for const queue
            },
            Self {
                queue: None,
                operations: 5,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(const_queue!(usize; 32)));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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
    queue: Option<Arc<Mutex<VecDeque<usize>>>>,
    operations: usize,
}

impl Payload for MutexQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 10,
            },
            Self {
                queue: None,
                operations: 10,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(Mutex::new(VecDeque::new())));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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
    queue: Option<Arc<lockfree::queue::Queue<usize>>>,
    operations: usize,
}

impl Payload for LockfreeQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 10,
            },
            Self {
                queue: None,
                operations: 10,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(lockfree::queue::Queue::new()));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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
    queue: Option<Arc<crossbeam_queue::SegQueue<usize>>>,
    operations: usize,
}

impl Payload for CrossbeamSegQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 10,
            },
            Self {
                queue: None,
                operations: 10,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(crossbeam_queue::SegQueue::new()));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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
    queue: Option<Arc<ArrayQueue<usize>>>,
    operations: usize,
}

impl Payload for CrossbeamArrayQueuePayload {
    fn new_pair() -> (Self, Self) {
        (
            Self {
                queue: None,
                operations: 5, // Smaller for bounded array queue
            },
            Self {
                queue: None,
                operations: 5,
            },
        )
    }

    fn prepare(&mut self) {
        self.queue = Some(Arc::new(ArrayQueue::new(32)));
    }

    fn process(&mut self) {
        let queue = self.queue.as_ref().expect("Queue should be initialized");
        
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

// Helper function to create different operation counts for different configurations
impl AllocBoundedQueuePayload {
    fn with_config(config: (usize, usize)) -> (Self, Self) {
        let (_, ops) = config;
        let scaled_ops = std::cmp::max(1, ops / 10); // Scale down operations
        
        (
            Self {
                queue: None,
                operations: scaled_ops,
                queue_size: 1024,
            },
            Self {
                queue: None,
                operations: scaled_ops,
                queue_size: 1024,
            },
        )
    }
}

// Similar helper implementations for other payload types
impl UnboundedQueuePayload {
    fn with_config(config: (usize, usize)) -> (Self, Self) {
        let (_, ops) = config;
        let scaled_ops = std::cmp::max(1, ops / 10);
        
        (
            Self {
                queue: None,
                operations: scaled_ops,
                segment_size: 1024,
            },
            Self {
                queue: None,
                operations: scaled_ops,
                segment_size: 1024,
            },
        )
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
