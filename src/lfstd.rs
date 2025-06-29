use core::{
    cell::UnsafeCell, fmt::Debug, marker::PhantomData, ptr::{null_mut, NonNull}
};

use crossbeam_utils::CachePadded;
use haphazard::{Domain, HazardPointer, Singleton};

use crate::atomics::*;
use crate::{
    BoundedQueue,
    scq::{
        ScqAlloc, ScqRing, determine_order, initialize_atomic_array_full, lfring_pow2,
        lfring_threshold3, yield_marker,
    },
};



/// The internal bounded queue type managed by an allocated buffer.
pub(crate) type AllocBoundedQueueInternal<T, const MODE: usize> =
    BoundedQueue<T, Box<[UnsafeCell<Option<T>>]>, Box<[CachePadded<AtomicUsize>]>, MODE>;

/// Allocates an array of [AtomicUsize] that is cache padded of size `n`. This
/// is used for the initializaton methods for the [ScqRing].
#[inline]
fn allocate_standard_atomic_array(n: usize) -> Box<[CachePadded<AtomicUsize>]> {
    (0..n)
        .map(|_| AtomicUsize::new((-1isize) as usize))
        .map(CachePadded::new)
        .collect()
}

impl<const MODE: usize> AllocScqRing<MODE> {
    /// Creates a new empty allocated ring.
    pub fn new_alloc_ring_empty(order: usize) -> Self {
        Self::new_from_sqalloc(order, allocate_atomic_array_empty(order))
    }
    /// Creates a new full allocated ring with values ranging
    /// from 0 to n.
    pub fn new_alloc_ring_full(order: usize) -> Self {
        Self::new_from_sqalloc(order, allocate_atomic_array_full(order))
    }
}

/// Allocates an array of atomics with the value of `-1` of a specific
/// order.
fn allocate_atomic_array_full(order: usize) -> ScqAlloc<Box<[CachePadded<AtomicUsize>]>> {
    let half = lfring_pow2(order);
    let n = half * 2;

    // Initialize an array of
    let vector = allocate_standard_atomic_array(n);

    initialize_atomic_array_full(&vector, half, n);

    ScqAlloc {
        array: vector,
        tail: half,
        thresh: lfring_threshold3(half, n) as isize,
    }
}

/// Allocates an empty atomic array.
fn allocate_atomic_array_empty(order: usize) -> ScqAlloc<Box<[CachePadded<AtomicUsize>]>> {
    let n = lfring_pow2(order + 1);
    let array = allocate_standard_atomic_array(n);
    ScqAlloc {
        array,
        tail: 0,
        thresh: -1,
    }
}

/// A bounded SCQ queue backed by an allocated array.
///
/// # Example
/// ```
/// use lfqueue::AllocBoundedQueue;
///
/// let queue = AllocBoundedQueue::<usize>::new(8);
/// assert!(queue.enqueue(1).is_ok());
/// ```
pub type AllocBoundedQueue<T> = AllocBoundedQueueInternal<T, 0>;

impl crate::scq::private::Sealed for Box<[CachePadded<AtomicUsize>]> {}
impl<T> crate::scq::private::Sealed for Box<[UnsafeCell<Option<T>>]> {}

/// An SCQ ring backed by an allocation.
type AllocScqRing<const MODE: bool> = ScqRing<Box<[CachePadded<AtomicUsize>]>, MODE>;

impl<T, const MODE: usize> AllocBoundedQueueInternal<T, MODE> {
    /// Allocates a bounded queue, this method backs most of he
    pub fn new(size: usize) -> Self {
        let order = determine_order(size);
        // let size = 1 << (order + 1);
        Self {
            alloc_queue: ScqRing::new_alloc_ring_empty(order),
            free_queue: ScqRing::new_alloc_ring_full(order),
            backing: (0..(2 * size))
                .map(|_| UnsafeCell::new(None))
                .collect::<Box<_>>(),
            used: CachePadded::new(AtomicUsize::new(0)),
            _type: PhantomData,
        }
    }
}



/// The internals of the LCSQ queue, holds the atomic pointers. These are from the `haphazard` crate by
/// John Gjengset, which is a great way of implementing hazard pointers to address the ABA problem.
///
/// The structure contains a head and tail and is a basic queue of [BoundedQueue]. Although this queue is
/// slightly less efficient than the highly efficient [BoundedQueue] implementation, the asymptotic complexity
/// is largely dominated by the bounded queue.
///
/// # References
/// Hazard Pointers & ABA problem, https://karevongeijer.com/blog/lock-free-queue-in-rust/
#[derive(Debug)]
struct UnboundedQueueInternal<T> {
    head: CachePadded<HazardAtomicPtr<LscqNode<T>>>,
    tail: CachePadded<HazardAtomicPtr<LscqNode<T>>>,
    internal_order: usize,
}

/// Stores the actual [BoundedQueue] along with a pointer
/// to the next node within the queue. This gives us an
/// unbounded amount of storage.
#[derive(Debug)]
struct LscqNode<T> {
    value: AllocBoundedQueueInternal<T, 1>,
    next: haphazard::AtomicPtr<LscqNode<T>, QueueDomain>,
}

impl<T> LscqNode<T> {
    /// Allocates a new [LscqNode] object with an order. This
    /// will create a queue with a size of 2 ^ (order + 1).
    pub fn allocate(size: usize) -> NonNull<Self> {
        // SAFETY: the pointer is allocated w/ Box and thus
        // it is not null.
        unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(Self::new(size)))) }
    }
    fn new(size: usize) -> Self {
    
        Self {
        
            next: unsafe { haphazard::AtomicPtr::new(std::ptr::null_mut()) },
            value: BoundedQueue::new(size),
        }
    }
}

/// The [haphazard::AtomicPtr] type which facilitates using atomic pointers
/// with hazard pointers for safe memory management.
type HazardAtomicPtr<T> = haphazard::AtomicPtr<T, QueueDomain>;

/// Checks the null alignment.
/// https://gitlab.com/bzim/lockfree/-/blob/master/src/ptr.rs?ref_type=heads
#[inline(always)]
pub fn check_null_align<T>() {
    debug_assert!(null_mut::<T>() as usize % align_of::<T>() == 0);
}

impl<T: Send + Sync> UnboundedQueueInternal<T> {
    /// Creates a new [UnboundedQueueInternal] with a single internal ring.
    pub fn new(segment_size: usize) -> Self {
        let queue = LscqNode::allocate(segment_size);

        // Get the actual pointer to set the pointer.
        let queue_ptr = queue.as_ptr();

        check_null_align::<LscqNode<T>>();
        Self {
            // SAFETY: the `haphazard` crate specifies several
            // safety contracts that must be upheld:
            // 1. `queue_ptr` references a valid `LscqNode` since it
            // was allocated through box.
            // 2. `queue_ptr` was allocated using the pointer type `LscqNode`.
            // 3. `queue_ptr` will only be dropped through [haphazard::Domain::retire_ptr].
            head: unsafe { HazardAtomicPtr::new(queue_ptr) }.into(),
            tail: unsafe { HazardAtomicPtr::new(queue_ptr) }.into(),
            internal_order: segment_size,
        }
    }
    /// Enqueues an element of type `T` into the queue.
    pub fn enqueue(&self, hp: &mut HazardPointer<'_, QueueDomain>, mut element: T) {
        loop {
            // Load the tail. This will never be null so we can just
            // unwrap it here.
            let cq_ptr = self.tail.safe_load(hp).unwrap();

            // If the next pointer is not null then we want to keep
            // going next until we have found the end.
            let next_ptr = cq_ptr.next.load_ptr();
            if !next_ptr.is_null() {
                // SAFETY: the next pointer was constructed through a [Box] allocation
                // and thus all the same safety requirements as `haphazard::AtomicPtr::new` are
                // satisfied.
                unsafe {
                    let _ = self.tail.compare_exchange_ptr(
                        cq_ptr as *const LscqNode<T> as *mut LscqNode<T>,
                        next_ptr,
                    );
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

            let ncq = LscqNode::new(self.internal_order);
            ncq.value
                .enqueue(element)
                .expect("Freshly allocated queue could not accept one value.");

            // SAFETY: `ncq` is allocated by box, thus the pointer is not null.
            let ncq = unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(ncq))) };

            // Try to insert a new tail into the queue.
            // SAFETY: the next pointer was constructed through a [Box] allocation
            // and thus all the same safety requirements as `haphazard::AtomicPtr::new` are
            // satisfied.
            if unsafe {
                cq_ptr
                    .next
                    .compare_exchange_ptr(null_mut(), ncq.as_ptr())
                    .is_ok()
            } {
                // Correct the list ordering.
                let _ = unsafe {
                    self.tail.compare_exchange_ptr(
                        cq_ptr as *const LscqNode<T> as *mut LscqNode<T>,
                        ncq.as_ptr(),
                    )
                };
                // NOTE: We do not have to free the allocation here
                // because we haave succesfully put it into the list.
                return;
            }

            // Extract the first element so the borrow checker is happy and we
            // can avoid clones.
            // SAFETY: This is the only instance of this pointer
            // and it is non-null because we allocated it with `Box`.
            let Some(value) = (unsafe { ncq.as_ref().value.dequeue() }) else {
                panic!("Failed to access previously enqueued value.")
            };
            element = value;

            // SAFETY: The allocation was created with `Box` and we
            // have unique access to it. This is sound because it
            // is not part of any atomic pointer and thus we can
            // manually free it.
            unsafe {
                free_owned_alloc(ncq.as_ptr());
            }

            yield_marker();
        }
    }
    /// Removes an element from the queue returning an [Option]. If the queue
    /// is empty then the returned value will be [Option::None].
    pub fn dequeue(&self, hp: &mut HazardPointer<'_, QueueDomain>, next: &mut HazardPointer<'_, QueueDomain>, domain: &Domain<QueueDomain>) -> Option<T> {
        loop {
            // SAFETY: The head node will always be a non-null node.
            let cq_ptr = self.head.safe_load(hp).unwrap();

            // Dequeue an entry.
            if let Some(p) = cq_ptr.value.dequeue() {
                // The entry actually holds some value, so we just
                // return it here.
                return Some(p);
            }

            // If the next pointer is null then we have nothing
            // to dequeue and thus we can just return [Option::None].
            let next_ptr = cq_ptr.next.safe_load(next)?;

            // Update the threshold.
            cq_ptr
                .value
                .alloc_queue
                .threshold
                .store(3 * (1 << (cq_ptr.value.free_queue.order + 1)) - 1, SeqCst);

            // Try dequeing again.
            if let Some(p) = cq_ptr.value.dequeue() {
                // The entry actually holds some value, so we just
                // return it here.
                return Some(p);
            }

            // SAFETY: the next pointer was constructed through a [Box] allocation
            // and thus all the same safety requirements as `haphazard::AtomicPtr::new` are
            // satisfied.
            if let Ok(mut ok) = unsafe {
                self.head.compare_exchange_ptr(
                    cq_ptr as *const LscqNode<T> as *mut LscqNode<T>,
                    next_ptr as *const LscqNode<T> as *mut LscqNode<T>,
                )
            } {
                // SAFETY: The haphazard crate requires us to uphold two
                // principal safety contracts:
                // 1. The pointed-to object will never again be returned by
                // any [`AtomicPtr::load`]. Since we've swapped it OUT of the list, it
                // is not possible for it to be loaded again and thus we can retire it.
                // 2. The pointed-to object has not been retired yet, by (1) we see
                // that we were the ones to remove it from the list and thus arrive here.
                unsafe { ok.take().unwrap().retire_in(&domain) };
            }

            yield_marker();
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
            // to this structure. All pointers were allocated by
            // [Box] and as a result of the above `null` check they
            // are not null.
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
/// let queue = UnboundedQueue::<usize>::new();
/// let mut handle = queue.enqueue_handle();
/// handle.enqueue(3);
/// ```
pub struct UnboundedEnqueueHandle<'a, T> {
    internal: &'a UnboundedQueueInternal<T>,
    primary: HazardPointer<'a, QueueDomain>,
}


impl<'a, T> UnboundedEnqueueHandle<'a, T>
where
    T: Send + Sync,
{
    /// Enqueues an item on the underlying queue.
    ///
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedEnqueueHandle, UnboundedQueue};
    ///
    /// let queue = UnboundedQueue::<usize>::new();
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
/// let queue = UnboundedQueue::<usize>::new();
/// let mut handle = queue.full_handle();
/// handle.enqueue(3);
///
/// assert_eq!(handle.dequeue(), Some(3));
/// assert!(handle.dequeue().is_none());
/// ```
pub struct UnboundedFullHandle<'a, T> {
    enqueue: UnboundedEnqueueHandle<'a, T>,
    secondary: HazardPointer<'a, QueueDomain>,
    domain: &'a Domain<QueueDomain>
}

impl<'a, T> UnboundedFullHandle<'a, T>
where
    T: Send + Sync,
{
    /// Enqueues an item on the underlying queue.
    ///
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedFullHandle, UnboundedQueue};
    ///
    /// let queue = UnboundedQueue::<usize>::new();
    /// let mut handle = queue.full_handle();
    /// handle.enqueue(3);
    /// ```
    pub fn enqueue(&mut self, item: T) {
        self.enqueue
            .internal
            .enqueue(&mut self.enqueue.primary, item);
    }
    /// Enqueues an item on the underlying queue.
    ///
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedFullHandle, UnboundedQueue};
    ///
    /// let queue = UnboundedQueue::<usize>::new();
    /// let mut handle = queue.full_handle();
    /// handle.enqueue(3);
    ///
    /// assert_eq!(handle.dequeue(), Some(3));
    /// ```
    pub fn dequeue(&mut self) -> Option<T> {
        self.enqueue
            .internal
            .dequeue(&mut self.enqueue.primary, &mut self.secondary,self.domain)
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
/// let queue = UnboundedQueue::<usize>::new();
/// queue.enqueue(4);
/// queue.enqueue(5);
///
/// assert_eq!(queue.dequeue(), Some(4));
/// assert_eq!(queue.dequeue(), Some(5));
/// ```
pub struct UnboundedQueue<T> {
    internal: UnboundedQueueInternal<T>,
    size: usize,
    domain: Domain<QueueDomain>
}


impl<T: Debug> core::fmt::Debug for UnboundedQueue<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("UnboundedQueue")
         .field("internal", &self.internal)
         .field("size", &self.size)
         .field("domain", &"Domain")
         .finish()
    }
}

/// The domain for queues.
#[non_exhaustive]
struct QueueDomain;

unsafe impl Singleton for QueueDomain {}

unsafe impl<T: Send + Sync> Send for UnboundedQueue<T> {}
unsafe impl<T: Send + Sync> Sync for UnboundedQueue<T> {}

impl<T> UnboundedQueue<T>
where
    T: Send + Sync,
{
    /// Creates a new [UnboundedQueue] with a default
    /// segment size of 32.
    ///
    /// # Examples
    /// ```
    /// use lfqueue::UnboundedQueue;
    ///
    /// let queue = UnboundedQueue::<()>::new();
    /// assert_eq!(queue.base_segment_capacity(), 32);
    /// ```
    pub fn new() -> Self {
        
        Self::with_segment_size(32)
    }
    /// Creates a new [UnboundedQueue] with a particular segment size. The segment
    /// size determines the size of each of the internal bounded queues. This method exists
    /// to allow developers to customize the internal sizes of the queues.
    ///
    /// # Examples
    /// ```
    /// use lfqueue::UnboundedQueue;
    ///
    /// let queue = UnboundedQueue::<()>::with_segment_size(4);
    /// assert_eq!(queue.base_segment_capacity(), 4);
    /// ```
    pub fn with_segment_size(size: usize) -> Self {
        let domain = Domain::new(&QueueDomain);
        Self {
            internal: UnboundedQueueInternal::new(size),
            size,
            domain
        }
    }
    /// Returns the base segment size of the unbounded queue.
    /// 
    /// # Examples
    /// ```
    /// use lfqueue::UnboundedQueue;
    ///
    /// let queue = UnboundedQueue::<()>::with_segment_size(4);
    /// assert_eq!(queue.base_segment_capacity(), 4);
    /// ```
    pub fn base_segment_capacity(&self) -> usize {
        self.size
    }
    /// Creates an [UnboundedEnqueueHandle] that allows for the execution of
    /// many
    ///
    /// # Example
    /// ```
    /// use lfqueue::{UnboundedEnqueueHandle, UnboundedQueue};
    ///
    /// let queue = UnboundedQueue::<usize>::new();
    /// let mut handle = queue.enqueue_handle();
    /// handle.enqueue(3);
    /// handle.enqueue(4);
    /// ```
    pub fn enqueue_handle(&self) -> UnboundedEnqueueHandle<'_, T> {
        UnboundedEnqueueHandle {
            internal: &self.internal,
            primary: HazardPointer::new_in_domain(&self.domain)
        }
    }
    pub fn full_handle(&self) -> UnboundedFullHandle<'_, T> {
        UnboundedFullHandle {
            enqueue: self.enqueue_handle(),
            secondary: HazardPointer::new_in_domain(&self.domain),
            domain: &self.domain
        }
    }
    /// Enqueues a single entry. Internally, this just creates an [UnboundedEnqueueHandle] and
    /// performs a single enqueue operation. If you intend to do several enqueues in a row, please
    /// see [UnboundedQueue::enqueue_handle].
    ///
    /// # Example
    /// ```
    /// use lfqueue::UnboundedQueue;
    ///
    /// let queue = UnboundedQueue::<usize>::new();
    /// queue.enqueue(4);
    /// ```
    pub fn enqueue(&self, item: T) {
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
    /// let queue = UnboundedQueue::<usize>::new();
    /// queue.enqueue(2);
    ///
    /// assert_eq!(queue.dequeue(), Some(2));
    /// ```
    pub fn dequeue(&self) -> Option<T> {
        self.full_handle().dequeue()
    }
}

/// Frees memory allocated by an [Box]. This uses manual ma
///
/// # Safety
/// This ptr must have been produced with [Box::into_raw] and represent
/// a valid pointer to initialized memory.
unsafe fn free_owned_alloc<T>(ptr: *mut T) {
    // SAFETY: The pointer is non-null and represents a valid
    // pointer to initialized memory and was constructed via a box allocation.
    drop(unsafe { Box::from_raw(ptr) });
}



impl<T> Default for UnboundedQueue<T>
where 
    T: Send + Sync
{
    /// Initializes an unbounded queue with a default
    /// segment size.
    /// 
    /// # Example
    /// ```
    /// use lfqueue::UnboundedQueue;
    /// 
    /// let queue = UnboundedQueue::<usize>::default();
    /// queue.enqueue(0);
    /// assert_eq!(queue.dequeue(), Some(0));
    /// ```
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::AllocBoundedQueue;
    #[cfg(not(loom))]
    use crate::ScqError;

    #[test]
    #[cfg(not(loom))]
    pub fn check_scq_fill() {
        use crate::{AllocBoundedQueue, ScqError};

        let mut fill = AllocBoundedQueue::new(8);
        for i in 0..8 {
            fill.enqueue(i).unwrap();
        }
        assert_eq!(fill.enqueue(0), Err(ScqError::QueueFull));
    }

    #[cfg(not(loom))]
    #[test]
    pub fn fullinit() {
        use crate::lfstd::ScqRing;

        let ring = ScqRing::<_, 0>::new_alloc_ring_full(3);

        for i in 0..8 {
            assert_eq!(ring.dequeue(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_lcsq_ptr() {
        use crate::UnboundedQueue;

        let ring = UnboundedQueue::with_segment_size(16);
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
        use crate::lfstd::ScqRing;

        let ring = ScqRing::<_, 0>::new_alloc_ring_empty(3);
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
    pub fn lcsq_circus() {
        // use std::sync::{Arc, Barrier};

        // use crate::scq::AllocBoundedQueue;

        use std::sync::Arc;

        use crate::AllocBoundedQueue;

        let context = Arc::new(AllocBoundedQueue::new(8));
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

    #[cfg(not(loom))]
    #[test]
    pub fn scqqueue_enq_deq() -> Result<(), ScqError> {
        let holder = AllocBoundedQueue::new(8);
        holder.enqueue("A")?;
        holder.enqueue("B")?;

        assert_eq!(holder.dequeue(), Some("A"));
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

        let holder: AllocBoundedQueue<&str> = AllocBoundedQueue::new(8);
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
        use crate::lfstd::ScqRing;

        let ring = ScqRing::<_, 0>::new_alloc_ring_empty(3);

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
        

        use crate::lfstd::ScqRing;

        let mut ring = ScqRing::<_, 0>::new_alloc_ring_empty(3);
        for i in 0..ring.capacity() {
            ring.enqueue(i).unwrap();
        }

        // println!("Capacity: {:?}", auto.capacity());

        // auto.enqueue(1).unwrap();

        // println!("Value: {:?}", auto.dequeue());

        let mut auto = ScqRing::new_alloc_ring_full(3);

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
            auto.enqueue(value.unwrap()).unwrap();
        }

        for i in 0..auto.capacity() {
            assert_eq!(auto.dequeue(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    pub fn cover_example() {
        // Create an allocated buffer.
        let buffer = AllocBoundedQueue::new(1);

        // Enqueue the value 0.
        assert!(buffer.enqueue(0).is_ok());

        // Dequeue the value.
        assert_eq!(buffer.dequeue(), Some(0));
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_init_alloc_queue() {
        // PURPOSE: the purpose of this test is to ensure
        // the rings are getting initialized to the correct size.
        // Check the queue for consistency.
        fn check_queue(size: usize) {
            let queue = AllocBoundedQueue::new(size);
            assert_eq!(queue.capacity(), size);
            for i in 0..(size) {
                assert!(queue.enqueue(i).is_ok());
            }
            for i in 0..(size) {
                assert_eq!(queue.enqueue(i), Err(ScqError::QueueFull));
            }
            for i in 0..(size) {
                assert_eq!(queue.dequeue(), Some(i));
            }
            for _ in 0..(size) {
                assert_eq!(queue.dequeue(), None);
            }
        }

        // Check a few queues.
        check_queue(1);
        check_queue(2);
        check_queue(4);
        check_queue(8);
        check_queue(16);
        check_queue(32);
        check_queue(64);
        check_queue(128);
    }

    #[cfg(not(loom))]
    #[test]
    pub fn alloc_cover_example() {
        // PURPOSE: verify the correctness of the example in the README.md.
        use crate::{const_queue, AllocBoundedQueue, UnboundedQueue, ConstBoundedQueue};

        // Make an allocated queue of size 8.
        let queue = AllocBoundedQueue::new(8);
        assert!(queue.enqueue(0).is_ok()); // this should enqueue correctly.
        assert_eq!(queue.dequeue(), Some(0));



        // Make an unbounded queue of segment size 8.
        let queue = UnboundedQueue::with_segment_size(8);
        queue.enqueue(0);
        assert_eq!(queue.dequeue(), Some(0));


        // Make a constant queue of size 8.
        let queue = const_queue!(usize; 8);
        assert!(queue.enqueue(8).is_ok());
        assert_eq!(queue.dequeue(), Some(8));
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_zst_alloc() {
        use crate::ConstBoundedQueue;


        let queue = AllocBoundedQueue::new(4);
        assert_eq!(queue.capacity(), 4);
        for _ in 0..queue.capacity() {
            assert!(queue.enqueue(()).is_ok());
        }
        assert_eq!(queue.enqueue(()), Err(ScqError::QueueFull));

        for _ in 0..queue.capacity() {
            assert_eq!(queue.dequeue(), Some(()));
        }
        assert!(queue.dequeue().is_none());

        // assert!(queue.enqueue(()).is_ok());
        

    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_zst_unbounded() {
        use crate::UnboundedQueue;


        let queue = UnboundedQueue::new();
        queue.enqueue(());
        assert_eq!(queue.dequeue(), Some(()));
        assert!(queue.dequeue().is_none());

    }
}
