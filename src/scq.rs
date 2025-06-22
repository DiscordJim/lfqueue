// //! An implementation of the lock-free queue
// //! from "[PAPER NAME HERE]"
// //!
// //!
// //! https://github.com/rusnikola/lfqueue/blob/master/lfring_cas1.h
// use std::sync::atomic::{AtomicIsize, AtomicUsize};

use std::cell::UnsafeCell;
use std::cmp;
use std::marker::PhantomData;
use std::ptr::{NonNull, null_mut};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize};

use std::sync::atomic::Ordering::*;

use crossbeam_utils::CachePadded;
use lockfree::incin::{Incinerator, Pause};
use lockfree::removable::Removable;
use owned_alloc::{OwnedAlloc, RawVec};

#[derive(Debug)]
struct ScqRing {
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
    threshold: CachePadded<AtomicIsize>,
    array: Box<[CachePadded<AtomicUsize>]>,
    order: usize,
}



const NON_YIELDING_CYCLES: usize = 20;
const YIELDING_CYCLES: usize = 10_000;

// #define __lfring_threshold3(half, n) ((long) ((half) + (n) - 1)

#[inline(always)]
fn lfring_threshold3(half: usize, n: usize) -> usize {
    (half) + (n) - 1
}

#[inline(always)]
fn lfring_pow2(order: usize) -> usize {
    return 1usize << order;
}

#[inline(always)]
fn modup(value: usize, n: usize) -> usize {
    value | (2 * n - 1)
}

#[inline(always)]
fn lfring_signed_cmp(a: usize, b: usize) -> cmp::Ordering {
    ((a as isize) - (b as isize)).cmp(&0)
}

type AtomicIndexArray = Box<[CachePadded<AtomicUsize>]>;

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
        

        vector[lfring_map(i, order, n)].store(n + lfring_raw_map(i, order, n), Relaxed);
        i += 1;
    }

    // while i != n {
    //     array[lfring_map(i, order, n)].store(-1isize as usize, Relaxed);
    //     i += 1;
    // }


    ScqAlloc {
        array: vector.into_boxed_slice(),
        tail: lfring_threshold3(half, n),
        thresh: half as isize
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
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(tail)),
            threshold: CachePadded::new(AtomicIsize::new(thresh)),
            array,
            order,
        }
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
            let tidx = lfring_map(tail, self.order, n);
            let entry = self.array[tidx].load(Acquire);

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
                    if self.array[tidx]
                        .compare_exchange_weak(entry, tcycle ^ eidx, AcqRel, Acquire)
                        .is_err()
                    {
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

        
        let mut entry_new = 0;
        let mut tail = 0;
        loop {
            let head = self.head.fetch_add(1, AcqRel);
            let hcycle = modup(head << 1, n);
            let hidx = lfring_map(head, self.order, n);
            let mut attempt = 0;

            'again: loop {
                let entry: usize = self.array[hidx].load(Acquire) as usize;

                // START DO
                loop {
                    let ecycle = modup(entry, n);
                    // println!("Ecycle: {}, Hcycle: {}", ecycle, hcycle);
                    if ecycle == hcycle {
                        self.array[hidx].fetch_or(n - 1, AcqRel);
                        return Some(entry & (n - 1));
                    }

                    if (entry | n) != ecycle {
                        entry_new = entry & !n;
                        if entry == entry_new {
                            break;
                        }
                    } else {
                        attempt += 1;
                        if (attempt <= 10000) {
                            continue 'again;
                        }
                        entry_new = hcycle ^ ((!entry) & n);
                    }

                    if !( lfring_signed_cmp(ecycle, hcycle).is_lt()
                        && self.array[hidx]
                            .compare_exchange_weak(entry, entry_new, AcqRel, Acquire)
                            .is_err())
                    {
                        break;
                    }
                }
                // END DO
                break;
            }

            tail = self.tail.load(Acquire);
            if lfring_signed_cmp(tail, head + 1).is_le() {
                self.catchup(tail, head + 1);
                self.threshold.fetch_sub(1, AcqRel);
                // println!("Exiting out of branch here...");
                return None;
            }

            if self.threshold.fetch_sub(1, AcqRel) <= 0 {
                return None;
            }
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
}

// const LF_CACHE_SHIFT: usize = 7;
// const CACHE_SHIFT_BYTES: usize = 1usize << LF_CACHE_SHIFT;
// const LFRING_MIN: usize = LF_CACHE_SHIFT - 2;

// fn map(idx: usize, order: usize, n: usize) -> usize {
//     idx & (n - 1)
// }
#[derive(Debug)]
pub struct ScqQueue<T> {
    backing: UnsafeCell<Box<[Option<T>]>>,
    free_queue: ScqRing,
    alloc_queue: ScqRing,
    used: AtomicUsize,
    _type: PhantomData<T>,
}

unsafe impl<T> Send for ScqQueue<T> {}
unsafe impl<T> Sync for ScqQueue<T> {}

impl<T> ScqQueue<T> {
    pub fn new(order: usize) -> Self {
        // let ring = ScqRing::new(order);
        let size = 1 << (order + 1);
        let ring = ScqRing::new(order);
        for i in 0..size {
            ring.enqueue(i).unwrap();
        }

        Self {
            backing: UnsafeCell::new(
                (0..size)
                    .map(|_| None)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            free_queue: ring,
            alloc_queue: ScqRing::new(order),
            used: AtomicUsize::new(0),
            _type: PhantomData,
        }
    }
    pub fn enqueue(&self, item: T) -> Result<(), ScqError> {
        // let value = OwnedAlloc::new(item);
        // let ptr = value.into_raw().as_ptr();

        // let current = self.used.fetch_add(1, Acquire);
        if self.free_queue.capacity() <= self.used.fetch_add(1, AcqRel) {
            self.used.fetch_sub(1, AcqRel);
            return Err(ScqError::QueueFull);
        }

        // self.used.fetch_add(1, Acquire);

        let pos = self.free_queue.dequeue().expect("Queue should be able to store 2^(order + 1) items but errored while dequeing a free slot that should have been present.");

        let slot = unsafe { &mut (&mut *self.backing.get())[pos] };

        // SAFETY: No one else has access to this slot.
        *slot = Some(item);
        // unsafe { *slot.get() = Some(item) };

        self.alloc_queue.enqueue(pos)?;

      
        Ok(())

        // let slot = self.free_queue.dequeue();
        // println!("Slot: {:?}", slot);
        // println!("ENQUEING PTR: {:?}", ptr);
        // self.proper.enqueue(ptr as usize);
    }
    pub fn dequeue(&self) -> Option<T> {
        let pos = self.alloc_queue.dequeue()?;

        self.used.fetch_sub(1, AcqRel);

        let value = unsafe { (&mut *self.backing.get())[pos].take().unwrap() };

        // let value = unsafe { &mut *self.holder[pos].get() }.take().unwrap();

        // This slot is now freed again!
        self.free_queue.enqueue(pos).unwrap();

        Some(value)
    }
}

const LF_CACHE_SHIFT: usize = 6;
const LFRING_MIN: usize = 0;
// const LFRING_MIN: usize = LF_CACHE_SHIFT - 3; // for 64-bit = 3

#[inline(always)]
fn lfring_raw_map(idx: usize, order: usize, n: usize) -> usize {
    if LFRING_MIN == 0 {
        idx & (n - 1)
    } else {
        let mask = n - 1;
        let lower = (idx & mask) >> (order - LFRING_MIN);
        let upper = (idx << LFRING_MIN) & mask;
        lower | upper
    }
}

#[inline(always)]
fn lfring_map(idx: usize, order: usize, n: usize) -> usize {
    lfring_raw_map(idx, order + 1, n)
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
    incin: Incinerator<OwnedAlloc<LscqNode<T>>>
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
    pub fn allocate(order: usize) -> OwnedAlloc<Self> {

      


        OwnedAlloc::new(Self {
            is_live: AtomicBool::new(true),
            next: AtomicPtr::default(),
            value: ScqQueue::new(order)
        })
    }
    
}

#[inline(always)]
unsafe fn bypass_null<T>(ptr: *mut T) -> NonNull<T> {
    debug_assert!(!ptr.is_null());
    // SAFETY: A caller following the contract will ensure
    // that this pointer is not null.
    unsafe { NonNull::new_unchecked(ptr) }
}


/// https://gitlab.com/bzim/lockfree/-/blob/master/src/ptr.rs?ref_type=heads
#[inline(always)]
pub fn check_null_align<T>() {
    debug_assert!(null_mut::<T>() as usize % align_of::<T>() == 0);
}





impl<T: Clone> LcsqQueue<T> {
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
            incin: Incinerator::new()
        }
    }
    pub fn enqueue(&self, element: T) {
        loop {
            let cq_ptr = self.tail.load(Acquire);
            println!("CQ: {:?}", cq_ptr);
            if unsafe { !(&*cq_ptr).next.load(Acquire).is_null() } {
                let _ = self.tail.compare_exchange(
                    cq_ptr,
                    unsafe { (*cq_ptr).next.load(Acquire) },
                    AcqRel,
                    Acquire,
                );
                continue;
            }

            let cq = unsafe { &*cq_ptr };

            if cq.value.enqueue(element.clone()).is_ok() {
                return;
            }

            let ncq = LscqNode::allocate(self.internal_order);

            ncq.value.enqueue(element.clone()).unwrap();

            let ncq = ncq.into_raw().as_ptr();
            println!("Making new queue");

            if cq
                .next
                .compare_exchange(null_mut(), ncq, AcqRel, Acquire)
                .is_ok()
            {
                self.tail
                    .compare_exchange(cq_ptr, ncq, AcqRel, Acquire)
                    .unwrap();
                return;
            }

            // SAFETY: We just allocated this with an `OwnedAlloc` object,
            // and the ptr is certainly not null, so we ma free it here.
            unsafe { free_owned_alloc(ncq) };
        }
    }
    pub fn dequeue(&self) -> Option<T> {
        let pause = self.incin.pause();
    
        loop {
            let cq_ptr = unsafe { bypass_null(self.head.load(Acquire)) };
            let cq = unsafe { &*cq_ptr.as_ptr() };
            let mut p = cq.value.dequeue();
            if p.is_some() {
                return p;
            }
            if cq.next.load(Acquire).is_null() {
                return None;
            }
            cq.value
                .alloc_queue
                .threshold
                .store(3 * (1 << (cq.value.free_queue.order + 1)) - 1, SeqCst);
            p = cq.value.dequeue();
            if p.is_some() {
                return p;
            }



            // Here we remove the SCQ.
            let mut front_ptr = cq_ptr;
            loop {

                if unsafe { cq_ptr.as_ref().is_live.swap(false, AcqRel) } {
                    unsafe { self.try_clear_first(front_ptr, &pause) };
                    break;
                } else {
                    // It is already dead.
                    front_ptr = unsafe { self.try_clear_first(front_ptr, &pause) }?;
                }

            }
        }
    }
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
        pause: &Pause<'_, OwnedAlloc<LscqNode<T>>>
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
                next = (&*next).next.load(Acquire);
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


    #[test]
    pub fn fullinit() {
        let mut ring = ScqRing::new_full(3);
        
        for i in 0..16 {
            assert_eq!(ring.dequeue(), Some(i));
        }
    }

    #[test]
    pub fn test_lcsq_ptr() {
        let ring = LcsqQueue::new(2);
        for i in 0..16 {
            ring.enqueue(i);
        }

        println!("RING: {:?}", ring);

        for i in 0..16 {
            println!("HELLO: {:?}", ring.dequeue());
        }
        println!("HELLO: {:?}", ring.dequeue());
    }


    #[test]
    pub fn test_length_function() {
        let ring = ScqRing::new(2);
        assert_eq!(ring.capacity(), 8);

        println!("Threshold: {}", ring.threshold.load(std::sync::atomic::Ordering::SeqCst));

        ring.enqueue(3).unwrap();
        println!("Threshold: {}", ring.threshold.load(std::sync::atomic::Ordering::SeqCst));
        ring.enqueue(2).unwrap();
        println!("Threshold: {}", ring.threshold.load(std::sync::atomic::Ordering::SeqCst));


    }

    #[test]
    pub fn test_special_comparision_function() {
        assert!(lfring_signed_cmp(1, 2).is_lt());
        assert!(lfring_signed_cmp(1, 2).is_lt());
        assert!(lfring_signed_cmp(2, 2).is_le());
        assert!(lfring_signed_cmp(2, 1).is_gt());
    }

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
