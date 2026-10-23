//! Decoding a hot block allocates a number of times that does not depend on its size (issue P4.2),
//! measured with a counting allocator on the decoding thread only.
#![allow(unsafe_code)]

use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::hot::*;
use pvec_entity::EntityId;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static COUNT: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

// SAFETY: every call is forwarded unchanged to the system allocator; the only addition is a
// thread-local counter increment, which does not allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        COUNT.with(|c| c.set(c.get() + 1));
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

fn allocations_in(f: impl FnOnce()) -> usize {
    let before = COUNT.with(Cell::get);
    f();
    COUNT.with(Cell::get) - before
}

fn block(kind: HotKind, n: u32) -> AlignedBytes {
    let mut b = HotBlockBuilder::new(kind);
    for i in 0..n {
        let f = f64::from(i);
        let id = EntityId::new(i).unwrap();
        match kind {
            HotKind::Line => b.push_line(id, i % 7, [f, f], [f + 1.0, f + 2.0]).unwrap(),
            HotKind::Polyline => {
                b.push_polyline(id, i % 7, &[[f, f], [f + 1.0, f], [f + 1.0, f + 1.0]]).unwrap()
            }
            HotKind::CircleArc => b.push_circle_arc(id, i % 7, [f, f], 1.0, [0.0, 1.0]).unwrap(),
            HotKind::EllipseArc => {
                b.push_ellipse_arc(id, i % 7, [f, f], [2.0, 1.0], [0.0, 0.0, 1.0]).unwrap()
            }
        }
    }
    b.finish().unwrap()
}

#[test]
fn decoding_a_block_of_n_entities_allocates_a_number_of_times_independent_of_n() {
    for kind in HotKind::ALL {
        let mut counts = Vec::new();
        for n in [1u32, 100, 20_000] {
            let buf = block(kind, n);
            let bytes = buf.as_bytes();
            let mut touched = 0usize;
            let allocs = allocations_in(|| {
                let v = HotBlockView::parse(bytes).unwrap();
                // reading every column is part of decoding a viewport; it must not allocate either
                for c in v.f64_columns() {
                    touched += c.len();
                }
                touched += v.entity_ids().len() + v.styles().len() + v.run_offsets().len();
            });
            assert!(touched > 0);
            counts.push(allocs);
        }
        assert!(counts.windows(2).all(|w| w[0] == w[1]), "{kind:?}: allocations vary with n: {counts:?}");
        assert_eq!(counts[0], 0, "{kind:?}: the view borrows everything, so there is nothing to allocate");
    }
}

#[test]
fn the_counting_allocator_does_see_allocations() {
    let n = allocations_in(|| {
        let v = vec![1u8; 1024];
        std::hint::black_box(&v);
    });
    assert!(n >= 1);
}
