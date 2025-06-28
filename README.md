
![lfqueue](assets/lfqueue.png)

<div align="center">

[![GitHub Actions Status](https://github.com/DiscordJim/lfqueue/actions/workflows/rust.yml/badge.svg)](https://github.com/DiscordJim/lfqueue/actions) · [![Crates.io Version](https://img.shields.io/crates/v/lfqueue.svg)](https://crates.io/crates/lfqueue) · [![Crates.io Downloads](https://img.shields.io/crates/d/lfqueue.svg)](https://crates.io/crates/lfqueue) · [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/DiscordJim/lfqueue/blob/main/LICENSE)

</div>

A lock-free queue for asynchronous &amp; synchronous code from the ACM paper, _"A Scalable, Portable, and Memory-Efficient Lock-Free FIFO Queue"_ by Ruslan Nikolaev ([arXiV/1908.04511](https://arxiv.org/abs/1908.04511)). This implements the proposed SCQ (bounded) and LSCQ (unbounded) variant of the queue using atomics.

# Features
- `#![no_std]` option for embedded environments
- bounded & unbounded MPMC concurrent queues
- very fast performance characteristics under high & low contentions.

# Quickstart
The following section will give quickstart code examples. One limitation of the library is that sizes must be powers of two, therefore, only 1, 2, 4, 8, 16, ... are supported as lengths.

## Allocated Queues
Usually what you want is an allocated queue, which means that the values are all on the heap. There are two types: `AllocBoundedQueue`, which is the bounded queue but heap-allocated, and the `UnboundedQueue` which is _always_ heap allocated.
```rust
// Import the allocated quuees.
use lfqueue::AllocBoundedQueue;

// Make an allocated queue of size 8.
let queue = AllocBoundedQueue::new(8);
assert!(queue.enqueue(0).is_ok()); // this should enqueue correctly.
assert_eq!(queue.dequeue(), Some(0));
```
You may also want an `UnboundedQueue`, which requires `std` which is enabled by default. These are created with an initial segment size:
```rust
use lfqueue::UnboundedQueue;

// Make an unbounded queue.
let queue = UnboundedQueue::new(8);
queue.enqueue(0);
assert_eq!(queue.dequeue(), Some(0));
```

## Constant Queues
This queue is not backed by the heap and instead lives on the stack. These can be created manually, but we almost always want to use the macro which will set it up easily.
```rust
use lfqueue::{const_queue, ConstBoundedQueue};

// Make a constant queue of size 8.
let queue = const_queue!(usize; 8);
assert!(queue.enqueue(8).is_ok());
assert_eq!(queue.dequeue(), Some(8));
```

# Benchmarks
The queues within the library were tested against several other queues. The benchmarking is not exhaustive, but the process can be seen in `benches/syncqueue.rs`.


| crate| structure | test | time (ms) |
| ---- | ---- | ---- | ---- |
| lfqueue | `ConstBoundedQueue` (32) | t=1,o=100 | 99.721µs |
| lfqueue | `ConstBoundedQueue` (32) | t=10,o=100 | 879.07 µs |
| lfqueue | `ConstBoundedQueue` (32) | t=100,o=100 | 16.303 ms |
| lfqueue | `ConstBoundedQueue` (32) | t=100,o=10000 | 129.34 ms |
| lfqueue | `AllocBoundedQueue` (32) | t=1,o=100 | 101.92µs |
| lfqueue | `AllocBoundedQueue` (32) | t=10,o=100 | 912.69µs |
| lfqueue | `AllocBoundedQueue` (32) | t=100,o=100 | 11.239ms |
| lfqueue | `AllocBoundedQueue` (32) | t=100,o=10000 | 114.89ms |
| lfqueue | `UnboundedQueue` (seg=1024) | t=1,o=100 |  107.90µs |
| lfqueue | `UnboundedQueue` (seg=1024) | t=10,o=100 | 882.55µs |
| lfqueue | `UnboundedQueue` (seg=1024) | t=100,o=100 |  11.888ms |
| lfqueue | `UnboundedQueue` (seg=1024) | t=100,o=10000 | 144.09ms |
| crossbeam | `SegQueue` | t=1,o=100 | 111.29µs |
| crossbeam | `SegQueue` | t=10,o=100 | 995.10µs |
| crossbeam | `SegQueue` | t=100,o=100 | 20.831ms |
| crossbeam | `SegQueue` | t=100,o=10000 | 41.886ms |
| crossbeam | `ArrayQueue` (32) | t=1,o=100 | 155.42µs |
| crossbeam | `ArrayQueue` (32) | t=10,o=100 |  939.54µs |
| crossbeam | `ArrayQueue` (32) | t=100,o=100 | 11.161ms |
| crossbeam | `ArrayQueue` (32) | t=100,o=10000 | 99.484ms |
| lockfree | `Queue` | t=1,o=100 | 114.87µs |
| lockfree | `Queue` | t=10,o=100 | 1.0594ms |
| lockfree | `Queue` | t=100,o=100 | 13.756ms |
| lockfree | `Queue` | t=100,o=10000 | 496.96ms |
| std | `Mutex<VecDequeue>` | t=1,o=100 | 100.49µs |
| std | `Mutex<VecDequeue>` | t=10,o=100 | 1.2141ms |
| std | `Mutex<VecDequeue>` | t=100,o=100 | 13.509ms |
| std | `Mutex<VecDequeue>` | t=100,o=10000 | 234.65ms |



# Todo
- finalization
- send sync is proper
- test on arm
- document
- main library doc
- write better unit tests
- add github workflow
- writeup readme.md
- full read of paper
- verify paper is correct
- add benchmarks to the readme
- are orders actually handled properly?
- relax bounds
- verify that haphazard memory alloc is correct

# Examples
1. _Basic Trading Order Scheduler_: (`examples/trading.rs`) A very basic example showing a single loop publishing orders to a network, and another process that sends the orders out to the exchange.


# Design

## SCQ Rings
Each data structure fundamentally relies on the SCQ ring described in the ACM paper.
The ring is a MPMC queue for indices. The size must be a power of two, hence why on initialization we must pass an order instead of a capacity directly.

## Bounded Queue
The bounded queue is the SCQ queue from the ACM paper. It works by maintaining two rings:
- _Free Ring_: This ring contains all the available indexes that we can slot a value into.
- _Allocation Ring_: This ring contains all the indexes that are currently allocated.

Additionally, the bounded queue contains a backing buffer, which stores the entries itself. The correctness of the various methods relies on the fact that both of these are correct MPMC index rings.

The bounded queue is initialized by filling the free ring with values from `0` to `n - 1` (all the valid indexes) and by leaving the allocation queue empty. Therefore, any given index can only be in **at most one** of these rings, and since they are correct concurrent MPMC index rings, only **one thread** possesses said index.

### Enqueue
The enqueue method works by first finding an available slot. To do this, an index is dequeued from the free queue. We can then access the backing buffer at this index, with two guarantees:
1. Since any index can only be in at most one of the queues, and only one thread can "possess" an index at any point in time, we have a unique index into the array and thus can safely take mutable access to that slot.
2. Since any index coming from the free queue is between `0` and `n` (`0, 1, 2, ..., n-1`).

Using this, we access the slot, set the value, and then we insert the index into the allocation array, indicating we are done.

### Dequeue

The dequeue method works similarly and off the same guarantees, except in reverse. An index is dequeued from the allocation ring, we remove it from it's slot using our unique access, and then we return the index to the free ring so it can be used.

## Unbounded Queue
This queue is described in the ACM paper as the _LCSQ_ queue. It is a more 'classical' implementation of a lock-free queue (and thus slower), but composed of bounded queues. Since adding and removing the queues themselves is a relatively rare operation, the cost is dominated by the operations on the bounded queue.

# Testing


## Loom
The loom tests are not exhaustive and should be improved in the future, since there are loops the state space can explode fast, and thus at times loom cannot run the full set of tests we would like.
```bash
$ LOOM_MAX_PREEMPTIONS=3 RUSTFLAGS="--cfg loom" cargo test loom_ --release
```

## Fuzzing
There are a whole suite of fuzz tests for finding unwanted behaviour, memory leaks, etc.

### Running Fuzz Tests
```bash
$ RUSTFLAGS="--cfg fuzzing" cargo +nightly fuzz run initfull
```

### Tests
- `scq_grind` (`fuzz/fuzz_targets/scq_grind.rs`) performs an arbitrary sequence of operations on an SCQ queue and checks that it lines up with the `VecDeque` implementation from `std`. This checks for correctness as a queue, ensuring that order is preserved correctly.
- `lscq_grind` (`fuzz/fuzz_targets/lscq_grind.rs`) performs an arbitrary sequence of operations on an SCQ queue and checks that it lines up with the `VecDeque` implementation from `std`. This checks for correctness as a queue, ensuring that order is preserved correctly.
- `scq_grind_rt` (`fuzz/fuzz_target/scq_grind_rt.rs`) performs an arbitrary sequence of operations on an SCQ queue on multiple threads. The idea is to find an error or a panic.
- `lscq_grind_rt` (`fuzz/fuzz_target/scq_grind_rt.rs`) performs an arbitrary sequence of operations on an unbounded queue on multiple threads. The idea is to find an error or a panic.
- `const_grind` (`fuzz/fuzz_target/const_grind.rs`) performs an arbitrary sequence of operations on a constant queues. It compares it with the `VecDeque` operation in `std` to verify correctness.


### Miri
```bash
$ rustup +nightly component add miri
$ cargo +nightly miri test
```
