//! A counting global allocator, so `steady_allocations` is a reading.
//!
//! The runner used to construct `QualityObservation { steady_allocations: 0,
//! .. }` — a literal the contract then "enforced". That is an assumption fed
//! into a check, not a measurement, and a check that can only be passed by
//! construction certifies nothing.
//!
//! This allocator forwards to the system allocator and counts every `alloc`
//! and `realloc` on the calling thread (thread-local, so a background font
//! scan or a worker pool cannot pollute a reading taken on the frame thread).
//!
//! # When the instrument is not installed
//!
//! `#[global_allocator]` is ignored when `std` is linked dynamically
//! (`-C prefer-dynamic`), and a counter that is never called reads zero —
//! indistinguishable from a perfect result. [`installed`] allocates and checks
//! the counter moved; the runner refuses to report a number when it did not.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static TRACING: Cell<bool> = const { Cell::new(false) };
    static IN_TRACE: Cell<bool> = const { Cell::new(false) };
}

fn record() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    if TRACING.try_with(Cell::get).unwrap_or(false) && !IN_TRACE.try_with(Cell::get).unwrap_or(true)
    {
        IN_TRACE.with(|t| t.set(true));
        eprintln!(
            "--- allocation ---\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        IN_TRACE.with(|t| t.set(false));
    }
}

/// Print a backtrace for every allocation `work` makes on this thread.
pub(crate) fn trace<T>(work: impl FnOnce() -> T) -> T {
    TRACING.with(|t| t.set(true));
    let value = work();
    TRACING.with(|t| t.set(false));
    value
}

fn count() -> u64 {
    ALLOCATIONS.try_with(Cell::get).unwrap_or(0)
}

pub(crate) struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: forwards the caller's contract unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // Counted whether or not it moved: undercounting is the direction
        // that lets a regression through.
        record();
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// Allocations `work` performed on this thread.
pub(crate) fn measure<T>(work: impl FnOnce() -> T) -> (T, u64) {
    let before = count();
    let value = work();
    (value, count().saturating_sub(before))
}

/// Whether this allocator is the one the process actually uses.
pub(crate) fn installed() -> bool {
    let (value, n) = measure(|| vec![0_u8; 64]);
    std::hint::black_box(&value);
    n > 0
}
