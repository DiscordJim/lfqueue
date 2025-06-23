// //! An implementation of the lock-free queue
// //! from "[PAPER NAME HERE]"
// //!
// //!
// //! https://github.com/rusnikola/lfqueue/blob/master/lfring_cas1.h
// use std::sync::atomic::{AtomicIsize, AtomicUsize};

use std::cell::UnsafeCell;
use std::{cmp, vec};
use std::marker::PhantomData;
use std::ptr::{NonNull, null_mut};

#[cfg(loom)]
use loom::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

#[cfg(not(loom))]
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

use crossbeam_utils::CachePadded;
use lockfree::incin::{Incinerator, Pause};
use owned_alloc::OwnedAlloc;

#[derive(Debug)]
struct ScqRing {
    is_finalized: CachePadded<AtomicBool>,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    threshold: CachePadded<AtomicIsize>,
    array: Box<[ScqEntry]>,
    order: usize,
}

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
        .map(|v| ScqEntry { value: v, is_safe: CachePadded::new(AtomicBool::new(true)) })
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
    while i != n {
        vector[lfring_map(i, n)].store(n + lfring_map(i, n), Relaxed);
        i += 1;
    }

    // while i != n {
    //     array[lfring_map(i, order, n)].store(-1isize as usize, Relaxed);
    //     i += 1;
    // }

    ScqAlloc {
        array: vector.into_iter().map(|value| ScqEntry {
            value,
            is_safe: CachePadded::new(AtomicBool::new(true))
        }).collect::<Vec<_>>().into_boxed_slice(),
        tail: lfring_threshold3(half, n),
        thresh: half as isize,
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

#[derive(Debug)]
struct ScqEntry {
    value: CachePadded<AtomicUsize>,
    is_safe: CachePadded<AtomicBool>
}

impl ScqRing {
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
        self.is_finalized.store(true, Release);
    }
    // Tail: 0, Tcycle: 15, Tidx: 0, entry: -1
    pub fn enqueue(&self, mut eidx: usize) -> Result<(), ScqError> {
        if eidx >= self.capacity() {
            // The index would be corrupted here.
            return Err(ScqError::IndexLargerThanOrder);
        }

        let half = lfring_pow2(self.order);
        let n = half * 2;

        // Modulo the value.
        eidx ^= n - 1;

        loop {
            let tail = self.tail.fetch_add(1, AcqRel);
            let tcycle = modup(tail << 1, n);
            let tidx = lfring_map(tail, n);
            let entry = self.array[tidx].value.load(Acquire);

            // println!("Tail: {tail}, TCycle: {tcycle}, TIDX: {tidx}, Entry: {entry}");

            'retry: loop {
                let ecycle = modup(entry, n);
                // let ecycle = entry | (2 * n - 1);
                // println!("ECycle: {}", ecycle as isize);

                //   exit(1);

                if (lfring_signed_cmp(ecycle, tcycle).is_lt())
                    && ((entry == ecycle)
                        || ((entry == (ecycle ^ n))
                            && lfring_signed_cmp(self.head.load(Acquire), tail).is_le()))
                {
                    if self.is_finalized.load(Acquire) {
                        return Err(ScqError::QueueFinalized);
                    }

                    if self.array[tidx]
                        .value
                        .compare_exchange_weak(entry, tcycle ^ eidx, AcqRel, Acquire)
                        .is_err()
                    {
                        yield_marker();
                        continue 'retry;
                    }

                    if self.threshold.load(SeqCst) != lfring_threshold3(half, n) as isize {
                        self.threshold
                            .store(lfring_threshold3(half, n) as isize, SeqCst);
                    }
                    return Ok(());
                } else {
                    break;
                }
            }
            yield_marker();
        }
    }
    pub fn capacity(&self) -> usize {
        1 << (self.order + 1)
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
        let n = self.capacity();

        if self.threshold.load(SeqCst) < 0 {
            return None;
        }

        let mut entry_new;


        loop {
            let mut head = self.head.fetch_add(1, AcqRel);
            let mut hcycle = modup(head << 1, n);
            let mut hidx = lfring_map(head, n);
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
                        self.array[hidx].value.fetch_or(n - 1, AcqRel);
                        return Some(entry & (n - 1));
                    }

                    
                    if (entry | n) != ecycle {
                        entry_new = entry & !n;
                        if entry == entry_new {
                            break;
                        }
                    } else {
                        attempt += 1;
                        if attempt <= 100 {
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
pub struct ScqQueue<T> {
    backing: Box<[UnsafeCell<Option<T>>]>,
    free_queue: ScqRing,
    alloc_queue: ScqRing,
    used: CachePadded<AtomicUsize>,
    _type: PhantomData<T>,
}

unsafe impl<T> Send for ScqQueue<T> {}
unsafe impl<T> Sync for ScqQueue<T> {}

impl<T> ScqQueue<T> {
    pub fn new(order: usize) -> Self {
        // let ring = ScqRing::new(order);
        let size = 1 << (order + 1);
        // let ring = ScqRing::n

        Self {
            backing: 
                (0..size)
                    .map(|_| UnsafeCell::new(None))
                    .collect::<Vec<_>>()
                    .into_boxed_slice()
            ,
            free_queue: ScqRing::new_full(order),
            alloc_queue: ScqRing::new(order),
            used: CachePadded::new(AtomicUsize::new(0)),
            _type: PhantomData,
        }
    }
    pub fn enqueue(&self, item: T) -> Result<(), ScqError> {
        // We want to make a call to the internal method here
        // without finalizing. If someone is calling the method
        // with [ScqQueue::enqueue] then it is not part of an unbounded
        // queue.
        self.enqueue_cycle(item, false).map_err(|(_, b)| b)
    }

    #[inline(always)]
    fn enqueue_cycle(&self, item: T, finalize: bool) -> Result<(), (T, ScqError)> {
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
            return Err((item, ScqError::QueueFull));
        };

        // println!("Accessing {pos}...");

        // let pos = self.free_queue.dequeue().expect("Queue should be able to store 2^(order + 1) items but errored while dequeing a free slot that should have been present.");

        // SAF
        unsafe { (*self.backing[pos].get()) = Some(item) };
        // unsafe { (&mut *self.backing.get())[pos] = Some(item) };
        // let slot = unsafe { &mut (&mut *self.backing.get())[pos] };

        // SAFETY: No one else has access to this slot.
        // *slot = Some(item);
        // unsafe { *slot.get() = Some(item) };

        if let Err(error) = self.alloc_queue.enqueue(pos) {
            let item = unsafe { (*self.backing[pos].get()).take() };
            return Err((item.unwrap(), error));
        }

        if finalize {
            // As described in the paper we must finalize this queue
            // so that nothing more will be added to it.
            if size + 1 >= self.free_queue.capacity() {
                self.alloc_queue.finalize();
            }
        }

        // self.alloc_queue.enqueue(pos)?;

        Ok(())
    }
    pub fn dequeue(&self) -> Option<T> {
        // println!("Starting dequeue../. (A)");
        let pos = self.alloc_queue.dequeue()?;
        // println!("Finishing dequeue... (A)");

        self.used.fetch_sub(1, AcqRel);

        let value = unsafe { (*self.backing[pos].get()).take().unwrap() };

        // let value = unsafe { &mut *self.holder[pos].get() }.take().unwrap();

        // This slot is now freed again!
        self.free_queue.enqueue(pos).unwrap();

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
    value: ScqQueue<T>,
    next: AtomicPtr<LscqNode<T>>,
}

impl<T> LscqNode<T> {
    /// Allocates a new [LscqNode] object with an order. This
    /// will create a queue with a size of 2 ^ (order + 1).
    pub fn allocate(order: usize) -> OwnedAlloc<Self> {
        OwnedAlloc::new(Self {
            is_live: AtomicBool::new(true),
            next: AtomicPtr::default(),
            value: ScqQueue::new(order),
        })
    }
    fn new(order: usize) -> Self {
        Self {
            is_live: AtomicBool::new(true),
            next: AtomicPtr::default(),
            value: ScqQueue::new(order)
        }
    }
}

#[inline(always)]
fn yield_marker() {
    std::thread::yield_now();
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
    pub fn enqueue(&self, mut element: T) {
        loop {
            let cq_ptr = self.tail.load(Acquire);
            // SAFETY: The tail can never be a null pointer so we can
            // safely dereference. Additionally, there are never any mutabkle
            // pointers to this memory address.
            let cq = unsafe { &*cq_ptr };

            // If the next pointer is not null then we want to keep
            // going next until we have found the end.
            if !cq.next.load(Acquire).is_null() {
                let _ = self
                    .tail
                    .compare_exchange(cq_ptr, cq.next.load(Acquire), AcqRel, Acquire);
                yield_marker();
                continue;
            }

            // Attempts to enqueue the item, we should finalize the
            // queue if we can.
            match cq.value.enqueue_cycle(element, true) {
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
            let ncq = OwnedAlloc::new(ncq).into_raw().as_ptr();

            // std::sync::atomic::fence(Release);

            // Try to insert a new tail into the queue.
            if cq
                .next
                .compare_exchange(null_mut(), ncq, AcqRel, Acquire)
                .is_ok()
            {
                // Correct the list ordering.
                let _ = self.tail.compare_exchange(cq_ptr, ncq, AcqRel, Acquire);
                // NOTE: We do not have to free the allocation here
                // because we haave succesfully put it into the list.
                return;
            }

            // Extract the first element so the borrow checker is happy and we
            // can avoid clones.
            // SAFETY: This is the only instance of this pointer
            // and it is non-null because we allocated it with `OwnedAlloc`.
            element = unsafe { (&mut *ncq).value.dequeue().unwrap() };

            // SAFETY: The allocation was created with `OwnedAlloc` and we
            // have unique access to it.
            unsafe {
                free_owned_alloc(ncq);
            }

            yield_marker();
        }
    }
    /// Removes an element from the queue returning an [Option]. If the queue
    /// is empty then the returned value will be [Option::None].
    pub fn dequeue(&self) -> Option<T> {
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
        loop {
            // println!("entering free loop...");
            if unsafe { front.as_ref().is_live.swap(false, AcqRel) } {
                // We were the ones to kill it, so we can
                // remove it like this.
                // SAFETY: The incinerator is paused and the front is a non-null pointer.
                unsafe { self.try_clear_first(front, pause) };
                break Some(());
            } else {
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
    use crate::scq::{LcsqQueue, ScqError, ScqQueue, ScqRing, lfring_signed_cmp};

    #[cfg(loom)]
    #[test]
    #[test]
    pub fn loom_unbounded_queue() {
        loom::model(|| {
            let ring = LcsqQueue::new(2);
            for i in 0..16 {
                ring.enqueue(i);
            }
            for i in 0..16 {
                assert_eq!(ring.dequeue(), Some(i));
            }
        });
    }

    #[cfg(loom)]
    #[test]
    pub fn loom_bounded_ring() {
        loom::model(|| {
            let ring = ScqRing::new(2);

            for i in 0..ring.capacity() {
                ring.enqueue(i).unwrap();
            }

            for i in 0..ring.capacity() {
                assert_eq!(ring.dequeue(), Some(i));
            }
        });
    }

    #[cfg(not(loom))]
    #[test]
    pub fn lcsq_circus() {
        use std::sync::{Arc, Barrier};

        let context = Arc::new(LcsqQueue::new(3));

        let threads = 25;
        let thread_runs = 10;

        let barrier = Arc::new(Barrier::new(threads));

        // let barrier_finalized = Arc::new(Barrier::new(threads + 1));
        let mut handles = vec![];
        for _ in 0..threads {
            handles.push(std::thread::spawn({
                // let barrier_finalized = barrier_finalized.clone();
                let context = context.clone();
                let barrier = barrier.clone();
                move || {
                    barrier.wait();
                    // println!("hello");
                    for i in 0..thread_runs {
                        // println!("start queue...");
                        context.enqueue(i);
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
                    // println!("I'm done.");
                    // barrier_finalized.wait();
                }
            }));
        }

        for handle in handles {
            handle.join().unwrap();
        }

        // barrier_finalized.wait();
        // println!("Done");
    }

    #[cfg(not(loom))]
    #[test]
    pub fn fullinit() {
        let ring = ScqRing::new_full(3);

        for i in 0..16 {
            assert_eq!(ring.dequeue(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    pub fn test_lcsq_ptr() {
        let ring = LcsqQueue::new(2);
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
        let ring = ScqRing::new(2);
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
    pub fn test_lfring_ptrs() -> Result<(), ScqError> {
        let holder = ScqQueue::new(2);
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

        let holder: ScqQueue<&str> = ScqQueue::new(2);
        println!("Holder: {:?}", holder);
        holder.enqueue("A").unwrap();
        holder.enqueue("B").unwrap();
        holder.enqueue("C").unwrap();
        holder.enqueue("D").unwrap();

        holder.enqueue("E").unwrap();
        holder.enqueue("F").unwrap();
        holder.enqueue("G").unwrap();
        holder.enqueue("H").unwrap();

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
        let ring = ScqRing::new(2);

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
    pub fn test_lfring() {
        // println!("Bruh: {}", (-1isize as usize));

        let queue = ScqRing::new(2);
        println!("Queue: {:?}", queue);
        queue.enqueue(4).unwrap();
        println!("Queue: {:?}", queue);
        queue.enqueue(2).unwrap();
        println!("Queue: {:?}", queue);
        queue.enqueue(3).unwrap();
        println!("Queue: {:?}", queue);
        queue.enqueue(6).unwrap();
        println!("Queue: {:?}", queue);

        let val = queue.dequeue();
        println!("Value: {:?}", val);
        let val = queue.dequeue();
        println!("Value: {:?}", val);
        let val = queue.dequeue();
        println!("Value: {:?}", val);
        let val = queue.dequeue();
        println!("Value: {:?}", val);
        let val = queue.dequeue();
        println!("Value: {:?}", val);
    }
}
