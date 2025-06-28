#[cfg(loom)]
pub use loom::sync::atomic::{AtomicBool, AtomicIsize, AtomicPtr, AtomicUsize, Ordering::*};

#[cfg(all(not(loom), not(feature = "portable-atomic")))]
pub use core::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering::*};

#[cfg(feature = "portable-atomic")]
pub use portable_atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering::*};
