//! Issue #56 (P7.5): picking never returns an id absent from the manifest. A corrupted file yields an
//! error rather than a fabricated id, and the work stays small against the file (the P4.8 budget
//! family: nothing is sized by a number the file declares).
#![allow(missing_docs, unsafe_code)]

use pvec::{
    Bounds2, Curve, EntityId, Manifest, PickIndex, PickTolerance, Point2, PvecReader, PvecWriter, Transform2,
};
use pvec_geometry::Polyline;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! { static BYTES: Cell<usize> = const { Cell::new(0) }; }

struct Counting;

fn add(n: usize) {
    let _ = BYTES.try_with(|c| c.set(c.get().saturating_add(n)));
}

// SAFETY: every call is forwarded unchanged to the system allocator; the only addition is a
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

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn requested() -> usize {
    BYTES.with(Cell::get)
}

const ENTITIES: u32 = 12;

fn file() -> Vec<u8> {
    let mut w = PvecWriter::new(Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-56".into(),
        producer_version: "1".into(),
    })
    .unwrap();
    for i in 0..ENTITIES {
        let f = f64::from(i);
        let c =
            Curve::Polyline(Polyline::new(vec![Point2 { x: f, y: 0.0 }, Point2 { x: f, y: 5.0 }]).unwrap());
        w.push(EntityId::new(i).unwrap(), &c).unwrap();
    }
    w.finish().unwrap()
}

/// Opens and indexes `bytes`; the ids it can ever return are exactly `0..entity_count`.
fn index(bytes: Vec<u8>) -> Result<(PickIndex, u64), pvec::PvecError> {
    let r = PvecReader::open(bytes)?;
    let ix = PickIndex::new(&r)?;
    Ok((ix, r.entity_count()?))
}

#[test]
fn issue_56_every_single_byte_corruption_is_an_error_or_a_file_whose_ids_are_all_in_the_manifest() {
    let good = file();
    let (_, count) = index(good.clone()).unwrap();
    assert_eq!(count, u64::from(ENTITIES));
    let (mut refused, mut accepted) = (0, 0);
    for at in 0..good.len() {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut bad = good.clone();
            bad[at] ^= flip;
            match index(bad) {
                Err(_) => refused += 1,
                Ok((mut ix, n)) => {
                    accepted += 1;
                    // Whatever the index accepts, an id at or above the declared count is refused.
                    let s = Curve::Polyline(
                        Polyline::new(vec![Point2 { x: 0.0, y: 0.0 }, Point2 { x: 1.0, y: 0.0 }]).unwrap(),
                    );
                    assert!(ix.insert_exact(EntityId::new(u32::try_from(n).unwrap()).unwrap(), s).is_err());
                }
            }
        }
    }
    assert!(refused > accepted, "most corruptions must be refused: {refused} refused, {accepted} accepted");
}

#[test]
fn issue_56_a_corrupted_file_yields_an_error_not_a_fabricated_id() {
    let mut bad = file();
    let mid = bad.len() / 2;
    for b in &mut bad[mid..mid + 16] {
        *b ^= 0xa5;
    }
    assert!(index(bad).is_err());
    assert!(index(Vec::new()).is_err());
    assert!(index(vec![0xff; 4096]).is_err());
}

#[test]
fn issue_56_indexing_a_corrupted_file_allocates_in_proportion_to_the_file_not_to_what_it_declares() {
    let good = file();
    let ceiling = 64 * good.len() + (256 << 10);
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] = 0xff;
        let before = requested();
        let _ = index(bad);
        let used = requested() - before;
        assert!(used <= ceiling, "flipping byte {at} cost {used} bytes against a ceiling of {ceiling}");
    }
}

#[test]
fn issue_56_picking_an_empty_index_returns_nothing() {
    let (ix, _) = index(file()).unwrap();
    let tol = PickTolerance::new(2.0, &Transform2::IDENTITY).unwrap();
    assert!(ix.query_point(Point2 { x: 1.0, y: 1.0 }, tol).is_none());
    assert!(ix.query_nearest(Point2 { x: 1.0, y: 1.0 }, f64::MAX).is_none());
}
