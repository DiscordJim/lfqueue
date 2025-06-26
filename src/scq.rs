// //! An implementation of the lock-free queue
// //! from "[PAPER NAME HERE]"
// //!
// //!
// //! https://github.com/rusnikola/lfqueue/blob/master/lfring_cas1.h
// use std::sync::atomic::{AtomicIsize, AtomicUsize};

use std::cell::UnsafeCell;
use std::fmt::Debug;
use std::marker::PhantomData;
use std::ptr::{NonNull, null_mut};
use std::{cmp};

use haphazard::HazardPointer;
#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

use crossbeam_utils::CachePadded;




#[derive(Debug)]
pub struct ScqRing<const MODE: bool> {
    is_finalized: CachePadded<AtomicBool>,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    threshold: CachePadded<AtomicIsize>,
    array: Box<[ScqEntry]>,
    order: usize,
}


impl<const MODE: bool> PartialEq for ScqRing<MODE> {
    fn eq(&self, other: &Self) -> bool {
        self.is_finalized.load(Relaxed) == other.is_finalized.load(Relaxed)
            && self.head.load(Relaxed) == other.head.load(Relaxed)
            && self.tail.load(Relaxed) == other.tail.load(Relaxed)
            && self.threshold.load(Relaxed) == other.threshold.load(Relaxed)
            && self.order == other.order
            && self
                .array
                .iter()
                .zip(other.array.iter())
                .all(|(a, b)| a.value.load(Relaxed) == b.value.load(Relaxed))
    }
}

impl<const MODE: bool> Eq for ScqRing<MODE> {}

#[inline(always)]
fn lfring_threshold3(half: usize, n: usize) -> usize {
    (half) + (n) - 1
}

#[inline(always)]
fn lfring_pow2(order: usize) -> usize {
    1usize << order
}

#[inline(always)]
fn modup(value: usize, n: usize) -> usize {
    value | (2 * n - 1)
}

#[inline(always)]
fn lfring_signed_cmp(a: usize, b: usize) -> cmp::Ordering {
    ((a as isize) - (b as isize)).cmp(&0)
}

type AtomicIndexArray = Box<[ScqEntry]>;

struct ScqAlloc {
    tail: usize,
    thresh: isize,
    array: AtomicIndexArray,
}

fn allocate_atomic_array_empty(order: usize) -> ScqAlloc {
    let n = lfring_pow2(order + 1);
    let array = (0..n)
        .map(|_| AtomicUsize::new((-1isize) as usize))
        .map(CachePadded::new)
        .map(|v| ScqEntry {
            value: v,
            // is_safe: CachePadded::new(AtomicBool::new(true)),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    ScqAlloc {
        array,
        tail: 0,
        thresh: -1,
    }
}

fn allocate_atomic_array_full(order: usize) -> ScqAlloc {
    let half = lfring_pow2(order);
    let n = half * 2;

    // Initialize an array of
    let mut vector = Vec::with_capacity(n);
    vector.reserve_exact(n);
    for _ in 0..n {
        vector.push(CachePadded::new(AtomicUsize::new(-1isize as usize)));
    }

    // let array = (0..n)
    //     .map(|_| AtomicUsize::new((-1isize) as usize))
    //     .map(CachePadded::new)
    //     .collect::<Vec<_>>()
    //     .into_boxed_slice();

    let mut i = 0;
    while i != half {
        vector[lfring_map(i, n)].store(n + lfring_map(i, half), Relaxed);
        i += 1;
    }

    while i != n {
        vector[lfring_map(i, n)].store(-1isize as usize, Relaxed);
        i += 1;
    }

    // while i != n {
    //     array[lfring_map(i, order, n)].store(-1isize as usize, Relaxed);
    //     i += 1;
    // }

    ScqAlloc {
        array: vector
            .into_iter()
            .map(|value| ScqEntry {
                value,
                // is_safe: CachePadded::new(AtomicBool::new(true)),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice(),
        tail: half,
        thresh: lfring_threshold3(half, n) as isize,
    }
}

// fn allocate_atomic_array_fill_linear(order: usize) -> ScqAlloc {
//     let half = lfring_pow2(order);
//     let n = half * 2;

//     ScqAlloc {
//         thresh: half,
//         array:
//     }

// }

#[repr(transparent)]
#[derive(Debug)]
struct ScqEntry {
    value: CachePadded<AtomicUsize>,
    // is_safe: CachePadded<AtomicBool>,
}



impl<const MODE: bool> ScqRing<MODE> {
    pub fn new(order: usize) -> Self {
        let array = allocate_atomic_array_empty(order);
        Self::new_from_sqalloc(order, array)
    }
    pub fn new_full(order: usize) -> Self {
        let array = allocate_atomic_array_full(order);
        Self::new_from_sqalloc(order, array)
    }
    fn new_from_sqalloc(
        order: usize,
        ScqAlloc {
            tail,
            thresh,
            array,
        }: ScqAlloc,
    ) -> Self {
        
        // assert!(order >= 1, "Order must at least be three.");
        Self {
            is_finalized: CachePadded::new(AtomicBool::new(false)),
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(tail)),
            threshold: CachePadded::new(AtomicIsize::new(thresh)),
            array,
            order,
        }
    }

    /// Finalizes the [ScqRing] so no more elements may be stored.
    fn finalize(&self) {
        if const { !MODE } {
            panic!("Called finalize() on a non-finalizable ring.");
        }
        self.is_finalized.store(true, Release);
    }
    // Tail: 0, Tcycle: 15, Tidx: 0, entry: -1
    pub fn enqueue(&self, mut eidx: usize) -> Result<(), ScqError> {
        if eidx >= self.capacity() {
            // The index would be corrupted here.
            return Err(ScqError::IndexLargerThanOrder);
        }

        // println!("Self: {}", self.threshold.load(SeqCst));

        let half = lfring_pow2(self.order);
        let n = half * 2;

        // Modulo the value.
        // println!("ORIGINAL EIDX: {eidx}, {}", n - 1);
        eidx ^= n - 1;

        // println!("EIDX: {}, N: {}", eidx, n);

        loop {
            let tail = self.tail.fetch_add(1, AcqRel);
            let tcycle = modup(tail << 1, n);
            let tidx = lfring_map(tail, n);
            // let mut entry = self.array[tidx].value.load(Acquire);

            // println!("Tail: {tail}, TCycle: {tcycle}, TIDX: {tidx}, Entry: {entry}");

            'retry: loop {
                let entry = self.array[tidx].value.load(Acquire);

                // println!("Entering retry... {}", sel);
                let ecycle = modup(entry, n);
                // let ecycle = entry | (2 * n - 1);
                // println!("ECycle: {}", ecycle as isize);

                //   exit(1);
                // println!("Entry: {}, Ecycle: {}", entry as isize, ecycle as isize);

                if (lfring_signed_cmp(ecycle, tcycle).is_lt())
                    && ((entry == ecycle)
                        || ((entry == (ecycle ^ n))
                            && lfring_signed_cmp(self.head.load(Acquire), tail).is_le()))
                {
                    if const { MODE } {
                        // Check the finalization, we want this to be compiled conditionally.
                        if self.is_finalized.load(Acquire) {
                            return Err(ScqError::QueueFinalized);
                        }
                    }
                    

                    if self.array[tidx]
                        .value
                        .compare_exchange_weak(entry, tcycle ^ eidx, AcqRel, Acquire)
                        .is_err()
                    {
                        yield_marker();
                        // println!("Spinning here...");
                        continue 'retry;
                    }

                    let threshold = lfring_threshold3(half, n) as isize;
                    if self.threshold.load(SeqCst) != threshold {
                        self.threshold.store(threshold, SeqCst);
                    }

                    return Ok(());
                } else {
                    break;
                }
            }
            // return Err(ScqError::QueueFull);
            // println!("loop c");
            yield_marker();

            // println!("Trigger");

            // println!("Entry: {}, Tcycle: {}", entry as isize, tcycle as isize);
            // println!("traj");

            // NOTE: We can return here as this generally only fails if it is empty, but this is not universally
            // true.
            // return Err(ScqError::QueueFull);
        }
    }
    pub fn capacity(&self) -> usize {
        1 << (self.order)
    }
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
    pub fn dequeue(&self) -> Option<usize> {
        let n = 1 << (self.order + 1);

        if self.threshold.load(SeqCst) < 0 {
            return None;
        }

        let mut entry_new;

        loop {
            let head = self.head.fetch_add(1, AcqRel);
            let hcycle = modup(head << 1, n);
            let hidx = lfring_map(head, n);
            let mut attempt = 0;
            // println!("Entering dequeue loop...");

            'again: loop {
                // START DO
                loop {
                    let entry = self.array[hidx].value.load(Acquire);

                    // println!("Retry loop...");
                    let ecycle = modup(entry, n);
                    // println!("Ecycle: {}, Hcycle: {}", ecycle, hcycle);
                    if ecycle == hcycle {
                        // NOTE: IN THE SOURCE THIS IS n - 1s
                        self.array[hidx].value.fetch_or(n - 1, AcqRel);
                        // println!("Dentry: {entry}, Ecycle: {ecycle}, Hidx: {hidx}, Head: {head}, Hcycle: {hcycle}");
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
                            // println!("Looping here...");
                            continue 'again;
                        }
                        // println!("John");
                        entry_new = hcycle ^ ((!entry) & n);
                    }

                    if !(lfring_signed_cmp(ecycle, hcycle).is_lt()
                        && self.array[hidx]
                            .value
                            .compare_exchange_weak(entry, entry_new, AcqRel, Acquire)
                            .is_err())
                    {
                        break;
                    }

                    // println!("Loop B");
                    yield_marker();
                }
                // END DO
                break;
            }

            let tail = self.tail.load(Acquire);
            if lfring_signed_cmp(tail, head + 1).is_le() {
                self.catchup(tail, head + 1);
                self.threshold.fetch_sub(1, AcqRel);
                // println!("Exiting out of branch here...");
                return None;
            }

            if self.threshold.fetch_sub(1, AcqRel) <= 0 {
                return None;
            }

            // println!("Loop C");
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

mod private {
    pub trait Sealed {}
}


/// A trait that represents a type.
pub unsafe trait IsolatedSlotable: private::Sealed {
    type Type;

    fn new(order: usize) -> Self;
    unsafe fn unique_index(&self, index: usize) -> *mut Option<Self::Type>;

}

#[derive(Debug)]
pub struct BoundedQueue<T, I, const SEAL: bool> {
    // backing: Box<[UnsafeCell<Option<T>>]>,
    backing: I,
    free_queue: ScqRing<SEAL>,
    alloc_queue: ScqRing<SEAL>,
    used: CachePadded<AtomicUsize>,
    _type: PhantomData<T>,
}


unsafe impl<T: Send + Sync, I: IsolatedSlotable, const S: bool> Send for BoundedQueue<T, I, S> {}
unsafe impl<T: Send + Sync, I: IsolatedSlotable, const S: bool> Sync for BoundedQueue<T, I, S> {}

pub type AllocBoundedQueue<T> = BoundedQueue<T, Box<[UnsafeCell<Option<T>>]>, false>;


/// A constant generic constant bounded queue, this implements the SCQ from the ACM paper,
/// "A Scalable, Portable, and Memory-Efficient Lock-Free FIFO Queue" by Ruslan Nikolaev.
/// 
/// The generic parameter must be chosen intelligently, you must choose N = 2 ^ (x + 1) where
/// 2 ^ x is the amount of elements you want to store. For instance, for a queue of size 2 (x = 2) then
/// you would choose N = 2 ^ (2 + 1) = 8;
/// 
/// # Example
/// ```
/// use lfqueue::{ConstBoundedQueue, ScqError};
/// 
/// let queue = ConstBoundedQueue::<usize, 4>::new_const_queue();
/// assert!(queue.enqueue(2).is_ok());
/// assert!(queue.enqueue(3).is_ok());
/// assert_eq!(queue.enqueue(4), Err(ScqError::QueueFull));
/// ```
pub type ConstBoundedQueue<T, const N: usize> = BoundedQueue<T, [Option<T>; N], false>;



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
    /// let queue = ConstBoundedQueue::<usize, 4>::new_const_queue();
    /// assert!(queue.enqueue(2).is_ok());
    /// assert!(queue.enqueue(3).is_ok());
    /// assert_eq!(queue.enqueue(4), Err(ScqError::QueueFull));
    /// ```
    pub fn new_const_queue() -> Self {
        if const { N } % 2 != 0 {
            panic!("Value must be a power of two.");
        }
        if const { N } == 0 {
            panic!("Constant arrays cannot be initialized to be empty.");
        }
        let order = (const { N } >> 1) - 1;
        Self::new(order)

    }
}

impl<T, I> BoundedQueue<T, I, false>
where 
    I: IsolatedSlotable<Type = T>
{
    pub fn new(order: usize) -> Self {
        Self::hidden_new(order)
    }
}

impl<T, const N: usize> private::Sealed for [Option<T>; N] {}

unsafe impl<T, const N: usize> IsolatedSlotable for [Option<T>; N] {

    type Type = T;

    /// Creates a new array and checks that N == 1 << (order + 1)
    fn new(order: usize) -> Self {
        assert_eq!(1 << (order + 1), const { N }, "N must be equal to 2 ^ (order + 1)");
        core::array::from_fn(|_| None)
    }

    unsafe fn unique_index(&self, index: usize) -> *mut Option<Self::Type> {
        &self[index] as *const Option<T> as *mut Option<T>
    }
}


impl<T> private::Sealed for Box<[UnsafeCell<Option<T>>]> {}

unsafe impl<T> IsolatedSlotable for Box<[UnsafeCell<Option<T>>]> {
    type Type = T;
    fn new(order: usize) -> Self {
        let size = 1 << (order + 1);
        (0..size).map(|_| UnsafeCell::new(None)).collect::<Vec<_>>().into_boxed_slice()
    }
    unsafe fn unique_index(&self, index: usize) -> *mut Option<Self::Type> {
        self[index].get()
    }
}

//(0..size)
                // .map(|_| UnsafeCell::new(None))
                // .collect::<Vec<_>>()
                // .into_boxed_slice()
                
impl<T, I, const S: bool> BoundedQueue<T, I, S>
where 
    I: IsolatedSlotable<Type = T>
{

    pub const MAX_ORDER: usize = 63;
    pub const MIN_ORDER: usize = 0;

    fn hidden_new(order: usize) -> Self {
        // let size = 1 << (order + 1);
        Self {
            backing: I::new(order),
            free_queue: ScqRing::new_full(order),
            alloc_queue: ScqRing::new(order),
            used: CachePadded::new(AtomicUsize::new(0)),
            _type: PhantomData,
        }
    }
    pub fn capacity(&self) -> usize {
        self.free_queue.capacity()
    }
    pub fn enqueue(&self, item: T) -> Result<(), ScqError> {
        // We want to make a call to the internal method here
        // without finalizing. If someone is calling the method
        // with [ScqQueue::enqueue] then it is not part of an unbounded
        // queue.
        self.enqueue_cycle::<false>(item).map_err(|(_, b)| b)
    }

    #[inline(always)]
    fn enqueue_cycle<const FINALIZE: bool>(&self, item: T) -> Result<(), (T, ScqError)> {
        // let current = self.used.fetch_add(1, Acquire);

        // Check if we
        let size = self.used.fetch_add(1, AcqRel);
        if size >= self.free_queue.capacity() {
            // if self.free_queue.capacity() <= self.used.fetch_add(1, AcqRel) {
            self.used.fetch_sub(1, AcqRel);
            return Err((item, ScqError::QueueFull));
        }

        // self.used.fetch_add(1, Acquire);

        let Some(pos) = self.free_queue.dequeue() else {
            self.used.fetch_sub(1, AcqRel);
            return Err((item, ScqError::QueueFull));
        };

        // println!("Enqueing @ {pos}");
        // println!("Accessing {pos}...");

        // println!("[ {pos} ]");
        // let pos = self.free_queue.dequeue().expect("Queue should be able to store 2^(order + 1) items but errored while dequeing a free slot that should have been present.");

        // SAF
        unsafe { *self.backing.unique_index(pos) = Some(item) };
        
        // unsafe { (*self.backing[pos].get()) = Some(item) };
        // std::sync::atomic::fence(std::sync::atomic::Ordering::Release); // prevent reordering
        // unsafe { (&mut *self.backing.get())[pos] = Some(item) };
        // let slot = unsafe { &mut (&mut *self.backing.get())[pos] };

        // SAFETY: No one else has access to this slot.
        // *slot = Some(item);
        // unsafe { *slot.get() = Some(item) };

        if let Err(error) = self.alloc_queue.enqueue(pos) {
            // println!("error: {:?}", error);
            debug_assert_eq!(error, ScqError::QueueFinalized, "Received a queue full notification.");
           
            self.used.fetch_sub(1, AcqRel);
            let item = unsafe { (*self.backing.unique_index(pos)).take() };
            self.free_queue.enqueue(pos).unwrap();
            return Err((item.unwrap(), error));
        }

        // println!("Scheduled @ {pos}");
        if const { FINALIZE } {
            // As described in the paper we must finalize this queue
            // so that nothing more will be added to it.
            if size + 1 >= self.free_queue.capacity() {
                // println!("Hello...");
                self.alloc_queue.finalize();
            }
        }

        // self.alloc_queue.enqueue(pos)?;

        Ok(())
    }
    pub fn dequeue(&self) -> Option<T>
    {
        // println!("Starting dequeue../. (A)");
        let pos = self.alloc_queue.dequeue()?;
        // println!("Position: {pos}");
        // println!("Finishing dequeue... (A)");

        // println!("Pos: {:?}", pos);

        self.used.fetch_sub(1, AcqRel);

        // std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire); // prevent reordering
        let value = unsafe { (*self.backing.unique_index(pos)).take() };
        // std::sync::atomic::fence(std::sync::atomic::Ordering::Release); // prevent reordering

        let Some(value) = value else {
            panic!("Failed to fetch, the position was: {pos}");
        };

        // let value = unsafe { &mut *self.holder[pos].get() }.take().unwrap();

        // println!("{{ {pos} }}");

        // This slot is now freed again!

        if let Err(e) = self.free_queue.enqueue(pos) {
            // println!("{self:?}");

            panic!("ScqError: {e:?}");
        }
        // self.free_queue.enqueue(pos).unwrap();

        Some(value)
    }
}

// =
// const LFRING_MIN: usize = LF_CACHE_SHIFT - 3; // for 64-bit = 3

#[inline(always)]
fn lfring_map(idx: usize, n: usize) -> usize {
    idx & (n - 1)
}


// TODO: Concurrent removal, likely need hazard pointers, read original paper.
// mabe haphazard crate?
// https://karevongeijer.com/blog/lock-free-queue-in-rust/
#[derive(Debug)]
pub struct UnboundedQueueInternal<T> {
    head: CachePadded<HazardAtomicPtr<LscqNode<T>>>,
    tail: CachePadded<HazardAtomicPtr<LscqNode<T>>>,
    internal_order: usize,
    // pointers: ThreadLocal<HazardPointer>
}

/// Stores the actual [ScqQueue] along with a pointer
/// to the next node within the queue. This gives us an
/// unbounded amount of storage.
#[derive(Debug)]
struct LscqNode<T> {
    value: BoundedQueue<T, Box<[UnsafeCell<Option<T>>]>, true>,
    next: haphazard::AtomicPtr<LscqNode<T>>,
}

impl<T> LscqNode<T> {
    /// Allocates a new [LscqNode] object with an order. This
    /// will create a queue with a size of 2 ^ (order + 1).
    pub fn allocate(order: usize) -> NonNull<Self> {
        NonNull::from(Box::leak(Box::new(Self::new(order))))
    }
    fn new(order: usize) -> Self {
        Self {
            next: unsafe { haphazard::AtomicPtr::new(std::ptr::null_mut()) },
            value: BoundedQueue::hidden_new(order),
        }
    }
}

#[inline(always)]
fn yield_marker() {
    // std::thread::yield_now();
    #[cfg(loom)]
    loom::thread::yield_now();
}


type HazardAtomicPtr<T> = haphazard::AtomicPtr<T>;

// TODO: Implement finalization properly.

/// Checks the null alignment.
/// https://gitlab.com/bzim/lockfree/-/blob/master/src/ptr.rs?ref_type=heads
#[inline(always)]
pub fn check_null_align<T>() {
    debug_assert!(null_mut::<T>() as usize % align_of::<T>() == 0);
}

impl<T: Send + Sync> UnboundedQueueInternal<T> {
    /// Creates a new [LcsqQueue] with a single internal ring.
    pub fn new(segment_order: usize) -> Self {
        let queue = LscqNode::allocate(segment_order);

        let queue_ptr = queue.as_ptr();
        // println!("initial {:?}", queue_ptr);

        // Check the null alignment of the `LscqNode`.
        check_null_align::<LscqNode<T>>();
        Self {
            head: unsafe { HazardAtomicPtr::new(queue_ptr) }.into(),
            tail: unsafe { HazardAtomicPtr::new(queue_ptr) }.into(),
            internal_order: segment_order
        }
    }
    /// Enqueues an element of type `T` into the queue.
    pub fn enqueue(&self, hp: &mut HazardPointer<'_>, mut element: T)
    {
        loop {
            let cq_ptr = self.tail.safe_load(hp).unwrap();
            // let cq_ptr = self.tail.load(Acquire);
            // SAFETY: The tail can never be a null pointer so we can
            // safely dereference. Additionally, there are never any mutabkle
            // pointers to this memory address.
            // println!("Accessing {cq_ptr:?} (D)");
            // let cq = unsafe { &*cq_ptr };

            // If the next pointer is not null then we want to keep
            // going next until we have found the end.
            let next_ptr = cq_ptr.next.load_ptr();
            if !next_ptr.is_null() {
                unsafe {
                    let _ = self
                    .tail
                    .compare_exchange_ptr(cq_ptr as *const LscqNode<T> as *mut LscqNode<T>, next_ptr);
                }
                
                yield_marker();
                continue;
            }

            // Attempts to enqueue the item, we should finalize the
            // queue if we can.
            match cq_ptr.value.enqueue_cycle::<true>(element) {
                Ok(_) => {
                    // We are done and have succesfully enqueued the item.
                    return;
                }
                Err((elem, _)) => {
                    // We do not care that we failed here, we just want to
                    // return the value to the element flow so that the borrow checker will be happy.
                    element = elem;
                }
            }

            // Allocate a new queue.
            // std::sync::atomic::fence(Acquire);

            let ncq = LscqNode::new(self.internal_order);
            ncq.value
                .enqueue(element)
                .expect("Freshly allocated queue could not accept one value.");
            // The forget_inner prevents miri from detecting data race?
            let ncq = NonNull::from(Box::leak(Box::new(ncq)));
            // println!("Allocated: {:?}", ncq);
            // std::sync::atomic::compiler_fence(SeqCst);
            // std::sync::atomic::fence(SeqCst);

            // Try to insert a new tail into the queue.
            if unsafe { cq_ptr
                .next
                .compare_exchange_ptr(null_mut(), ncq.as_ptr())
                .is_ok() }
            {
                // Correct the list ordering.
                let _ = unsafe { self
                    .tail
                    .compare_exchange_ptr(cq_ptr as *const LscqNode<T> as *mut LscqNode<T>, ncq.as_ptr()) };
                // NOTE: We do not have to free the allocation here
                // because we haave succesfully put it into the list.
                return;
            }

            // Extract the first element so the borrow checker is happy and we
            // can avoid clones.
            // SAFETY: This is the only instance of this pointer
            // and it is non-null because we allocated it with `OwnedAlloc`.
            // println!("Accessing {ncq:?} (E)");
            let Some(value) = (unsafe { ncq.as_ref().value.dequeue() }) else {
                panic!("Failed to access previously enqueued value.")
            };
            element = value;
            // element = unsafe { ncq.as_ref().value.dequeue().unwrap() };

            // SAFETY: The allocation was created with `OwnedAlloc` and we
            // have unique access to it.
            unsafe {
                free_owned_alloc(ncq.as_ptr());
            }

            yield_marker();
        }
    }
    /// Removes an element from the queue returning an [Option]. If the queue
    /// is empty then the returned value will be [Option::None].
    pub fn dequeue(&self, hp:&mut HazardPointer<'_>, next: &mut HazardPointer  ) -> Option<T>
    {
        loop {
            // SAFETY: The head node will always be a non-null node.
            // let head_ptr = self.head;
            let cq_ptr = self.head.safe_load(hp).unwrap();
            // SAFETY: The pointer is non-null and there are only immutable
            // references to it. Additionlly, since it is the head, there are
            // no mutable references to this memory location.
            // println!("Accessing {cq_ptr:?} (A)");
            // let cq = unsafe { &*cq_ptr.as_ptr() };

            // Dequeue an entry.
            // println!("Enteirng...");
            let mut p = cq_ptr.value.dequeue();
            // println!("Existing...");
            if p.is_some() {
                // The entry actually holds a value, in this case,
                // we can just return the value.
                return p;
            }

            // If the next pointer is null then we have nothing
            // to dequeue and thus we can just return [Option::None].
            if cq_ptr.next.safe_load(next).is_none() {
                return None;
            }
            // Update the threshold.
            cq_ptr.value
                .alloc_queue
                .threshold
                .store(3 * (1 << (cq_ptr.value.free_queue.order + 1)) - 1, SeqCst);

            // Try dequeing again.
            p = cq_ptr.value.dequeue();
            if p.is_some() {
                return p;
            }

            
            if let Ok(mut ok) = unsafe { self.head.compare_exchange_ptr(cq_ptr as *const LscqNode<T> as *mut LscqNode<T>, cq_ptr.next.load_ptr()) } {
                unsafe { ok.take().unwrap().retire() };
            }

            // Here we remove the SCQ.
            // let cq_ptr = unsafe { NonNull::new_unchecked(self.head.load(Acquire)) };
            // unsafe { self.free_front(cq_ptr, &pause) }?;
            // unsafe { self.head.retire() };

            yield_marker();
            // println!("injunctive");
        }
    }
}

impl<T> Drop for UnboundedQueueInternal<T> {
    /// Drops the queue, deallocating all the nodes. Requires
    /// exclusive access to the queue.
    fn drop(&mut self) {
        let mut next = self.head.load_ptr();
        while !next.is_null() {
            // SAFETY: If drop is called then we have exclusive access
            // to this strucutture. TODO: Improve docs with inspiration from lockfree crate.
            unsafe {
                let temp = next;
                next = (*next).next.load_ptr();
                free_owned_alloc(temp);
            }
        }
    }
}

/// A enqueue handle to an [UnboundedQueue], allowing for
/// bulk enqueues efficiently. Internally, the unbounded queue manages
/// the queue of bounded queues with hazard pointers to avoid the ABA problem, this
/// allows minimizing the creation of these hazard pointers.
/// 
/// # Example
/// ```
/// use lfqueue::UnboundedQueue;
/// 
/// let queue = UnboundedQueue::<usize>::new(2);
/// let mut handle = queue.enqueue_handle();
/// handle.enqueue(3);
/// ```
pub struct UnboundedEnqueueHandle<'a, T> {
    internal: &'a UnboundedQueueInternal<T>,
    primary: HazardPointer<'static>
}

impl<'a, T> UnboundedEnqueueHandle<'a, T>
where 
    T: Send + Sync
{
    /// Enqueues an item on the underlying queue.
    /// 
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedEnqueueHandle, UnboundedQueue};
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// let mut handle = queue.enqueue_handle();
    /// handle.enqueue(3);
    /// ```
    pub fn enqueue(&mut self, item: T) {
        self.internal.enqueue(&mut self.primary, item);
    }
}

/// A full handle to an [UnboundedQueue], allowing for
/// bulk enqueues efficiently. Internally, the unbounded queue manages
/// the queue of bounded queues with hazard pointers to avoid the ABA problem, this
/// allows minimizing the creation of these hazard pointers.
/// 
/// # Example
/// ```
/// use lfqueue::UnboundedQueue;
/// 
/// let queue = UnboundedQueue::<usize>::new(2);
/// let mut handle = queue.full_handle();
/// handle.enqueue(3);
/// 
/// assert_eq!(handle.dequeue(), Some(3));
/// assert!(handle.dequeue().is_none());
/// ```
pub struct UnboundedFullHandle<'a, T> {
    enqueue: UnboundedEnqueueHandle<'a, T>,
    secondary: HazardPointer<'static>
}

impl<'a, T> UnboundedFullHandle<'a, T>
where 
    T: Send + Sync
{
    /// Enqueues an item on the underlying queue.
    /// 
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedFullHandle, UnboundedQueue};
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// let mut handle = queue.full_handle();
    /// handle.enqueue(3);
    /// ```
    pub fn enqueue(&mut self, item: T) {
        self.enqueue.internal.enqueue(&mut self.enqueue.primary, item);
    }
    /// Enqueues an item on the underlying queue.
    /// 
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedFullHandle, UnboundedQueue};
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// let mut handle = queue.full_handle();
    /// handle.enqueue(3);
    /// 
    /// assert_eq!(handle.dequeue(), Some(3));
    /// ```
    pub fn dequeue(&mut self) -> Option<T> {
        self.enqueue.internal.dequeue(&mut self.enqueue.primary, &mut self.secondary)
    }
}


/// An unbounded lock-free queue. This is the LCSQ from the ACM paper,
/// "A Scalable, Portable, and Memory-Efficient Lock-Free FIFO Queue" by Ruslan Nikolaev.
/// 
/// Internally, it manages a linked list of bounded queue types and this allows it
/// to grow in an unbounded manner. Since with a reasonable order new queue creation and deletion
/// should be sparse, the operation cost is largely dominated by the internal queues and thus is still
/// extremely fast.
/// 
/// # Example
/// ```
/// use lfqueue::UnboundedQueue;
/// 
/// let queue = UnboundedQueue::<usize>::new(2);
/// queue.enqueue(4);
/// queue.enqueue(5);
/// 
/// assert_eq!(queue.dequeue(), Some(4));
/// assert_eq!(queue.dequeue(), Some(5));
/// ```
pub struct UnboundedQueue<T> {
    internal: UnboundedQueueInternal<T>,
    order: usize
}

unsafe impl<T: Send + Sync> Send for UnboundedQueue<T> {}
unsafe impl<T: Send + Sync> Sync for UnboundedQueue<T> {}

impl<T> UnboundedQueue<T>
where 
    T: Send + Sync
{
    /// Creates a new [UnboundedQueue] with an initial ring order of `order`. This means
    /// each queue segment has a size of 2 ^ order.
    /// 
    /// # Examples
    /// ```
    /// use lfqueue::UnboundedQueue;
    /// 
    /// let queue = UnboundedQueue::<()>::new(2);
    /// assert_eq!(queue.base_segment_capacity(), 4);
    /// ```
    pub fn new(order: usize) -> Self {
        Self {
            internal: UnboundedQueueInternal::new(order),
            order
        }
    }
    /// Returns the base segment capacity of the [UnboundedQueue], this is the
    /// capacity of the base ring. Or in other words, 2 ^ order.
    /// 
    /// # Examples 
    /// ```
    /// use lfqueue::UnboundedQueue;
    /// 
    /// let queue = UnboundedQueue::<()>::new(2);
    /// assert_eq!(queue.base_segment_capacity(), 4);
    /// ```
    pub fn base_segment_capacity(&self) -> usize {
        1 << self.order
    }
    /// Creates an [UnboundedEnqueueHandle] that allows for the execution of
    /// many 
    /// 
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedEnqueueHandle, UnboundedQueue};
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// let mut handle = queue.enqueue_handle();
    /// handle.enqueue(3);
    /// handle.enqueue(4);
    /// ```
    pub fn enqueue_handle(&self) -> UnboundedEnqueueHandle<'_, T> {
        UnboundedEnqueueHandle {
            internal: &self.internal,
            primary: HazardPointer::new()
        }
    }
    pub fn full_handle(&self) -> UnboundedFullHandle<'_, T> {
        UnboundedFullHandle { enqueue: self.enqueue_handle(), secondary: HazardPointer::new() }
    }
    /// Enqueues a single entry. Internally, this just creates an [UnboundedEnqueueHandle] and
    /// performs a single enqueue operation. If you intend to do several enqueues in a row, please
    /// see [UnboundedQueue::enqueue_handle].
    /// 
    /// # Example
    /// ```
    /// use lfqueue::UnboundedQueue;
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// queue.enqueue(4);
    /// ```
    pub fn enqueue(&self, item: T)
    {
        self.enqueue_handle().enqueue(item);
    }

    /// Dequeues a single entry. Internally, this just creates an [UnboundedFullHandle] and performs
    /// a single dequeue operation. If you intend to do several dequeues in a row, please see
    /// [UnboundedQueue::full_handle].
    /// 
    /// # Example
    /// ```
    /// use lfqueue::UnboundedQueue;
    /// 
    /// let queue = UnboundedQueue::<usize>::new(2);
    /// queue.enqueue(2);
    /// 
    /// assert_eq!(queue.dequeue(), Some(2));
    /// ```
    pub fn dequeue(&self) -> Option<T>
    {
       self.full_handle().dequeue()
    }
}

/// Frees memory allocated by an [Box]. This uses manual ma
///
/// # Safety
/// This ptr must have been produced with [Box::leak] and represent
/// a valid pointer to initialized memory.
unsafe fn free_owned_alloc<T>(ptr: *mut T) {
    // SAFETY: The pointer is non-null and represents a valid
    // pointer to initialized memory and was constructed via a box allocation.
    drop(unsafe { Box::from_raw(ptr)  });
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;

    use lockfree::queue;

    use crate::scq::{
        lfring_signed_cmp, AllocBoundedQueue, BoundedQueue, ScqError, ScqRing, UnboundedQueue
    };

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

    

    #[cfg(not(loom))]
    #[test]
    pub fn lcsq_circus() {
        use std::sync::{Arc, Barrier};

        use crate::scq::AllocBoundedQueue;

        let context = Arc::new(AllocBoundedQueue::new(3));
        loop {
            
        let threads = 50;
        let thread_runs = 100;

        // let barrier = Arc::new(Barrier::new(threads));

        // let barrier_finalized = Arc::new(Barrier::new(threads + 1));
        let mut handles = vec![];
        for _ in 0..threads {
            handles.push(std::thread::spawn({
                // let barrier_finalized = barrier_finalized.clone();
                let context = context.clone();
                // let barrier = barrier.clone();
                move || {
                    // barrier.wait();
                    // println!("hello");
                    for i in 0..thread_runs {
                        // println!("start queue...");
                        let _ = context.enqueue(i);
                        // println!("end queue...");
                        // context.lock().unwrap().push_back(std::hint::black_box(i));
                    }
                    for _ in 0..thread_runs {
                        // println!("Hello");
                        context.dequeue();
                        // println!("Bye...");
                        // dequeue(&context);
                        // context.lock().unwrap().pop_front();
                    }
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }
        break;
        }

        // barrier_finalized.wait();
        // println!("Done");
    }


    #[test]
    pub fn verify_small_queue_correctness() {

        fn queue_harness( order: usize) {
            let queue = AllocBoundedQueue::new(order);
            assert_eq!(queue.capacity(), 1 << order);
            for i in 0..queue.capacity() {
                queue.enqueue(i).unwrap();
            }
            for i in 0..queue.capacity() {
                assert_eq!(queue.dequeue(), Some(i));
            }
            for i in 0..queue.capacity() {
                assert_eq!(queue.dequeue(), None);
            }
        }

        // Weird effects happen at small queue sized, this checks for that.
        queue_harness(3);
        queue_harness(2);
        queue_harness(1);
        queue_harness(0);



    }

    #[test]
    #[cfg(not(loom))]
    pub fn test_const_queue() {
        use crate::ConstBoundedQueue;

        let queue = ConstBoundedQueue::<usize, 4>::new_const_queue();
        assert!(queue.enqueue(1).is_ok());
        assert!(queue.enqueue(2).is_ok());
        assert_eq!(queue.enqueue(3), Err(ScqError::QueueFull));
        panic!("hello: {}", queue.capacity());

    }

    #[test]
    #[cfg(not(loom))]
    pub fn check_scq_fill() {
        let mut fill = AllocBoundedQueue::new(3);
        for i in 0..8 {
            fill.enqueue(i).unwrap();
        }
        assert_eq!(fill.enqueue(0), Err(ScqError::QueueFull));
    }





    #[cfg(not(loom))]
    #[test]
    pub fn fullinit() {
        let ring = ScqRing::<false>::new_full(3);

        for i in 0..8 {
            assert_eq!(ring.dequeue(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_lcsq_ptr() {
        let ring = UnboundedQueue::new(3);
        for i in 0..16 {
            ring.enqueue(i);
        }

        // println!("RING: {:?}", ring);

        for i in 0..16 {
            assert_eq!(ring.dequeue(), Some(i));
            // println!("HELLO: {:?}", ring.dequeue());
        }
        // println!("HELLO: {:?}", ring.dequeue());
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_length_function() {
        let ring = ScqRing::<false>::new(3);
        assert_eq!(ring.capacity(), 8);

        // println!(
        //     "Threshold: {}",
        //     ring.threshold.load(std::sync::atomic::Ordering::SeqCst)
        // );

        ring.enqueue(3).unwrap();
        println!(
            "Threshold: {}",
            ring.threshold.load(std::sync::atomic::Ordering::SeqCst)
        );
        ring.enqueue(2).unwrap();
        println!(
            "Threshold: {}",
            ring.threshold.load(std::sync::atomic::Ordering::SeqCst)
        );
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
    pub fn scqqueue_enq_deq() -> Result<(), ScqError> {
        let holder = AllocBoundedQueue::new(3);
        holder.enqueue("A")?;
        holder.enqueue("B")?;
        println!("Enqueued items.");

        // println!("Holder: {:?}", holder);

        assert_eq!(holder.dequeue(), Some("A"));
        println!("pOpped");
        assert_eq!(holder.dequeue(), Some("B"));

        assert_eq!(holder.dequeue(), None);

        Ok(())
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_lfring_ptrs_full() {
        // let ring = ScqRing::new(3);
        // ring.enqueue(0x22cf301a020);
        // println!("Value: {:?}", ring);
        // assert_eq!(ring.dequeue(), Some(0x22cf301a020));

        let holder: AllocBoundedQueue<&str> = AllocBoundedQueue::new(3);
        println!("Holder: {:?}", holder);
        for _ in 0..holder.alloc_queue.capacity() {
            holder.enqueue("A").unwrap();
        }

        assert_eq!(holder.enqueue("I"), Err(ScqError::QueueFull));
        // holder.enqueue("J").unwrap();
        // holder.enqueue("K").unwrap();
        // holder.enqueue("L").unwrap();

        // holder.enqueue("hello").unwrap();
        // println!("Holder: {:?}", holder);
        // holder.enqueue("wow").unwrap();

        // assert_eq!(holder.dequeue(), Some("hello"));
        // assert_eq!(holder.dequeue(), Some("wow"));
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_lfring_basic() {
        let ring = ScqRing::<false>::new(3);

        // ring.enqueue(303030).unwrap();

        for i in 0..ring.capacity() {
            ring.enqueue(i).unwrap();
        }

        for i in 0..ring.capacity() {
            assert_eq!(ring.dequeue(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    pub fn initialize_full_correctly() {
        use std::sync::atomic::Ordering;

        let mut ring = ScqRing::<false>::new(3);
        for i in 0..ring.capacity() {
            ring.enqueue(i).unwrap();
        }

        // println!("Capacity: {:?}", auto.capacity());

        // auto.enqueue(1).unwrap();

        

        // println!("Value: {:?}", auto.dequeue());

        let mut auto = ScqRing::new_full(3);
        
        assert_eq!(ring, auto);

        // for i in 0..16 {
        //     println!("Value: {:?}", auto.dequeue());
        // }

        // auto = ring;

        // println!("Manual: {:?}", ring);
        // println!("Auto: {:?}", auto);

        for i in 0..auto.capacity() {
            // println!("I: {i}");
            // let value = ring.enqueue(eidx)
            let value = auto.dequeue();
            assert_eq!(value, Some(i));
            // println!("Enqueing {value:?}...");
            auto.enqueue(value.unwrap()).unwrap();
        }

        // println!("Extra: {:?}", auto.dequeue());
        // println!("Extra: {:?}", auto.dequeue());
        // println!("Extra: {:?}", auto.dequeue());
        for i in 0..auto.capacity() {
            assert_eq!(auto.dequeue(),Some(i));
            // println!("Dequeing... {:?}", auto.dequeue());
            // assert_eq!(auto.dequeue(), Some((ring.capacity() - 1) - i));
        }


    }
}
