//! A counting allocator for a test binary: the bytes requested on the current thread.
//!
//! A `#[global_allocator]` has to be `unsafe impl GlobalAlloc`, and the library crates deny `unsafe`
//! (the guard pins `aligned.rs` at 3), so this lives with the tests, as `pvec-container`'s
//! `hot_allocations.rs` does. A test binary includes it with `mod support;` and installs it:
//!
//! ```ignore
//! #[global_allocator]
//! static A: support::counting_alloc::Counting = support::counting_alloc::Counting;
//! ```
#![allow(unsafe_code, dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

/// The allocator.
pub struct Counting;

fn add(n: usize) {
    // `try_with`: the counter is being torn down on a thread that is exiting.
    let _ = BYTES.try_with(|c| c.set(c.get().saturating_add(n)));
}

/// Bytes requested so far on this thread.
pub fn requested() -> usize {
    BYTES.with(Cell::get)
}

// SAFETY: every call is forwarded unchanged to the system allocator. The only addition is a
// thread-local counter update, which does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        add(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        add(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        add(n);
        unsafe { System.realloc(p, l, n) }
    }
}
