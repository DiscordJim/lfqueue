//! Many-CPU benchmarks for queue implementations using the many_cpus_benchmarking framework.
//!
//! This benchmark suite provides a many-CPU variant of the original syncqueue.rs benchmarks,
//! designed to measure how queue performance is affected by memory locality and processor
//! distribution in multi-memory-region systems when used for their intended purpose:
//! **producer-consumer communication**.
//!
//! ## Design Philosophy
//!
//! Queues are fundamentally about inter-thread communication between producers and consumers.
//! This benchmark implements a true producer-consumer pattern where:
//!
//! 1. **Producer worker** - Enqueues items into the shared queue
//! 2. **Consumer worker** - Dequeues items from the shared queue  
//! 3. **Shared queue instance** - The communication channel between the two workers
//!
//! This follows the "different actions" pattern from the many_cpus_benchmarking framework,
//! allowing us to measure how memory locality affects inter-thread queue communication
//! performance. **Importantly, we exclude "self" distribution modes** (PinnedSelf, UnpinnedSelf, 
//! UnpinnedPerMemoryRegionSelf) because these would prevent true concurrent producer-consumer 
//! operation, leading to deadlock scenarios where the consumer waits indefinitely for items 
//! that the producer cannot produce due to lack of concurrent execution.
//!
//! ## Memory Locality Effects
//!
//! The benchmark measures several important scenarios:
//! - **Same memory region**: Producer and consumer on processors sharing the same memory
//! - **Different memory regions**: Producer and consumer on processors in different memory regions
//! - **Payload exchange modes**: How performance changes when the queue is allocated in the
//!   producer's vs consumer's memory region
//!
//! ## Queue Configuration
//!
//! Queue sizes are set to handle the communication load while matching original benchmark intentions:
//! - AllocBoundedQueue: 2048 (sufficient buffer for producer-consumer communication)
//! - ConstBoundedQueue: 256 (larger than original to handle async producer-consumer pattern)
//! - ArrayQueue: 256 (sufficient buffer for communication)
//! - UnboundedQueue: 1024 segment size (matches original)
//!
//! ## Operation Pattern
//!
//! Uses realistic producer-consumer communication with sufficient operations to measure
//! memory locality effects clearly, with a moderate multiplier to amortize framework overhead.
//!
//! ## Key Insights This Benchmark Measures
//!
//! 1. **Queue allocation locality**: How performance changes when the shared queue is allocated
//!    in the producer's vs consumer's memory region (via payload exchange modes)
//! 2. **Cross-memory-region communication**: Performance differences when producer and consumer
//!    are in the same vs different memory regions  
//! 3. **True queue contention**: Real producer-consumer contention patterns rather than artificial
//!    single-threaded enqueue-all/dequeue-all sequences
//! 4. **Implementation comparison**: How different queue implementations handle the memory locality
//!    challenges of multi-memory-region systems
//! 5. **Concurrent operation requirement**: Excludes "self" distribution modes that would prevent
//!    simultaneous producer-consumer operation, focusing only on truly concurrent scenarios

use criterion::{Criterion, criterion_group, criterion_main};
use crossbeam_queue::ArrayQueue;
use lfqueue::{AllocBoundedQueue, ConstBoundedQueue, UnboundedQueue, const_queue};
use many_cpus_benchmarking::{Payload, WorkDistribution, execute_runs};
use std::collections::VecDeque;
use std::hint::black_box;
use std::sync::{Arc, Mutex};

/// Number of operations each worker performs in the benchmark
/// Producer sends this many items, consumer receives this many items
const OPERATIONS_PER_WORKER: usize = 5000;

/// Payload for AllocBoundedQueue benchmarking - Producer-Consumer pattern
struct AllocBoundedQueuePayload {
    queue: Arc<AllocBoundedQueue<usize>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for AllocBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared queue instance for producer-consumer communication
        let shared_queue = Arc::new(AllocBoundedQueue::new(2048));
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state and avoid initial blocking
        if self.is_producer {
            for i in 0..1000 {
                let _ = self.queue.enqueue(i);
            }
        }
    }
    
    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                // Keep trying until successful (bounded queue may be full)
                while queue.enqueue(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = queue.dequeue() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for UnboundedQueue benchmarking - Producer-Consumer pattern
struct UnboundedQueuePayload {
    queue: Arc<UnboundedQueue<usize>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for UnboundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared queue instance for producer-consumer communication
        let shared_queue = Arc::new(UnboundedQueue::with_segment_size(1024));
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            let mut handle = self.queue.full_handle();
            for i in 0..1000 {
                handle.enqueue(i);
            }
        }
    }

    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            let mut full_handle = queue.full_handle();
            for i in 0..self.operations {
                full_handle.enqueue(i);
            }
        } else {
            // Consumer: continuously dequeue items
            let mut full_handle = queue.full_handle();
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = full_handle.dequeue() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for ConstBoundedQueue benchmarking - Producer-Consumer pattern
struct ConstBoundedQueuePayload {
    queue: Arc<ConstBoundedQueue<usize, 512>>, // Larger size for producer-consumer pattern
    operations: usize,
    is_producer: bool,
}

impl Payload for ConstBoundedQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared queue instance for producer-consumer communication
        let shared_queue = Arc::new(const_queue!(usize; 256)); // Larger for async communication
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            for i in 0..100 {
                let _ = self.queue.enqueue(i);
            }
        }
    }
    
    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                // Keep trying until successful (bounded queue may be full)
                while queue.enqueue(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = queue.dequeue() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for Mutex<VecDeque> benchmarking - Producer-Consumer pattern
struct MutexQueuePayload {
    queue: Arc<Mutex<VecDeque<usize>>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for MutexQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared mutex-protected queue for producer-consumer communication
        let shared_queue = Arc::new(Mutex::new(VecDeque::new()));
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            let mut queue = self.queue.lock().unwrap();
            for i in 0..1000 {
                queue.push_back(i);
            }
        }
    }

    fn process(&mut self) {
        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                self.queue.lock().unwrap().push_back(i);
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    let mut queue = self.queue.lock().unwrap();
                    if let Some(value) = queue.pop_front() {
                        drop(queue); // Release lock before black_box
                        black_box(value);
                        break;
                    }
                    drop(queue); // Release lock before spin
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for lockfree::queue::Queue benchmarking - Producer-Consumer pattern
struct LockfreeQueuePayload {
    queue: Arc<lockfree::queue::Queue<usize>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for LockfreeQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared lockfree queue for producer-consumer communication
        let shared_queue = Arc::new(lockfree::queue::Queue::new());
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            for i in 0..1000 {
                self.queue.push(i);
            }
        }
    }

    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                queue.push(i);
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = queue.pop() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for crossbeam SegQueue benchmarking - Producer-Consumer pattern
struct CrossbeamSegQueuePayload {
    queue: Arc<crossbeam_queue::SegQueue<usize>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for CrossbeamSegQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared crossbeam SegQueue for producer-consumer communication
        let shared_queue = Arc::new(crossbeam_queue::SegQueue::new());
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            for i in 0..1000 {
                self.queue.push(i);
            }
        }
    }
    
    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                queue.push(i);
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = queue.pop() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

/// Payload for crossbeam ArrayQueue benchmarking - Producer-Consumer pattern
struct CrossbeamArrayQueuePayload {
    queue: Arc<ArrayQueue<usize>>,
    operations: usize,
    is_producer: bool,
}

impl Payload for CrossbeamArrayQueuePayload {
    fn new_pair() -> (Self, Self) {
        // Shared crossbeam ArrayQueue for producer-consumer communication
        let shared_queue = Arc::new(ArrayQueue::new(256)); // Larger for async communication
        
        let producer = Self {
            queue: shared_queue.clone(),
            operations: OPERATIONS_PER_WORKER,
            is_producer: true,
        };
        
        let consumer = Self {
            queue: shared_queue,
            operations: OPERATIONS_PER_WORKER,
            is_producer: false,
        };
        
        (producer, consumer)
    }

    fn prepare(&mut self) {
        // Pre-populate the queue to establish steady state
        if self.is_producer {
            for i in 0..100 {
                let _ = self.queue.push(i);
            }
        }
    }

    fn process(&mut self) {
        let queue = &self.queue;

        if self.is_producer {
            // Producer: continuously enqueue items
            for i in 0..self.operations {
                // Keep trying until successful (bounded queue may be full)
                while queue.push(i).is_err() {
                    std::hint::spin_loop();
                }
            }
        } else {
            // Consumer: continuously dequeue items
            for _ in 0..self.operations {
                // Keep trying until successful (queue may be empty)
                loop {
                    if let Some(value) = queue.pop() {
                        black_box(value);
                        break;
                    }
                    std::hint::spin_loop();
                }
            }
        }
    }
}

// Benchmark functions implementing producer-consumer patterns

/// Producer-Consumer benchmark for AllocBoundedQueue
fn bench_alloc_bounded_queue_many_cpus(c: &mut Criterion) {
    // Different actions (producer vs consumer), but exclude "self" modes to avoid deadlock
    // Self modes would prevent concurrent producer-consumer operation leading to deadlock
    execute_runs::<AllocBoundedQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for UnboundedQueue
fn bench_unbounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<UnboundedQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for ConstBoundedQueue
fn bench_const_bounded_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<ConstBoundedQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for Mutex<VecDeque>
fn bench_mutex_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<MutexQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for lockfree::queue::Queue
fn bench_lockfree_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<LockfreeQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for crossbeam SegQueue
fn bench_crossbeam_seg_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamSegQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

/// Producer-Consumer benchmark for crossbeam ArrayQueue
fn bench_crossbeam_array_queue_many_cpus(c: &mut Criterion) {
    execute_runs::<CrossbeamArrayQueuePayload, 5>(c, WorkDistribution::all_with_unique_processors_without_self());
}

criterion_group!(
    syncqueue_many_cpus_benchmarks,
    // Producer-Consumer benchmarks for all queue implementations
    bench_alloc_bounded_queue_many_cpus,
    bench_const_bounded_queue_many_cpus,
    bench_unbounded_queue_many_cpus,
    bench_mutex_queue_many_cpus,
    bench_lockfree_queue_many_cpus,
    bench_crossbeam_array_queue_many_cpus,
    bench_crossbeam_seg_queue_many_cpus,
);

criterion_main!(syncqueue_many_cpus_benchmarks);
