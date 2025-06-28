use crate::atomics::{AtomicBool, AtomicIsize, AtomicUsize};
use core::cell::UnsafeCell;
use core::cmp;
use core::fmt::Debug;
use core::marker::PhantomData;
use core::sync::atomic::Ordering::*;
use crossbeam_utils::{Backoff, CachePadded};

/// A constant generic [ScqRing] that does not use the heap to allocate
/// itself.
type ConstScqRing<const MODE: bool, const N: usize> = ScqRing<[CachePadded<AtomicUsize>; N], MODE>;

/// The cache padded atomic type used for ring arrays.
type PaddedAtomics = [CachePadded<AtomicUsize>];

impl<T, const N: usize> private::Sealed for [UnsafeCell<Option<T>>; N] {}
impl<const N: usize> private::Sealed for [CachePadded<AtomicUsize>; N] {}

#[inline(always)]
pub(crate) fn determine_order(length: usize) -> usize {
    assert!((length % 2 == 0) || length == 1, "Length must be a multiple of two.");
    let Some(value) = length.checked_ilog2() else {
        panic!("could not take log2 of length: {length}. Is it a power of two?");
    };
    value as usize
}

#[inline(always)]
pub(crate) const fn determine_order_const(length: usize) -> usize {
    assert!((length % 2 == 0) || length == 1, "Length must be a multiple of two.");
    let Some(value) = length.checked_ilog2() else {
        panic!("could not take log2 of length. Is it a power of two?");
    };
    value as usize - 1
}

/// The actual SCQ ring from the ACM paper. This only stores indices, and will
/// corrupt indices above a certain value. There is an error check to prevent this from
/// occuring.
///
/// # Generics
/// - `MODE` expresses the mode of the ring and enables compiler optimizaiton. It only has two
/// valid values, `0` and `1`. If it is `0`, the ring is not finalizable and this field will be optimized
/// out by the compiler. If it is `1` then the ring is finalizable and this field will not be optimized out
/// by the compiler.
#[derive(Debug)]
pub struct ScqRing<I, const MODE: usize> {
    /// Is the ring finalized? This tells us if we can insert more entries. This
    /// field is not used unless we are using it as part of an actual unbounded queue.
    ///
    /// The reason *why* this is a generic is explained in the struct documentation.
    pub(crate) is_finalized: [CachePadded<AtomicBool>; MODE],
    /// The head of the ring.
    pub(crate) head: CachePadded<AtomicUsize>,
    /// The tail of the ring.
    pub(crate) tail: CachePadded<AtomicUsize>,
    /// The threshold value, described in the ACM paper.
    pub(crate) threshold: CachePadded<AtomicIsize>,
    /// The backing array. This has strict contraints and cna only
    /// be one of two types.
    pub(crate) array: I,
    /// The order of the ring.
    pub(crate) order: usize,
}

impl<I, const MODE: usize> PartialEq for ScqRing<I, MODE>
where
    I: AsRef<PaddedAtomics> + private::Sealed,
{
    /// Checks if two [ScqRing] are identical.
    fn eq(&self, other: &Self) -> bool {
        ((self.is_finalized.len() == other.is_finalized.len())
            && !(self.is_finalized.len() > 0
                && self.is_finalized[0].load(Relaxed) != other.is_finalized[0].load(Relaxed)))
            && self.head.load(Relaxed) == other.head.load(Relaxed)
            && self.tail.load(Relaxed) == other.tail.load(Relaxed)
            && self.threshold.load(Relaxed) == other.threshold.load(Relaxed)
            && self.order == other.order
            && self
                .array
                .as_ref()
                .iter()
                .zip(other.array.as_ref().iter())
                .all(|(a, b)| a.load(Relaxed) == b.load(Relaxed))
    }
}

impl<I, const MODE: usize> Eq for ScqRing<I, MODE> where I: AsRef<PaddedAtomics> + private::Sealed {}

#[inline(always)]
pub(crate) fn lfring_threshold3(half: usize, n: usize) -> usize {
    half + n - 1
}

/// Calculates 2 ^ order.
#[inline(always)]
pub(crate) fn lfring_pow2(order: usize) -> usize {
    1usize << order
}

#[inline(always)]
pub(crate) fn modup(value: usize, n: usize) -> usize {
    value | ((n << 1) - 1)
}

/// Performs a signed comparison function, this is emulating a function
/// from the `C` code implementation and performs a signed comparison by casting
/// the two [usize] values to [usize], performing a comparison and then returning
/// the [cmp::Ordering.]
#[inline(always)]
pub(crate) fn lfring_signed_cmp(a: usize, b: usize) -> cmp::Ordering {
    ((a as isize) - (b as isize)).cmp(&0)
}

/// Represents the allocation details of an [ScqRing]. The `I` type may
/// be used to differentiate between allocated (heap) and const generic rings for
/// storage reasons so that this can be used in a no-std environment.
pub(crate) struct ScqAlloc<I> {
    /// The backing buffer to use.
    pub(crate) array: I,
    /// Where the tail should start.
    pub(crate) tail: usize,
    /// Where the threshold should tart.
    pub(crate) thresh: isize,
}

/// Initializes a full SCQ ring. This is a convienence method
/// that allows initlizing the ring with all the entries populated
/// instead of intitializing the ring and then performing costly
/// enqueue operations.
#[inline]
pub(crate) fn initialize_atomic_array_full<I>(buffer: &I, half: usize, n: usize)
where
    I: AsRef<[CachePadded<AtomicUsize>]>,
{
    let buffer = buffer.as_ref();
    let mut i = 0;

    // Initialize the part of the array that is actually filled, this
    // contains the actual values.
    while i != half {
        buffer[lfring_map(i, n)].store(n + lfring_map(i, half), Relaxed);
        i += 1;
    }

    // Intitialize the rest of the array.
    while i != n {
        buffer[lfring_map(i, n)].store(-1isize as usize, Relaxed);
        i += 1;
    }
}

/// Creates a new full array of [AtomicUsize] of size `N` with cache padding.
#[inline]
fn create_const_atomic_array_empty<const N: usize>() -> ScqAlloc<[CachePadded<AtomicUsize>; N]> {
    let array = core::array::from_fn(|_| CachePadded::new(AtomicUsize::new((-1isize) as usize)));
    ScqAlloc {
        array,
        tail: 0,
        thresh: -1,
    }
}

/// Creates a new full array of [AtomicUsize] of size `N` with cache padding.
#[inline]
fn create_const_atomic_array_full<const N: usize>() -> ScqAlloc<[CachePadded<AtomicUsize>; N]> {
    let array = core::array::from_fn(|_| CachePadded::new(AtomicUsize::new((-1isize) as usize)));

    let n = const { N };
    let half = n >> 1;

    initialize_atomic_array_full(&array, half, n);

    ScqAlloc {
        array,
        tail: half,
        thresh: lfring_threshold3(half, n) as isize,
    }
}

impl<const MODE: usize, const N: usize> ConstScqRing<MODE, N> {
    /// Creates a new constant ring that is empty.
    pub fn new_const_ring_empty() -> Self {
        // println!("made queue of size: {}", (const { N } >> 1) - 1);
        Self::new_from_sqalloc(determine_order_const(N), create_const_atomic_array_empty())
    }
    /// Creates a new constant ring that is full.
    pub fn new_const_ring_full() -> Self {
        Self::new_from_sqalloc(determine_order_const(N), create_const_atomic_array_full())
    }
}

impl<I, const MODE: usize> ScqRing<I, MODE>
where
    I: AsRef<[CachePadded<AtomicUsize>]> + private::Sealed,
{
    /// Creates a new [ScqRing] from an [ScqAlloc]. This is a helper
    /// function so implementations of [ScqRing] with different types of backing
    /// arrays can be used correctly.
    pub(crate) fn new_from_sqalloc(
        order: usize,
        ScqAlloc {
            tail,
            thresh,
            array,
        }: ScqAlloc<I>,
    ) -> Self {
        // This method is private, so this assertion is a debug insertion
        // because this will be correctly handled by the structs that use
        // the ring.
        debug_assert!(MODE <= 1, "The mode must be either 0 or 1.");

        Self {
            // When the ring is not finalizable, the compiler will optimize out this instruction.
            is_finalized: core::array::from_fn(|_| CachePadded::new(AtomicBool::new(false))),
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(tail)),
            threshold: CachePadded::new(AtomicIsize::new(thresh)),
            array,
            order,
        }
    }
    /// Finalizes the [ScqRing] so no more elements may be stored.
    /// If MODE != 1, this is a no-op.
    #[inline(always)]
    fn finalize(&self) {
        if const { MODE == 1 } {
            // SAFETY: We just checked that MODE == 1, thus the array has a
            // size of 1 and there is no point in repeating the index check here.
            unsafe { self.is_finalized.get_unchecked(0) }.store(true, Release);
        } else {
            // If we are in debug mode, to be thorough, let's throw an exception here.
            debug_assert!(const { MODE == 1 }, "Called finalize() on a non-finalizable ring.");
        }
    }
    /// Enqueues an index in the ring. The index must be less than 1 << order or
    /// else it could be corrupted as a modulo operation.
    pub fn enqueue(&self, mut eidx: usize) -> Result<(), ScqError> {
        if eidx >= self.capacity() {
            // The index would be corrupted here.
            return Err(ScqError::IndexLargerThanOrder);
        }

        let backff = Backoff::new();

        // Calculate `n` and `half`.
        let half = lfring_pow2(self.order);
        let n = half << 1;

        // Perform a modulo by `N`.
        eidx ^= n - 1;

    
        loop {
            // Load the tail.
            let tail = self.tail.fetch_add(1, AcqRel);
            let tcycle = modup(tail << 1, n);
            let tidx = lfring_map(tail, n);
          
            'retry: loop {
                // Load the entry, calculate the ecycle.
                let entry = self.array.as_ref()[tidx].load(Acquire);
                let ecycle = modup(entry, n);
              
                if (lfring_signed_cmp(ecycle, tcycle).is_lt())
                    && ((entry == ecycle)
                        || ((entry == (ecycle ^ n))
                            && lfring_signed_cmp(self.head.load(Acquire), tail).is_le()))
                {
                    // If this is a finalizable ring, we will proceed to finalize it.
                    // This is done with constants to encourage the compiler to optimize
                    // out the operation if we are working with a bounded array.
                    if const { MODE == 1 } {
                        // SAFETY: This is a generic array, and we have just checked the length by verifying the mode.
                        // Therefore it is safe to access this index.
                        if unsafe { self.is_finalized.get_unchecked(0) }.load(Acquire) {
                            return Err(ScqError::QueueFinalized);
                        }
                    }

                    // Try to insert the entry.
                    if self.array.as_ref()[tidx]
                        .compare_exchange_weak(entry, tcycle ^ eidx, AcqRel, Acquire)
                        .is_err()
                    {
                        yield_marker();
                        backff.spin();
                        continue 'retry;
                    }

                    // Update the threshold.
                    // FUTURE: Does this need SeqCst ordering?
                    let threshold = lfring_threshold3(half, n) as isize;
                    if self.threshold.load(SeqCst) != threshold {
                        self.threshold.store(threshold, SeqCst);
                    }

                    return Ok(());
                } else {
                    break;
                }
            }
            backff.snooze();
            yield_marker();
        }
    }
    /// Returns the capacity of the ring. This is 2 ^ order.
    #[inline]
    pub fn capacity(&self) -> usize {
        1 << (self.order)
    }
    /// Catches the tail up with the head.
    fn catchup(&self, mut tail: usize, mut head: usize) {
        while self
            .tail
            .compare_exchange_weak(tail, head, AcqRel, Acquire)
            .is_err()
        {
            head = self.head.load(Acquire);
            tail = self.tail.load(Acquire);
            if lfring_signed_cmp(tail, head).is_ge() {
                break;
            }
        }
    }
    /// Dequeues an index from the [ScqRing].
    pub fn dequeue(&self) -> Option<usize> {
        let n = lfring_pow2(self.order + 1);

        // Check the threshold and if we are empty, if we
        // are less than zero then it must be zero.
        if self.threshold.load(SeqCst) < 0 {
            return None;
        }

        let backoff = Backoff::new();

        let mut entry_new;

        loop {
            // Load the head.
            let head = self.head.fetch_add(1, AcqRel);
            let hcycle = modup(head << 1, n);
            let hidx = lfring_map(head, n);
            let mut attempt = 0;

            'again: loop {
                loop {
                    // Load the entry and calculate the cycle of the entry.
                    let entry = self.array.as_ref()[hidx].load(Acquire);
                    let ecycle = modup(entry, n);


                    if ecycle == hcycle {
                        // The cycle is the same, remove the entry.
                        self.array.as_ref()[hidx].fetch_or(n - 1, AcqRel);
                        return Some(entry & (n - 1));
                    }

                    if (entry | n) != ecycle {
                        entry_new = entry & !n;
                        if entry == entry_new {
                            break;
                        }
                    } else {
                        attempt += 1;
                        if attempt <= 10 {
                            yield_marker();
                            backoff.spin();
                            continue 'again;
                        }
                        entry_new = hcycle ^ ((!entry) & n);
                    }

                    // Try to swap out the entry.
                    if !(lfring_signed_cmp(ecycle, hcycle).is_lt()
                        && self.array.as_ref()[hidx]
                            .compare_exchange_weak(entry, entry_new, AcqRel, Acquire)
                            .is_err())
                    {
                        break;
                    }


                    backoff.snooze();
                    yield_marker();
                }
                break;
            }

            
            // Check update the tail & threshold.
            let tail = self.tail.load(Acquire);
            if lfring_signed_cmp(tail, head + 1).is_le() {
                self.catchup(tail, head + 1);
                self.threshold.fetch_sub(1, AcqRel);
                return None;
            }

            if self.threshold.fetch_sub(1, AcqRel) <= 0 {
                return None;
            }

            yield_marker();
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum ScqError {
    /// The items inserted will get corrupted if they are greater
    /// or equal to 2 ^ (order + 1).
    IndexLargerThanOrder,
    /// The queue is completely filled.
    QueueFull,
    /// The queue is finalized.
    QueueFinalized,
}

impl core::fmt::Display for ScqError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::IndexLargerThanOrder => "IndexLargerThanOrder",
            Self::QueueFinalized => "QueueFinalized",
            Self::QueueFull => "QueueFull"
        })
    }
}

impl core::error::Error for ScqError {
    fn cause(&self) -> Option<&dyn core::error::Error> {
        None
    }
    fn description(&self) -> &str {
        match self {
            Self::IndexLargerThanOrder => "The entry provided was greater or equal than 2 ^ order.",
            Self::QueueFinalized => "Attempted to insert an entry but the queue was finalized.",
            Self::QueueFull => "The queue is full of elements."
        }
    }
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        None
    }
}

pub(crate) mod private {
    pub trait Sealed {}
}
#[inline(always)]
pub(crate) fn yield_marker() {
    // std::thread::yield_now();
    #[cfg(loom)]
    loom::thread::yield_now();
}


/// The bounded queue type from the ACM paper. This uses
/// two SCQ rings internally, one that keeps track of free indices
/// and the other that keeps track of the allocated indices.
#[derive(Debug)]
pub struct BoundedQueue<T, I, RING, const SEAL: usize>
where 
    I: private::Sealed
{
    /// The backing array that keeps track of all the slots.
    pub(crate) backing: I,
    /// The queue that tracks all the free indices.
    pub(crate) free_queue: ScqRing<RING, SEAL>,
    /// The queue that tracks all the allocated indices.
    pub(crate) alloc_queue: ScqRing<RING, SEAL>,
    /// How many slots have been used up.
    pub(crate) used: CachePadded<AtomicUsize>,
    /// The actual type that the backing array will be storing.
    pub(crate) _type: PhantomData<T>,
}

unsafe impl<T: Send + Sync, I: private::Sealed, R, const SEAL: usize> Send for BoundedQueue<T, I, R, SEAL> {}
unsafe impl<T: Send + Sync, I: private::Sealed, R, const SEAL: usize> Sync for BoundedQueue<T, I, R, SEAL> {}

/// A constant generic constant bounded queue, this implements the SCQ from the ACM paper,
/// "A Scalable, Portable, and Memory-Efficient Lock-Free FIFO Queue" by Ruslan Nikolaev.
///
/// Generally, if you want to work with these what you really want is the [const_queue!](crate::const_queue) macro
/// which will configure the size for you properly. Due to the inner workings of the data structure, it needs 2 * N slots
/// to operate properly. Thus, to make a constant queue of size 2, the constant parameter should be set to `4`. To ease
/// the burden of this, the [const_queue!](crate::const_queue) macro exists.
///
/// # Preferred Initialization
/// ```
/// 
/// ```
/// 
/// # Manual Example
/// ```
/// use lfqueue::{ConstBoundedQueue, ScqError};
///
/// // Make a queue of size 4.
/// let queue = ConstBoundedQueue::<usize, 4>::new_const();
///
/// assert_eq!(queue.capacity(), 2);
/// assert!(queue.enqueue(2).is_ok());
/// assert!(queue.enqueue(3).is_ok());
/// assert_eq!(queue.enqueue(4), Err(ScqError::QueueFull));
/// ```
pub type ConstBoundedQueue<T, const N: usize> =
    BoundedQueue<T, [UnsafeCell<Option<T>>; N], [CachePadded<AtomicUsize>; N], 0>;


/// Creates a [ConstBoundedQueue]. This function exists mostly to creates
/// queues of the correct size easily. For instance, to create a queue that can
/// hold 2 values, the bound needs to be four. This macro assists with that by generating
/// an initialization that takes this into account.
/// 
/// # Compile Error
/// Constant queues have a few limitations due to the inner workings of the ring:
/// 1. They must not be empty. You cannot create an `empty` ring.
/// 2. They must be powers of two. Thus only 1, 2, 4, etc. are valid.
/// 
/// # Example
/// ```
/// use lfqueue::{const_queue, ConstBoundedQueue, ScqError};
/// 
/// // Let us create a constant queue of size 1.
/// let queue = const_queue!(usize; 1);
/// assert!(queue.enqueue(1).is_ok());
/// assert_eq!(queue.enqueue(2), Err(ScqError::QueueFull));
/// 
/// // Let us create a constant queue of size 8;
/// let queue = const_queue!(usize; 8);
/// for i in 0..8 {
///     assert!(queue.enqueue(i).is_ok());
/// }
/// assert!(queue.enqueue(0).is_err()); // queue full
/// 
/// ```
#[macro_export]
macro_rules! const_queue {
    ($ttype:ty; $size:expr) => {
        ConstBoundedQueue::<$ttype, {
            const _ASSERT: () = {
                assert!($size != 0, "Size cannot be empty for constant queues.");
                assert!($size % 2 == 0 || $size == 1, "Size is not valid for a constant queue. Must be even or one.");
                ()
            };
            $size * 2

        }>::new_const()
    };
}



impl<T, const N: usize> ConstBoundedQueue<T, N> {
    /// A helper function for creating constant bounded queues, will automatically
    /// try to calculate the correct order.
    ///
    /// # Panics
    /// This function will panic if the value is not a power of two and also if the
    /// value is zero as we cannot initialize zero sized constant bounded queues.
    ///
    /// # Example
    /// ```
    /// use lfqueue::{ConstBoundedQueue, ScqError};
    ///
    /// let queue = ConstBoundedQueue::<usize, 4>::new_const();
    /// assert!(queue.enqueue(2).is_ok());
    /// assert!(queue.enqueue(3).is_ok());
    /// assert_eq!(queue.enqueue(4), Err(ScqError::QueueFull));
    /// ```
    pub fn new_const() -> Self {
        if const { N } % 2 != 0 {
            panic!("Value must be a power of two.");
        }
        if const { N } == 0 {
            panic!("Constant arrays cannot be initialized to be empty.");
        }
        
        Self {
            alloc_queue: ScqRing::new_const_ring_empty(),
            free_queue: ScqRing::new_const_ring_full(),
            backing: core::array::from_fn(|_| UnsafeCell::new(None)),
            used: CachePadded::new(AtomicUsize::new(0)),
            _type: PhantomData,
        }
    }
}

impl<T, I, P, const S: usize> BoundedQueue<T, I, P, S>
where
    I: AsRef<[UnsafeCell<Option<T>>]> + private::Sealed,
    P: AsRef<PaddedAtomics> + private::Sealed,
{
    pub const MAX_ORDER: usize = 63;
    pub const MIN_ORDER: usize = 0;

    /// Returns the capacity of the bounded ring. This is 2 ^ order.
    /// 
    /// # Example
    /// ```
    /// use lfqueue::ConstBoundedQueue;
    /// 
    /// let value = ConstBoundedQueue::<usize, 4>::new_const();
    /// assert_eq!(value.capacity(), 2);
    /// ```
    pub fn capacity(&self) -> usize {
        self.free_queue.capacity()
    }

    /// Enqueues an element to the bounded queue.
    ///
    /// # Example
    /// ```
    /// use lfqueue::AllocBoundedQueue;
    ///
    /// let queue = AllocBoundedQueue::<usize>::new(2);
    /// assert_eq!(queue.enqueue(4), Ok(()));
    /// assert_eq!(queue.dequeue(), Some(4));
    /// ```
    pub fn enqueue(&self, item: T) -> Result<(), ScqError> {
        // We want to make a call to the internal method here
        // without finalizing. If someone is calling the method
        // with [BounedQueue::enqueue] then it is not part of an unbounded
        // queue.
        //
        // The idea is to make this into the public method, as a developer
        // using the crate should never have to make the decision whether
        // to enqueue with finalization or not.
        self.enqueue_cycle::<false>(item).map_err(|(_, b)| b)
    }

    /// Indexes a raw pointer to an index.
    /// 
    /// # Safety
    /// The index must be within bounds always. This will skip
    /// the bounds check in release mode.
    #[inline(always)]
    unsafe fn index_ptr(&self, index: usize) -> *mut Option<T> {
        let bref = self.backing.as_ref();
        debug_assert!((0..bref.len()).contains(&index), "Index is out of bounds.");
        // SAFETY: The caller safety contract requires the index to be valid.
        unsafe { bref.get_unchecked(index) }.get()
    }

    /// The internal enqueue function. This prevents cloning by returning the original
    ///
    #[inline(always)]
    pub(crate) fn enqueue_cycle<const FINALIZE: bool>(&self, item: T) -> Result<(), (T, ScqError)> {
        // Check if we may add an item to the queue.
        let size = self.used.fetch_add(1, AcqRel);
        if size >= self.free_queue.capacity() {
            self.used.fetch_sub(1, AcqRel);
            return Err((item, ScqError::QueueFull));
        }

        // Check if we may dequeue an item.
        let Some(pos) = self.free_queue.dequeue() else {
            self.used.fetch_sub(1, AcqRel);
            return Err((item, ScqError::QueueFull));
        };

        // SAFETY: the ring only contains valid indices.
        unsafe { *self.index_ptr(pos) = Some(item) };

        if let Err(error) = self.alloc_queue.enqueue(pos) {
            debug_assert_eq!(
                error,
                ScqError::QueueFinalized,
                "Received a queue full notification."
            );

            self.used.fetch_sub(1, AcqRel);
            let item = unsafe { (*self.index_ptr(pos)).take() };
            self.free_queue.enqueue(pos).unwrap();
            return Err((item.unwrap(), error));
        }


        if const { FINALIZE } && size + 1 >= self.free_queue.capacity() {
            // As described in the paper we must finalize this queue
            // so that nothing more will be added to it.
            self.alloc_queue.finalize();
        }


        Ok(())
    }
    /// Dequeues an element from the bounded queue.
    ///
    /// # Example
    /// ```
    /// use lfqueue::AllocBoundedQueue;
    ///
    /// let queue = AllocBoundedQueue::<usize>::new(2);
    /// assert_eq!(queue.enqueue(4), Ok(()));
    /// assert_eq!(queue.dequeue(), Some(4));
    /// ```
    pub fn dequeue(&self) -> Option<T> {
        // Dequeue an allocated position, if this returns
        // some it will become a unique index.
        let pos = self.alloc_queue.dequeue()?;


        // Decrease the length of the queue.
        self.used.fetch_sub(1, AcqRel);

        // Take the value out of the option
        let Some(value) = (unsafe { (*self.index_ptr(pos)).take() }) else {
            panic!("Failed to remove a value from the slot.");
        };

        // Enqueue error, this should never happen.
        if let Err(e) = self.free_queue.enqueue(pos) {
            panic!("ScqError: {e:?}");
        }
        Some(value)
    }
}

/// The mapping function from the ACM paper.
#[inline(always)]
fn lfring_map(idx: usize, n: usize) -> usize {
    idx & (n - 1)
}






#[cfg(test)]
mod tests {
    // use std::marker::PhantomData;

    use crate::scq::{ScqError, lfring_signed_cmp};

    #[cfg(loom)]
    #[test]
    pub fn loom_finalization_weak() {
        // Checks that post finalization we cannot insert anything into the queue.
        loom::model(|| {
            use loom::sync::atomic::Ordering;

            let v1 = loom::sync::Arc::new(ScqRing::<true>::new(3));
            let v2 = v1.clone();

            // Finalize the queue.
            v1.finalize();

            loom::thread::spawn(move || {
                // Anything after this should be an error.
                assert!(v1.enqueue(0).is_err());
            });
        });
    }

    // #[cfg(loom)]
    // #[test]
    // pub fn loom_bounded_queue() {
    //     loom::model(|| {
    //         let ring = loom::sync::Arc::new(ScqQueue::new(4));

    //         let mut handles = vec![];

    //         for _ in 0..2 {
    //             handles.push(loom::thread::spawn({
    //                 let ring = ring.clone();
    //                 move || {
    //                     for i in 0..16 {
    //                         ring.enqueue(i).unwrap();
    //                     }
    //                     assert!(ring.dequeue().unwrap() <= 1);
    //                     // for i in 0..16 {
    //                     //     assert_eq!(ring.dequeue(), Some(i));
    //                     // }
    //                 }
    //             }));
    //         }

    //         for handle in handles {
    //             handle.join().unwrap();
    //         }

    //         let backed = ring
    //             .backing
    //             .iter()
    //             .map(|f| unsafe { *f.get() }.clone())
    //             .collect::<Vec<_>>();
    //         println!("Backed: {backed:?}");
    //         let mut count = 0;
    //         for i in 0..32 {
    //             // SAFETY: All threads have terminated, we have exclusive access.
    //             // assert!(backed[i].is_some());
    //             if backed[i].is_some() {
    //                 count += 1;
    //             }
    //         }
    //         assert_eq!(count, 30);
    //     });
    // }

    // #[cfg(not(loom))]
    // #[test]
    // pub fn unloom_bounded_queue() {
    //     // loom::model(|| {
    //     let ring = std::sync::Arc::new(ScqQueue::new(4));

    //     let mut handles = vec![];

    //     for _ in 0..2 {
    //         handles.push(std::thread::spawn({
    //             let ring = ring.clone();
    //             move || {
    //                 for i in 0..16 {
    //                     ring.enqueue(i).unwrap();
    //                 }
    //                 assert!(ring.dequeue().unwrap() <= 1);
    //                 // assert_eq!(ring.dequeue(), Some(0));
    //                 // for i in 0..16 {
    //                 //     assert_eq!(ring.dequeue(), Some(i));
    //                 // }
    //             }
    //         }));
    //     }

    //     for handle in handles {
    //         handle.join().unwrap();
    //     }

    //     let backed = ring
    //         .backing
    //         .iter()
    //         .map(|f| unsafe { *f.get() }.clone())
    //         .collect::<Vec<_>>();
    //     println!("Backed: {backed:?}");
    //     for i in 0..32 {
    //         // SAFETY: All threads have terminated, we have exclusive access.
    //         assert!(backed[i].is_some());
    //     }

    //     // for val in ring.backing {
    //     //     // SAFETY: We have exclusive access as all threads have terminated.
    //     //     val.get_mut()
    //     // }

    //     while let Some(val) = ring.dequeue() {
    //         println!("Value: {val}");
    //     }
    //     // });
    // }

    // #[cfg(loom)]
    // #[test]
    // pub fn loom_bounded_ring() {
    //     loom::model(|| {
    //         let ring = ScqRing::new(2);

    //         for i in 0..ring.capacity() {
    //             ring.enqueue(i).unwrap();
    //         }

    //         for i in 0..ring.capacity() {
    //             assert_eq!(ring.dequeue(), Some(i));
    //         }
    //     });
    // }

    #[test]
    #[cfg(not(loom))]
    pub fn test_pow2() {
        // this is just a sanity check for myself.
        let half = 4;
        assert_eq!(half * 2, half << 1);
    }

    #[test]
    #[cfg(not(loom))]
    pub fn test_const_queue() {
        use crate::ConstBoundedQueue;

        let queue = ConstBoundedQueue::<usize, 4>::new_const();
        assert!(queue.enqueue(1).is_ok());
        assert!(queue.enqueue(2).is_ok());
        assert_eq!(queue.enqueue(3), Err(ScqError::QueueFull));
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_special_comparision_function() {
        assert!(lfring_signed_cmp(1, 2).is_lt());
        assert!(lfring_signed_cmp(1, 2).is_lt());
        assert!(lfring_signed_cmp(2, 2).is_le());
        assert!(lfring_signed_cmp(2, 1).is_gt());
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_determine_order() {
        use crate::scq::determine_order;

        assert_eq!(determine_order(1), 0);
        assert_eq!(determine_order(2), 1);
        assert_eq!(determine_order(4), 2);
        assert_eq!(determine_order(512), 9);
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_init_const_queue() {
        // PURPOSE: the purpose of this test is to ensure
        // the rings are getting initialized to the correct size.
        use crate::{const_queue, ConstBoundedQueue};

        // Check the queue for consistency.
        fn check_queue<const N: usize>(queue: ConstBoundedQueue<usize, N>) {
            assert_eq!(queue.capacity(), N >> 1);
            for i in 0..(N >> 1) {
                assert!(queue.enqueue(i).is_ok());
            }
            for i in 0..(N >> 1) {
                assert_eq!(queue.enqueue(i), Err(ScqError::QueueFull));
            }
            for i in 0..(N >> 1) {
                assert_eq!(queue.dequeue(), Some(i));
            }
            for _ in 0..(N >> 1) {
                assert_eq!(queue.dequeue(), None);
            }
        }

        assert_eq!(const_queue!(usize; 1).capacity(), 1);

        // Check a few queues.
        check_queue(const_queue!(usize; 1));
        check_queue(const_queue!(usize; 2));
        check_queue(const_queue!(usize; 4));
        check_queue(const_queue!(usize; 8));
        check_queue(const_queue!(usize; 16));
        check_queue(const_queue!(usize; 32));
        check_queue(const_queue!(usize; 64));
        check_queue(const_queue!(usize; 128));


  
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_zst_const() {
        use crate::ConstBoundedQueue;


        let queue = const_queue!((); 4);
        assert_eq!(queue.capacity(), 4);
        for _ in 0..queue.capacity() {
            assert!(queue.enqueue(()).is_ok());
        }
        assert_eq!(queue.enqueue(()), Err(ScqError::QueueFull));

        for _ in 0..queue.capacity() {
            assert_eq!(queue.dequeue(), Some(()));
        }
        assert!(queue.dequeue().is_none());

        assert!(queue.enqueue(()).is_ok());
        

    }
}
