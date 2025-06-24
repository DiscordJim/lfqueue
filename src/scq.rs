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

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

use crossbeam_utils::CachePadded;
use lockfree::incin::{Incinerator, Pause};
use owned_alloc::OwnedAlloc;




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

#[derive(Debug)]
pub struct ScqQueue<T, const SEAL: bool> {
    backing: Box<[UnsafeCell<Option<T>>]>,
    free_queue: ScqRing<SEAL>,
    alloc_queue: ScqRing<SEAL>,
    used: CachePadded<AtomicUsize>,
    _type: PhantomData<T>,
}


unsafe impl<T, const S: bool> Send for ScqQueue<T, S> {}
unsafe impl<T, const S: bool> Sync for ScqQueue<T, S> {}

impl<T> ScqQueue<T, false> {
    pub fn new(order: usize) -> Self {
        Self::hidden_new(order)
    }
}

impl<T, const S: bool> ScqQueue<T, S> {

    pub const MAX_ORDER: usize = 63;
    pub const MIN_ORDER: usize = 2;

    fn hidden_new(order: usize) -> Self {
        let size = 1 << (order + 1);
        Self {
            backing: (0..size)
                .map(|_| UnsafeCell::new(None))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
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
        unsafe { (*self.backing[pos].get()) = Some(item) };
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release); // prevent reordering
        // unsafe { (&mut *self.backing.get())[pos] = Some(item) };
        // let slot = unsafe { &mut (&mut *self.backing.get())[pos] };

        // SAFETY: No one else has access to this slot.
        // *slot = Some(item);
        // unsafe { *slot.get() = Some(item) };

        if let Err(error) = self.alloc_queue.enqueue(pos) {
            println!("error");
            self.used.fetch_sub(1, AcqRel);
            let item = unsafe { (*self.backing[pos].get()).take() };
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
    where
        T: Debug + Clone,
    {
        // println!("Starting dequeue../. (A)");
        let pos = self.alloc_queue.dequeue()?;
        // println!("Position: {pos}");
        // println!("Finishing dequeue... (A)");

        // println!("Pos: {:?}", pos);

        self.used.fetch_sub(1, AcqRel);

        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire); // prevent reordering
        let value = unsafe { (*self.backing[pos].get()).take() };
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release); // prevent reordering

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

type UnboundedQueueIncinerator<T> = Incinerator<OwnedAlloc<LscqNode<T>>>;

// TODO: Concurrent removal, likely need hazard pointers, read original paper.
// mabe haphazard crate?
// https://karevongeijer.com/blog/lock-free-queue-in-rust/
#[derive(Debug)]
pub struct LcsqQueue<T> {
    head: CachePadded<AtomicPtr<LscqNode<T>>>,
    tail: CachePadded<AtomicPtr<LscqNode<T>>>,
    internal_order: usize,
    incin: UnboundedQueueIncinerator<T>,
}

/// Stores the actual [ScqQueue] along with a pointer
/// to the next node within the queue. This gives us an
/// unbounded amount of storage.
#[derive(Debug)]
struct LscqNode<T> {
    is_live: AtomicBool,
    value: ScqQueue<T, true>,
    next: AtomicPtr<LscqNode<T>>,
}

impl<T> LscqNode<T> {
    /// Allocates a new [LscqNode] object with an order. This
    /// will create a queue with a size of 2 ^ (order + 1).
    pub fn allocate(order: usize) -> OwnedAlloc<Self> {
        OwnedAlloc::new(Self {
            is_live: AtomicBool::new(true),
            next: AtomicPtr::default(),
            value: ScqQueue::hidden_new(order),
        })
    }
    fn new(order: usize) -> Self {
        Self {
            is_live: AtomicBool::new(true),
            next: AtomicPtr::default(),
            value: ScqQueue::hidden_new(order),
        }
    }
}

#[inline(always)]
fn yield_marker() {
    // std::thread::yield_now();
    #[cfg(loom)]
    loom::thread::yield_now();
}

/// Bypasses the null check for the creation of a [NonNull]. There
/// is still a check in debug mode so that tests may catch a `null`
/// value and fail as intended.
///
/// # Safety
/// To call this method safely, `ptr` must be a non-null pointer.
#[inline(always)]
unsafe fn bypass_null<T>(ptr: *mut T) -> NonNull<T> {
    debug_assert!(!ptr.is_null());
    // SAFETY: A caller following the contract will ensure
    // that this pointer is not null.
    unsafe { NonNull::new_unchecked(ptr) }
}

// TODO: Implement finalization properly.

/// Checks the null alignment.
/// https://gitlab.com/bzim/lockfree/-/blob/master/src/ptr.rs?ref_type=heads
#[inline(always)]
pub fn check_null_align<T>() {
    debug_assert!(null_mut::<T>() as usize % align_of::<T>() == 0);
}

impl<T> LcsqQueue<T> {
    /// Creates a new [LcsqQueue] with a single internal ring.
    pub fn new(segment_order: usize) -> Self {
        let queue = LscqNode::allocate(segment_order);

        let queue_ptr = queue.into_raw().as_ptr();

        // Check the null alignment of the `LscqNode`.
        check_null_align::<LscqNode<T>>();
        Self {
            head: AtomicPtr::new(queue_ptr).into(),
            tail: AtomicPtr::new(queue_ptr).into(),
            internal_order: segment_order,
            incin: Incinerator::new(),
        }
    }
    /// Enqueues an element of type `T` into the queue.
    pub fn enqueue(&self, mut element: T)
    where
        T: Clone + Debug,
    {
        loop {
            let cq_ptr = self.tail.load(Acquire);
            // SAFETY: The tail can never be a null pointer so we can
            // safely dereference. Additionally, there are never any mutabkle
            // pointers to this memory address.
            let cq = unsafe { &*cq_ptr };

            // If the next pointer is not null then we want to keep
            // going next until we have found the end.
            let next_ptr = cq.next.load(Acquire);
            if !next_ptr.is_null() {
                let _ = self
                    .tail
                    .compare_exchange(cq_ptr, next_ptr, AcqRel, Acquire);
                yield_marker();
                continue;
            }

            // Attempts to enqueue the item, we should finalize the
            // queue if we can.
            match cq.value.enqueue_cycle::<true>(element) {
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
            let ncq = OwnedAlloc::new(ncq).forget_inner().into_raw();
            // std::sync::atomic::compiler_fence(SeqCst);
            // std::sync::atomic::fence(SeqCst);

            // Try to insert a new tail into the queue.
            if cq
                .next
                .compare_exchange(null_mut(), ncq.as_ptr(), AcqRel, Acquire)
                .is_ok()
            {
                // Correct the list ordering.
                let _ = self
                    .tail
                    .compare_exchange(cq_ptr, ncq.as_ptr(), AcqRel, Acquire);
                // NOTE: We do not have to free the allocation here
                // because we haave succesfully put it into the list.
                return;
            }

            // Extract the first element so the borrow checker is happy and we
            // can avoid clones.
            // SAFETY: This is the only instance of this pointer
            // and it is non-null because we allocated it with `OwnedAlloc`.
            element = unsafe { ncq.as_ref().value.dequeue().unwrap() };

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
    pub fn dequeue(&self) -> Option<T>
    where
        T: Debug + Clone,
    {
        // Since dequeue actually removes nodes, we need to pause the incinerator
        // for the time being.
        // println!("Pausing...");
        let pause = self.incin.pause();
        // println!("Paused...");

        loop {
            // SAFETY: The head node will always be a non-null node.
            let cq_ptr = unsafe { bypass_null(self.head.load(Acquire)) };
            // SAFETY: The pointer is non-null and there are only immutable
            // references to it. Additionlly, since it is the head, there are
            // no mutable references to this memory location.
            let cq = unsafe { &*cq_ptr.as_ptr() };

            // Dequeue an entry.
            // println!("Enteirng...");
            let mut p = cq.value.dequeue();
            // println!("Existing...");
            if p.is_some() {
                // The entry actually holds a value, in this case,
                // we can just return the value.
                return p;
            }

            // If the next pointer is null then we have nothing
            // to dequeue and thus we can just return [Option::None].
            if cq.next.load(Acquire).is_null() {
                return None;
            }
            // Update the threshold.
            cq.value
                .alloc_queue
                .threshold
                .store(3 * (1 << (cq.value.free_queue.order + 1)) - 1, SeqCst);

            // Try dequeing again.
            p = cq.value.dequeue();
            if p.is_some() {
                return p;
            }

            // Here we remove the SCQ.
            // let cq_ptr = unsafe { NonNull::new_unchecked(self.head.load(Acquire)) };
            unsafe { self.free_front(cq_ptr, &pause) }?;

            yield_marker();
            // println!("injunctive");
        }
    }

    /// Frees the front of the queue.
    ///
    /// # Safety
    /// For this method to be called safely, front must be a non-null pointer
    /// and the incinerator must be properly paused.
    #[inline(always)]
    unsafe fn free_front(
        &self,
        mut front: NonNull<LscqNode<T>>,
        pause: &Pause<'_, OwnedAlloc<LscqNode<T>>>,
    ) -> Option<()> {
        // println!("Entering switch...");
        loop {
            // println!("entering free loop...");
            if unsafe { front.as_ref().is_live.swap(false, AcqRel) } {
                // We were the ones to kill it, so we can
                // remove it like this.
                // SAFETY: The incinerator is paused and the front is a non-null pointer.
                unsafe { self.try_clear_first(front, pause) };
                break Some(());
            } else {
                // println!("Switching up...");
                // It is already dead, we can help try to clear it.
                front = unsafe { self.try_clear_first(front, pause) }?;
            }
            // println!("disjunctive...");
            yield_marker();
        }
    }
    #[inline(always)]
    /// Replaces the front of the queue with the next node, inserting
    /// the previous front into the incinerator.
    ///
    /// Heavily inspired by the following code from [lockfree]:
    /// https://gitlab.com/bzim/lockfree/-/blob/master/src/queue.rs?ref_type=heads
    ///
    /// SAFETY: For this method to be safe several things must be upheld:
    /// 1. The nonnull points to a valid non-null pointer allocated by [OwnedAlloc].
    /// 2. The incinerator is paused.
    unsafe fn try_clear_first(
        &self,
        expected: NonNull<LscqNode<T>>,
        pause: &Pause<'_, OwnedAlloc<LscqNode<T>>>,
    ) -> Option<NonNull<LscqNode<T>>> {
        // SAFETY: There are no mutable references to expected and it
        // is a non-null pojinter.
        let next = unsafe { expected.as_ref().next.load(Acquire) };

        // If this is the only node, we will not remove it. We want front and
        // back to share the same node rather than having to set both to null when
        // the queue is empty.
        NonNull::new(next).map(|next_nnptr| {
            let ptr = expected.as_ptr();

            // This is cleanup-- another thread might do it.
            match self.head.compare_exchange(ptr, next, Relaxed, Relaxed) {
                Ok(_) => {
                    // Delete nodes via incinater to address ABA problem & use-after-frees.
                    pause.add_to_incin(unsafe { OwnedAlloc::from_raw(expected) });
                    next_nnptr
                }
                Err(found) => {
                    // Here it is safe to by-pass the check since we only store non-null
                    // pointers on the front.
                    unsafe { bypass_null(found) }
                }
            }
        })
    }
}

impl<T> Drop for LcsqQueue<T> {
    fn drop(&mut self) {
        // TODO: Implement the drop.

        let mut next = self.head.load(Acquire);
        while !next.is_null() {
            // SAFETY: If drop is called then we have exclusive access
            // to this strucutture. TODO: Improve docs with inspiration from lockfree crate.
            unsafe {
                let temp = next;
                next = (*next).next.load(Acquire);
                free_owned_alloc(temp);
            }
        }
    }
}

/// Frees memory allocated by an [OwnedAlloc]
///
/// # Safety
/// This ptr must have been produced with [OwnedAlloc::into_raw] and represent
/// a valid pointer to initialized memory.
unsafe fn free_owned_alloc<T>(ptr: *mut T) {
    unsafe { OwnedAlloc::from_raw(NonNull::new_unchecked(ptr)) };
}

#[cfg(test)]
mod tests {
    use std::marker::PhantomData;

    use lockfree::queue;

    use crate::scq::{
        LcsqQueue, ScqError, ScqQueue, ScqRing,
        lfring_signed_cmp,
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

        let context = Arc::new(ScqQueue::new(3));

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

        // barrier_finalized.wait();
        // println!("Done");
    }


    #[test]
    pub fn verify_small_queue_correctness() {

        fn queue_harness( order: usize) {
            let queue = ScqQueue::new(order);
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
    pub fn check_scq_fill() {
        let mut fill = ScqQueue::new(3);
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
        let ring = LcsqQueue::new(3);
        for i in 0..16 {
            ring.enqueue(i);
        }

        println!("RING: {:?}", ring);

        for _ in 0..16 {
            println!("HELLO: {:?}", ring.dequeue());
        }
        println!("HELLO: {:?}", ring.dequeue());
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_length_function() {
        let ring = ScqRing::<false>::new(3);
        assert_eq!(ring.capacity(), 8);

        println!(
            "Threshold: {}",
            ring.threshold.load(std::sync::atomic::Ordering::SeqCst)
        );

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
        let holder = ScqQueue::new(3);
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

        let holder: ScqQueue<&str, false> = ScqQueue::new(3);
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
