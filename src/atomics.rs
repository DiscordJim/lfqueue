#[cfg(loom)]
pub use loom::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*, AtomicU8};

#[cfg(all(not(loom), not(feature = "portable-atomic")))]
pub use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering::*, AtomicU8};

#[cfg(feature = "portable-atomic")]
pub use portable_atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering::*, AtomicU8};
