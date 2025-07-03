//! This crate is an implementation of "A Scalable, Portable, and Memory-Efficient Lock-Free FIFO Queue"_ by Ruslan Nikolaev ([arXiV/1908.04511](https://arxiv.org/abs/1908.04511)).
//! 
//! It supports `#![no_std]` and `std`. `std` gives you access to the rings backed by an allocated buffer.
//! 
//! # Example
//! ```
//! use lfqueue::ConstBoundedQueue;
//! 
//! // We want a queue of size 2, so we make a constant queue
//! // with the generic set to 2 ^ 2 = 4.
//! let queue = ConstBoundedQueue::<usize, 4>::new_const();
//! 
//! // Let us add a few values.
//! assert!(queue.enqueue(1).is_ok());
//! assert!(queue.enqueue(2).is_ok());
//! 
//! // Let us verify the correctnes by dequeing these values.
//! assert_eq!(queue.dequeue(), Some(1));
//! assert_eq!(queue.dequeue(), Some(2));
//! assert_eq!(queue.dequeue(), None);
//! ```
//! 
//! # Design
//! 
//! ## SCQ Rings
//! Each data structure fundamentally relies on the SCQ ring described in the ACM paper.
//! The ring is a MPMC queue for indices. The size must be a power of two, hence why on initialization
//! we must pass an order instead of a capacity directly.
//! 
//! ## Bounded Queue
//! The bounded queue is the SCQ queue from the ACM paper. It works by maintaining two rings:
//! - _Free Ring_: This ring contains all the available indexes that we can slot a value into.
//! - _Allocation Ring_: This ring contains all the indexes that are currently allocated.
//! The correctness of the various methods relies 
//! 
//! 

#![cfg_attr(not(feature = "std"), no_std)]

pub(crate) mod scq;

pub(crate) mod atomics;

mod single;

#[cfg(feature = "std")]
mod lfstd;

pub use scq::BoundedQueue;
pub use scq::ConstBoundedQueue;
pub use single::SingleSize;
pub use scq::QueueError;


#[cfg(feature = "std")]
pub use lfstd::AllocBoundedQueue;
#[cfg(feature = "std")]
pub use lfstd::UnboundedQueue;
#[cfg(feature = "std")]
pub use lfstd::UnboundedEnqueueHandle;
#[cfg(feature = "std")]
pub use lfstd::UnboundedFullHandle;




// #[cfg(test)]
#[cfg(fuzzing)]
#[cfg(feature = "std")]
pub use fuzzing::*;
// mod fuzzing;

// #[cfg(test)]
#[cfg(fuzzing)]
#[cfg(feature = "std")]
mod fuzzing {
    use std::{collections::VecDeque, sync::Mutex};

    use arbitrary::{Arbitrary, *};



    #[derive(Arbitrary, Debug, Clone)]
    pub enum GrindInstr {
        Enqueue(usize),
        Dequeue
    }

    #[derive(Debug, Clone)]
    pub struct GrindConfiguration {
        pub order: usize,
        pub instructions: Vec<GrindInstr>
    }

    #[derive(Debug)]
    pub struct MockQueue<T> {
        queue: Mutex<VecDeque<T>>,
        size: usize
    }

    impl<T> MockQueue<T> {
        pub fn new(size: usize) -> Self {
            Self {
                queue: VecDeque::with_capacity(size).into(),
                size
            }
        }
        pub fn enqueue(&self, value: T) -> Result<(), T> {
            let mut handle = self.queue.lock().unwrap();
            if handle.len() >= self.size {
                return Err(value);
            }
            handle.push_back(value);
            Ok(())
        }
        pub fn enqueue_boundless(&self, value: T) {
            self.queue.lock().unwrap().push_back(value);
        }
        pub fn dequeue(&self) -> Option<T> {
            self.queue.lock().unwrap().pop_front()
        }
    }

    
    pub fn configure_grind(unstructed: &mut Unstructured) -> GrindConfiguration {
        let value = usize::arbitrary(unstructed).unwrap();

        let delta = value as f64 / usize::MAX as f64;
        let order = 1 << ((14.0 * delta) as usize + 1);

        let instructions = Vec::<GrindInstr>::arbitrary(unstructed).unwrap();
        // println!("Instructions: {:?}", instructions);

        // println!("Value: {:?}", order);

        GrindConfiguration {
            order,
            instructions
        }

    }

  
    #[cfg(test)]
    mod tests {

        // pub struct 

        #[cfg(not(loom))]
        #[test]
        pub fn check_drop_detector() {

        }
    }

}

