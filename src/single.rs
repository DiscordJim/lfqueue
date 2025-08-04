use core::{cell::UnsafeCell, mem::MaybeUninit};

use crate::atomics::*;

const EMPTY: u8 = 0x00;
const WRITING: u8 = 0x01;
const READY: u8 = 0x02;
const READING: u8 = 0x03;

#[derive(Debug)]
/// A queue that has a capacity of 1.
/// 
/// Inspired by the design in Mara Bos' book Rust Atomics & Locks
/// # Examples
/// ```
/// use lfqueue::SingleSize;
/// 
/// let queue = SingleSize::new();
/// assert!(queue.enqueue(0).is_ok());
/// assert_eq!(queue.dequeue(), Some(0));
/// 
/// ```
pub struct SingleSize<T> {
    payload: UnsafeCell<MaybeUninit<T>>,
    state: AtomicU8
}

impl<T> Default for SingleSize<T> {
    fn default() -> Self {
        Self::new()
    }
}

unsafe impl<T: Send> Send for SingleSize<T> {}
unsafe impl<T: Send> Sync for SingleSize<T> {}

impl<T> SingleSize<T> {
    /// Creates a new [SingleSize] queue.
    pub const fn new() -> Self {
        Self {
            payload: UnsafeCell::new(MaybeUninit::uninit()),
            state: AtomicU8::new(EMPTY)
        }
    }
    /// Enqueues a message.
    pub fn enqueue(&self, message: T) -> Result<(), T> {
        // We do a relaxed load here as compare exchange will lock
        // the cache line.
        if self.state.load(Relaxed) != EMPTY {
            // If the state is not empty, we cannot enqueue.
            return Err(message);
        }
        if self.state.compare_exchange(EMPTY, WRITING, Relaxed, Relaxed).is_err() {
            return Err(message);
        }
        // SAFETY: Only one thread can get access here.
        unsafe { (*self.payload.get()).write(message) };
        self.state.store(READY, Release);
        Ok(())
    }
    /// Dequeues a message.
    pub fn dequeue(&self) -> Option<T> {

        // We do a relaxed load here as compare exchange will lock
        // the cache line.
        if self.state.load(Relaxed) != READY {
            return None;
        }
        if self.state.compare_exchange(READY, READING, Acquire, Relaxed).is_err() {
            None
        } else {
            // SAFETY: atomic CAS guarantees unique usage here.
            let value = Some(unsafe { (*self.payload.get()).assume_init_read() });
            self.state.store(EMPTY, Release);
            value
        }
    }
}

impl<T> Drop for SingleSize<T> {
    fn drop(&mut self) {
        if *self.state.get_mut() == READY {
            // SAFETY: we have a unique ref & mem is initiatlized.
            unsafe {
                self.payload.get_mut().assume_init_drop()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::SingleSize;


    #[test]
    pub fn test_singlesize() {
        let value = SingleSize::<usize>::new();
        assert!(value.enqueue(1).is_ok());
        assert!(!value.enqueue(1).is_ok());
        assert_eq!(value.dequeue(), Some(1));
        assert_eq!(value.dequeue(), None);

        assert!(value.enqueue(2).is_ok());
        assert_eq!(value.dequeue(), Some(2));
    }
}