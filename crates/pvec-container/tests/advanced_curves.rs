//! Advanced curve records over arenas (issue P4.3): consistency of counts, weights, sharing, the read
//! cost of one curve and composite cycles, plus the record block's own structure.

use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::advanced::*;
use pvec_container::blocks::arena::*;
use pvec_entity::EntityId;
use pvec_numeric::NumericError;
use std::cell::Cell;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn aligned(bytes: &[u8]) -> AlignedBytes {
    AlignedBytes::from_bytes(bytes)
}

const CUBIC_KNOTS: [f64; 9] = [0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
const CUBIC_X: [f64; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];
const CUBIC_Y: [f64; 5] = [0.0, 1.0, 0.0, 1.0, 0.0];
const WEIGHTS: [f64; 5] = [1.0, 2.0, 1.5, 2.0, 1.0];

fn spec<'a>(knots: &'a [f64], x: &'a [f64], y: &'a [f64], weights: Option<&'a [f64]>) -> SplineSpec<'a> {
    SplineSpec { degree: 3, knots, x, y, weights, closed: false, periodic: false }
}

fn cubic() -> SplineSpec<'static> {
    spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, None)
}

fn view(blocks: &AdvancedBlocks) -> AdvancedView<'_> {
    AdvancedView::parse(blocks.slices()).expect("the blocks parse")
}

fn spline(v: CurveView<'_>) -> SplineView<'_> {
    match v {
        CurveView::Spline(s) => s,
        CurveView::Composite(_) => panic!("expected a spline"),
    }
}

fn composite(v: CurveView<'_>) -> CompositeView<'_> {
    match v {
        CurveView::Composite(c) => c,
        CurveView::Spline(_) => panic!("expected a composite"),
    }
}

// ---- the record block, by hand ------------------------------------------------------------------

/// Two records written by hand from the table in `docs/CONTAINER.md` chapter 3: a closed rational cubic
/// (id 3, style 10, knots 0, weights 0, control points 0) and a composite (id 9) whose segment list is
/// entry 0 and whose trims are entry 0. `ff ff ff ff` is "no entry".
#[rustfmt::skip]
const RECORDS: [u8; 96] = [
    0x50, 0x56, 0x52, 0x31,                         // magic PVR1
    0x01, 0x00,                                     // version 1
    0x00, 0x00,                                     // reserved
    0x02, 0x00, 0x00, 0x00,                         // record_count 2
    0x60, 0x00, 0x00, 0x00,                         // block_length 96
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    0x03, 0x00, 0x00, 0x00,                         // id 3
    0x0a, 0x00, 0x00, 0x00,                         // style 10
    0x01, 0x03, 0x03, 0x00,                         // kind 1 (spline), degree 3, flags 3 (rational, closed)
    0x00, 0x00, 0x00, 0x00,                         // knots entry 0
    0x00, 0x00, 0x00, 0x00,                         // weights entry 0
    0x00, 0x00, 0x00, 0x00,                         // control points entry 0
    0xff, 0xff, 0xff, 0xff,                         // segments: none
    0xff, 0xff, 0xff, 0xff,                         // trims: none
    0x09, 0x00, 0x00, 0x00,                         // id 9
    0x00, 0x00, 0x00, 0x00,                         // style 0
    0x02, 0x00, 0x00, 0x00,                         // kind 2 (composite), degree 0, flags 0
    0xff, 0xff, 0xff, 0xff,                         // knots: none
    0xff, 0xff, 0xff, 0xff,                         // weights: none
    0xff, 0xff, 0xff, 0xff,                         // control points: none
    0x00, 0x00, 0x00, 0x00,                         // segments entry 0
    0x00, 0x00, 0x00, 0x00,                         // trims entry 0
];

fn records_patched(at: usize, patch: &[u8]) -> AlignedBytes {
    let mut v = RECORDS.to_vec();
    v[at..at + patch.len()].copy_from_slice(patch);
    aligned(&v)
}

fn records_refused(bytes: &AlignedBytes) -> ArenaError {
    match RecordsView::parse(bytes.as_bytes()) {
        Ok(_) => panic!("the record block was read, expected a refusal"),
        Err(e) => e,
    }
}

#[test]
fn a_hand_written_record_block_is_read_field_for_field() {
    let buf = aligned(&RECORDS);
    let r = RecordsView::parse(buf.as_bytes()).expect("the literal parses");
    assert_eq!(r.len(), 2);
    let a = r.get(0).unwrap();
    assert_eq!(
        a,
        AdvancedCurveRecord {
            id: 3,
            style: 10,
            kind: RecordKind::Spline,
            degree: 3,
            flags: FLAG_RATIONAL | FLAG_CLOSED,
            knots: 0,
            weights: 0,
            control_points: 0,
            segments: NO_ENTRY,
            trims: NO_ENTRY,
        }
    );
    let b = r.get(1).unwrap();
    assert_eq!((b.id, b.kind, b.degree, b.flags), (9, RecordKind::Composite, 0, 0));
    assert_eq!((b.segments, b.trims, b.knots), (0, 0, NO_ENTRY));
    assert!(r.get(2).is_none());
}

#[test]
fn the_builder_writes_the_hand_written_record_block_byte_for_byte() {
    let mut b = AdvancedBuilder::new();
    let knots = [0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0];
    let (x, y, w) = ([0.0, 1.0, 2.0, 0.0], [0.0, 1.0, 0.0, 0.0], [1.0, 2.0, 2.0, 1.0]);
    let s = SplineSpec {
        degree: 3,
        knots: &knots,
        x: &x,
        y: &y,
        weights: Some(&w),
        closed: true,
        periodic: false,
    };
    // pushed in the opposite order to the block: the writer orders by id
    b.push_composite(id(9), 0, &[id(3)], Some(&[[0.0, 1.0]]), false).unwrap();
    b.push_spline(id(3), 10, s).unwrap();
    let out = b.finish().unwrap();
    assert_eq!(out.records.as_bytes(), &RECORDS[..]);
}

#[test]
fn record_block_refusals_name_their_rule() {
    assert_eq!(records_refused(&records_patched(0, b"PVRX")), ArenaError::BadMagic);
    assert_eq!(records_refused(&records_patched(4, &[2, 0])), ArenaError::UnsupportedVersion { found: 2 });
    assert_eq!(records_refused(&records_patched(6, &[1])), ArenaError::NonZeroReserved);
    assert_eq!(records_refused(&records_patched(20, &[1])), ArenaError::NonZeroReserved);
    assert_eq!(
        records_refused(&records_patched(12, &[0x70])),
        ArenaError::LengthMismatch { declared: 0x70, actual: 96 }
    );
    assert_eq!(records_refused(&records_patched(8, &[0, 0, 0, 0])), ArenaError::EmptyBlock);
    assert_eq!(records_refused(&records_patched(32 + 8, &[7])), ArenaError::UnknownKind { id: 7 });
    // ids must ascend: record 1 gets id 3 again
    assert_eq!(records_refused(&records_patched(64, &[3])), ArenaError::IdsNotAscending { index: 1 });
    // an id at the NP-18 limit (2^28)
    assert_eq!(
        records_refused(&records_patched(64, &[0, 0, 0, 0x10])),
        ArenaError::IdsNotAscending { index: 1 }
    );
}

#[test]
fn flags_and_fields_that_a_kind_cannot_have_are_refused() {
    // undefined flag bit 3
    assert!(matches!(records_refused(&records_patched(32 + 10, &[0x0b])), ArenaError::InvalidFlags { .. }));
    // a composite cannot be rational
    assert!(matches!(records_refused(&records_patched(64 + 10, &[0x01])), ArenaError::InvalidFlags { .. }));
    // a spline with a segment list, a spline with no knots, a composite with a degree
    assert!(matches!(
        records_refused(&records_patched(32 + 24, &[0])),
        ArenaError::RecordFields { index: 0 }
    ));
    assert!(matches!(
        records_refused(&records_patched(32 + 12, &[0xff, 0xff, 0xff, 0xff])),
        ArenaError::RecordFields { index: 0 }
    ));
    assert!(matches!(records_refused(&records_patched(64 + 9, &[3])), ArenaError::RecordFields { index: 1 }));
    // a rational spline needs a weights entry and a non-rational one must not have one
    assert!(matches!(
        records_refused(&records_patched(32 + 16, &[0xff, 0xff, 0xff, 0xff])),
        ArenaError::RecordFields { index: 0 }
    ));
    assert!(matches!(
        records_refused(&records_patched(32 + 10, &[0x02])),
        ArenaError::RecordFields { index: 0 }
    ));
}

#[test]
fn the_periodic_flag_is_refused_because_the_profile_has_no_wrapped_form() {
    // flags 3 -> 7 sets bit 2
    let e = records_refused(&records_patched(32 + 10, &[0x07]));
    assert_eq!(e, ArenaError::PeriodicUnsupported);
    assert_eq!(e.rule(), "ARN-16");
    let mut b = AdvancedBuilder::new();
    let mut s = cubic();
    s.periodic = true;
    assert_eq!(b.push_spline(id(1), 0, s), Err(ArenaError::PeriodicUnsupported));
}

#[test]
fn every_truncation_of_the_record_block_is_refused_without_a_panic() {
    assert!(RecordsView::parse(aligned(&RECORDS).as_bytes()).is_ok(), "the whole block must parse first");
    for n in 0..RECORDS.len() {
        let buf = aligned(&RECORDS[..n]);
        assert!(RecordsView::parse(buf.as_bytes()).is_err(), "a {n} byte prefix was accepted");
    }
}

#[test]
fn the_record_has_no_per_entity_byte_blob_it_is_a_fixed_32_bytes() {
    assert_eq!(RECORD_LEN, 32);
    let blocks = one_of_each();
    assert_eq!(blocks.records.len(), RECORD_HEADER_LEN + RECORD_LEN * view(&blocks).len());
}

// ---- criterion 2 and 3: counts, knots, weights --------------------------------------------------

fn push_one(spec: SplineSpec<'_>) -> Result<AdvancedBlocks, ArenaError> {
    let mut b = AdvancedBuilder::new();
    b.push_spline(id(1), 0, spec)?;
    b.finish()
}

#[test]
fn a_clamped_cubic_with_the_right_counts_is_accepted_and_read_back_bit_for_bit() {
    let blocks = push_one(cubic()).unwrap();
    let v = view(&blocks);
    let s = spline(v.curve(0).unwrap());
    assert_eq!((s.id, s.degree, s.closed), (1, 3, false));
    assert_eq!(s.knots, &CUBIC_KNOTS);
    assert_eq!(s.x, &CUBIC_X);
    assert_eq!(s.y, &CUBIC_Y);
    assert!(s.weights.is_none());
}

#[test]
fn knots_must_equal_control_points_plus_degree_plus_one() {
    // 5 control points and degree 3 need 9 knots
    for n in [0usize, 1, 7, 8, 10, 12] {
        let knots: Vec<f64> = (0..n).map(|k| k as f64).collect();
        let e = push_one(spec(&knots, &CUBIC_X, &CUBIC_Y, None)).unwrap_err();
        assert_eq!(e, ArenaError::KnotCount { knots: n, control_points: 5, degree: 3 }, "{n} knots");
        assert_eq!(e.rule(), "ARN-13");
    }
}

#[test]
fn x_and_y_columns_of_different_length_are_refused() {
    let e = push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y[..4], None)).unwrap_err();
    assert_eq!(e, ArenaError::ControlPointColumns { x: 5, y: 4 });
}

#[test]
fn the_numeric_policy_still_decides_knot_order_degree_and_values() {
    let mut knots = CUBIC_KNOTS;
    knots.swap(3, 5);
    assert_eq!(
        push_one(spec(&knots, &CUBIC_X, &CUBIC_Y, None)).unwrap_err(),
        ArenaError::Numeric(NumericError::BadKnots)
    );
    let mut s = cubic();
    s.degree = 0;
    assert!(matches!(push_one(s).unwrap_err(), ArenaError::Numeric(NumericError::Limit)));
    s.degree = 17;
    assert!(matches!(push_one(s).unwrap_err(), ArenaError::Numeric(NumericError::Limit)));
    let mut x = CUBIC_X;
    x[2] = f64::NAN;
    assert_eq!(
        push_one(spec(&CUBIC_KNOTS, &x, &CUBIC_Y, None)).unwrap_err(),
        ArenaError::Numeric(NumericError::NotFinite)
    );
    x[2] = f64::INFINITY;
    assert!(push_one(spec(&CUBIC_KNOTS, &x, &CUBIC_Y, None)).is_err());
}

#[test]
fn a_closed_flag_that_contradicts_the_ends_is_refused_and_a_true_one_accepted() {
    let mut s = cubic();
    s.closed = true;
    assert_eq!(push_one(s).unwrap_err(), ArenaError::Numeric(NumericError::ClosedMismatch));
    let (x, y) = ([0.0, 1.0, 2.0, 1.0, 0.0], [0.0, 2.0, 0.0, -2.0, 0.0]);
    let mut s = spec(&CUBIC_KNOTS, &x, &y, None);
    s.closed = true;
    let blocks = push_one(s).unwrap();
    assert!(spline(view(&blocks).curve(0).unwrap()).closed);
}

#[test]
fn a_rational_curve_with_fewer_weights_than_control_points_is_refused_not_padded() {
    for n in [0usize, 1, 4] {
        let e = push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&WEIGHTS[..n]))).unwrap_err();
        assert_eq!(e, ArenaError::WeightCount { weights: n, control_points: 5 }, "{n} weights");
        assert_eq!(e.rule(), "ARN-14");
    }
}

#[test]
fn a_rational_curve_with_more_weights_than_control_points_is_refused_too() {
    let six = [1.0, 2.0, 1.5, 2.0, 1.0, 1.0];
    let e = push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&six))).unwrap_err();
    assert_eq!(e, ArenaError::WeightCount { weights: 6, control_points: 5 });
}

#[test]
fn a_rational_curve_with_one_weight_per_control_point_is_accepted_and_read_back() {
    let blocks = push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&WEIGHTS))).unwrap();
    let v = view(&blocks);
    assert_eq!(v.record(0).unwrap().flags & FLAG_RATIONAL, FLAG_RATIONAL);
    assert_eq!(spline(v.curve(0).unwrap()).weights.unwrap(), &WEIGHTS);
}

#[test]
fn weights_under_the_numeric_policy_floor_or_not_positive_are_refused() {
    let mut w = WEIGHTS;
    w[1] = 0.0;
    assert_eq!(
        push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&w))).unwrap_err(),
        ArenaError::Numeric(NumericError::BadWeight)
    );
    w[1] = -1.0;
    assert!(push_one(spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&w))).is_err());
}

// The reader must refuse the same things even when the writer is somebody else's.

/// Six blocks assembled by hand from records and payloads, so a reader sees what a hostile writer could
/// produce. Payload indices are the order of the lists, so none may repeat.
struct Hand {
    rows: Vec<AdvancedCurveRecord>,
    knots: Vec<Vec<f64>>,
    weights: Vec<Vec<f64>>,
    cps: Vec<(Vec<f64>, Vec<f64>)>,
    segments: Vec<Vec<u32>>,
    trims: Vec<Vec<f64>>,
}

impl Hand {
    fn new(rows: Vec<AdvancedCurveRecord>) -> Hand {
        Hand { rows, knots: vec![], weights: vec![], cps: vec![], segments: vec![], trims: vec![] }
    }

    fn build(&self) -> AdvancedBlocks {
        let mut records = Vec::new();
        records.extend_from_slice(&RECORD_MAGIC);
        records.extend_from_slice(&1u16.to_le_bytes());
        records.extend_from_slice(&[0, 0]);
        records.extend_from_slice(&(self.rows.len() as u32).to_le_bytes());
        records.extend_from_slice(&((32 + 32 * self.rows.len()) as u32).to_le_bytes());
        records.resize(32, 0);
        for r in &self.rows {
            let kind = match r.kind {
                RecordKind::Spline => 1u32,
                RecordKind::Composite => 2,
            };
            let word2 = kind | u32::from(r.degree) << 8 | u32::from(r.flags) << 16;
            for w in [r.id, r.style, word2, r.knots, r.weights, r.control_points, r.segments, r.trims] {
                records.extend_from_slice(&w.to_le_bytes());
            }
        }
        let f64s = |kind, lists: &Vec<Vec<f64>>| {
            let mut b = ArenaBuilder::new(kind);
            for l in lists {
                b.push_f64(l).unwrap();
            }
            b.finish().unwrap()
        };
        let mut c = ArenaBuilder::new(PayloadKind::ControlPoints);
        for (x, y) in &self.cps {
            c.push_xy(x, y).unwrap();
        }
        let mut s = ArenaBuilder::new(PayloadKind::Segments);
        for l in &self.segments {
            s.push_u32(l).unwrap();
        }
        AdvancedBlocks {
            records: aligned(&records),
            knots: f64s(PayloadKind::Knots, &self.knots),
            weights: f64s(PayloadKind::Weights, &self.weights),
            control_points: c.finish().unwrap(),
            segments: s.finish().unwrap(),
            trims: f64s(PayloadKind::Trims, &self.trims),
        }
    }
}

fn spline_row(id: u32, degree: u8, flags: u16, k: u32, w: u32, c: u32) -> AdvancedCurveRecord {
    AdvancedCurveRecord {
        id,
        style: 0,
        kind: RecordKind::Spline,
        degree,
        flags,
        knots: k,
        weights: w,
        control_points: c,
        segments: NO_ENTRY,
        trims: NO_ENTRY,
    }
}

fn composite_row(id: u32, segments: u32, trims: u32) -> AdvancedCurveRecord {
    AdvancedCurveRecord {
        id,
        style: 0,
        kind: RecordKind::Composite,
        degree: 0,
        flags: 0,
        knots: NO_ENTRY,
        weights: NO_ENTRY,
        control_points: NO_ENTRY,
        segments,
        trims,
    }
}

fn hand_spline(rational: bool, knots: usize, weights: usize) -> Hand {
    let flags = if rational { FLAG_RATIONAL } else { 0 };
    let mut h = Hand::new(vec![spline_row(1, 3, flags, 0, if rational { 0 } else { NO_ENTRY }, 0)]);
    h.knots.push((0..knots).map(|k| k as f64).collect());
    if rational {
        h.weights.push(vec![1.0; weights]);
    }
    h.cps.push((CUBIC_X.to_vec(), CUBIC_Y.to_vec()));
    h
}

#[test]
fn a_reader_refuses_a_knot_count_that_does_not_match_even_from_another_writer() {
    for n in [8usize, 10] {
        let blocks = hand_spline(false, n, 0).build();
        let v = view(&blocks);
        assert_eq!(v.curve(0).unwrap_err(), ArenaError::KnotCount { knots: n, control_points: 5, degree: 3 });
    }
    let good = hand_spline(false, 9, 0).build();
    assert!(view(&good).curve(0).is_ok());
}

#[test]
fn a_reader_refuses_short_and_long_weight_arrays_from_another_writer() {
    for n in [4usize, 6] {
        let blocks = hand_spline(true, 9, n).build();
        assert_eq!(
            view(&blocks).curve(0).unwrap_err(),
            ArenaError::WeightCount { weights: n, control_points: 5 }
        );
    }
    assert!(view(&hand_spline(true, 9, 5).build()).curve(0).is_ok());
}

#[test]
fn a_reader_refuses_an_entry_index_that_points_nowhere() {
    let mut h = hand_spline(false, 9, 0);
    h.rows[0].control_points = 4;
    let blocks = h.build();
    assert_eq!(view(&blocks).curve(0).unwrap_err(), ArenaError::EntryOutOfRange { index: 4, entries: 1 });
}

// ---- criterion 4: sharing, and determinism ------------------------------------------------------

fn two_sharing_knots() -> AdvancedBuilder {
    let mut b = AdvancedBuilder::new();
    let x2 = [0.0, 1.0, 3.0, 3.0, 5.0];
    b.push_spline(id(1), 0, cubic()).unwrap();
    b.push_spline(id(2), 0, spec(&CUBIC_KNOTS, &x2, &CUBIC_Y, None)).unwrap();
    b
}

#[test]
fn two_entities_with_an_identical_knot_vector_share_one_arena_range() {
    let blocks = two_sharing_knots().finish().unwrap();
    let v = view(&blocks);
    let (a, b) = (v.record(0).unwrap(), v.record(1).unwrap());
    assert_eq!(a.knots, b.knots, "one table entry for both");
    assert_ne!(a.control_points, b.control_points, "their control points differ");
    let knots = ArenaView::parse(blocks.knots.as_bytes()).unwrap();
    assert_eq!(knots.entries(), 1);
    assert_eq!(knots.arena_len(), 9, "the vector is stored once, not twice");
    let (ka, kb) = (spline(v.curve(0).unwrap()).knots, spline(v.curve(1).unwrap()).knots);
    assert_eq!(ka.as_ptr(), kb.as_ptr(), "the same memory");
}

#[test]
fn knot_vectors_one_ulp_apart_are_not_shared() {
    let mut other = CUBIC_KNOTS;
    other[4] = f64::from_bits(other[4].to_bits() + 1);
    let mut b = AdvancedBuilder::new();
    b.push_spline(id(1), 0, cubic()).unwrap();
    b.push_spline(id(2), 0, spec(&other, &CUBIC_X, &CUBIC_Y, None)).unwrap();
    let blocks = b.finish().unwrap();
    assert_eq!(ArenaView::parse(blocks.knots.as_bytes()).unwrap().entries(), 2);
    // and the second one still reads back its own bits
    assert_eq!(spline(view(&blocks).curve(1).unwrap()).knots[4].to_bits(), other[4].to_bits());
}

type Push = Box<dyn Fn(&mut AdvancedBuilder)>;

fn all_curves() -> Vec<Push> {
    vec![
        Box::new(|b| b.push_spline(id(5), 1, cubic()).unwrap()),
        Box::new(|b| {
            b.push_spline(id(2), 2, spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&WEIGHTS))).unwrap()
        }),
        Box::new(|b| {
            let x = [9.0, 8.0, 7.0, 6.0, 5.0];
            b.push_spline(id(9), 3, spec(&CUBIC_KNOTS, &x, &CUBIC_Y, None)).unwrap()
        }),
        Box::new(|b| b.push_composite(id(20), 4, &[id(2), id(5), id(9)], None, false).unwrap()),
        Box::new(|b| {
            b.push_composite(id(21), 5, &[id(20), id(5)], Some(&[[0.0, 1.0], [0.5, 0.75]]), false).unwrap()
        }),
    ]
}

fn written(order: &[usize]) -> Vec<Vec<u8>> {
    let curves = all_curves();
    let mut b = AdvancedBuilder::new();
    for &i in order {
        curves[i](&mut b);
    }
    let o = b.finish().unwrap();
    [&o.records, &o.knots, &o.weights, &o.control_points, &o.segments, &o.trims]
        .iter()
        .map(|a| a.as_bytes().to_vec())
        .collect()
}

#[test]
fn the_output_is_byte_identical_across_runs_and_across_push_orders_with_sharing() {
    let first = written(&[0, 1, 2, 3, 4]);
    assert_eq!(first, written(&[0, 1, 2, 3, 4]), "a second run");
    for order in [[4, 3, 2, 1, 0], [2, 4, 0, 3, 1], [1, 0, 4, 2, 3], [3, 4, 1, 2, 0]] {
        assert_eq!(first, written(&order), "push order {order:?} changed the bytes");
    }
    // sharing really happened in what was compared
    let knots = ArenaView::parse(AlignedBytes::from_bytes(&first[1]).as_bytes()).map(|v| v.entries());
    assert_eq!(knots, Ok(1));
}

// ---- criterion 5: the read cost -----------------------------------------------------------------

#[derive(Default)]
struct Counter {
    tables: [Cell<usize>; 5],
    payloads: [Cell<usize>; 5],
}

fn slot(kind: PayloadKind) -> usize {
    PayloadKind::ALL.iter().position(|k| *k == kind).unwrap()
}

impl ReadProbe for Counter {
    fn table_read(&self, kind: PayloadKind) {
        let c = &self.tables[slot(kind)];
        c.set(c.get() + 1);
    }
    fn payload_read(&self, kind: PayloadKind) {
        let c = &self.payloads[slot(kind)];
        c.set(c.get() + 1);
    }
}

impl Counter {
    fn totals(&self) -> ([usize; 5], [usize; 5]) {
        (self.tables.clone().map(|c| c.get()), self.payloads.clone().map(|c| c.get()))
    }
    fn reset(&self) {
        for c in self.tables.iter().chain(&self.payloads) {
            c.set(0);
        }
    }
}

// order of PayloadKind::ALL: knots, weights, control points, segments, trims

#[test]
fn reading_a_rational_curve_is_one_table_read_and_one_arena_read_per_payload_kind() {
    let mut b = AdvancedBuilder::new();
    b.push_spline(id(1), 0, spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&WEIGHTS))).unwrap();
    let blocks = b.finish().unwrap();
    let probe = Counter::default();
    let v = AdvancedView::parse_probed(blocks.slices(), &probe).unwrap();
    probe.reset();
    let _ = v.record(0).unwrap();
    assert_eq!(probe.totals(), ([0; 5], [0; 5]), "the record itself is in the record block, not an arena");
    v.curve(0).unwrap();
    assert_eq!(probe.totals(), ([1, 1, 1, 0, 0], [1, 1, 1, 0, 0]));
}

#[test]
fn a_non_rational_curve_does_not_read_the_weights_arena_at_all() {
    let blocks = push_one(cubic()).unwrap();
    let probe = Counter::default();
    let v = AdvancedView::parse_probed(blocks.slices(), &probe).unwrap();
    probe.reset();
    v.curve(0).unwrap();
    assert_eq!(probe.totals(), ([1, 0, 1, 0, 0], [1, 0, 1, 0, 0]));
}

#[test]
fn a_composite_reads_its_segment_list_and_trims_once_each_and_follows_nothing() {
    let blocks = one_of_each();
    let probe = Counter::default();
    let v = AdvancedView::parse_probed(blocks.slices(), &probe).unwrap();
    let comp = (0..v.len()).find(|&i| v.record(i).unwrap().kind == RecordKind::Composite).unwrap();
    probe.reset();
    let c = composite(v.curve(comp).unwrap());
    assert!(!c.segments.is_empty());
    assert_eq!(probe.totals(), ([0, 0, 0, 1, 1], [0, 0, 0, 1, 1]));
}

#[test]
fn the_cost_of_one_curve_does_not_grow_with_the_number_of_curves_in_the_file() {
    let mut costs = Vec::new();
    for n in [1u32, 50, 2000] {
        let mut b = AdvancedBuilder::new();
        for i in 1..=n {
            let k = f64::from(i);
            let x = [k, k + 1.0, k + 2.0, k + 3.0, k + 4.0];
            let knots = [0.0, 0.0, 0.0, 0.0, 0.25 + 0.5 / (k + 1.0), 1.0, 1.0, 1.0, 1.0];
            b.push_spline(id(i), 0, spec(&knots, &x, &CUBIC_Y, Some(&WEIGHTS))).unwrap();
        }
        let blocks = b.finish().unwrap();
        let probe = Counter::default();
        let v = AdvancedView::parse_probed(blocks.slices(), &probe).unwrap();
        probe.reset();
        v.curve(v.len() - 1).unwrap();
        costs.push(probe.totals());
    }
    assert!(costs.iter().all(|c| *c == ([1, 1, 1, 0, 0], [1, 1, 1, 0, 0])), "{costs:?}");
}

// ---- criterion 6: composites --------------------------------------------------------------------

fn spline_at(b: &mut AdvancedBuilder, n: u32) {
    let k = f64::from(n);
    let x = [k, k + 1.0, k + 2.0, k + 3.0, k + 4.0];
    b.push_spline(id(n), 0, spec(&CUBIC_KNOTS, &x, &CUBIC_Y, None)).unwrap();
}

fn one_of_each() -> AdvancedBlocks {
    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    b.push_spline(id(2), 0, spec(&CUBIC_KNOTS, &CUBIC_X, &CUBIC_Y, Some(&WEIGHTS))).unwrap();
    b.push_composite(id(3), 7, &[id(1), id(2)], Some(&[[0.0, 1.0], [0.25, 0.75]]), false).unwrap();
    b.finish().unwrap()
}

#[test]
fn a_composite_is_an_index_list_of_other_records_with_its_trims() {
    let blocks = one_of_each();
    let v = view(&blocks);
    let c = composite(v.curve(2).unwrap());
    assert_eq!((c.id, c.style, c.closed), (3, 7, false));
    assert_eq!(c.segments, &[0, 1], "record indices, in order");
    assert_eq!(c.trims.unwrap(), &[0.0, 1.0, 0.25, 0.75]);
    assert_eq!(v.nesting_depth(2).unwrap(), 2);
    assert_eq!(v.nesting_depth(0).unwrap(), 1);
}

#[test]
fn a_segment_two_composites_share_is_stored_once() {
    let mut b = AdvancedBuilder::new();
    for n in 1..=3 {
        spline_at(&mut b, n);
    }
    b.push_composite(id(10), 0, &[id(1), id(2)], None, false).unwrap();
    b.push_composite(id(11), 0, &[id(2), id(3)], None, false).unwrap();
    let blocks = b.finish().unwrap();
    let v = view(&blocks);
    // ids 1, 2, 3, 10, 11 are records 0..5; both lists name record 1 for the shared spline
    assert_eq!(composite(v.curve(3).unwrap()).segments, &[0, 1]);
    assert_eq!(composite(v.curve(4).unwrap()).segments, &[1, 2]);
    // three splines' geometry, once each: the composites added no control points or knots of their own
    assert_eq!(ArenaView::parse(blocks.control_points.as_bytes()).unwrap().entries(), 3);
    assert_eq!(ArenaView::parse(blocks.control_points.as_bytes()).unwrap().arena_len(), 15);
    assert_eq!(ArenaView::parse(blocks.segments.as_bytes()).unwrap().arena_len(), 4);
}

#[test]
fn a_diamond_of_shared_composites_is_not_a_cycle() {
    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    b.push_composite(id(2), 0, &[id(1)], None, false).unwrap();
    b.push_composite(id(3), 0, &[id(1)], None, false).unwrap();
    b.push_composite(id(4), 0, &[id(2), id(3)], None, false).unwrap();
    let blocks = b.finish().unwrap();
    assert_eq!(view(&blocks).nesting_depth(3).unwrap(), 3);
}

#[test]
fn the_builder_refuses_a_cycle_and_a_self_reference() {
    let mut b = AdvancedBuilder::new();
    b.push_composite(id(1), 0, &[id(2)], None, false).unwrap();
    b.push_composite(id(2), 0, &[id(1)], None, false).unwrap();
    let e = b.finish().unwrap_err();
    assert!(matches!(e, ArenaError::CompositeCycle { .. }), "{e:?}");
    assert_eq!(e.rule(), "ARN-21");

    let mut b = AdvancedBuilder::new();
    b.push_composite(id(1), 0, &[id(1)], None, false).unwrap();
    assert!(matches!(b.finish().unwrap_err(), ArenaError::CompositeCycle { .. }));
}

#[test]
fn a_cycle_written_by_somebody_else_is_refused_and_not_traversed() {
    // record 0 (id 1) lists record 1, record 1 (id 2) lists record 0
    let mut h = Hand::new(vec![composite_row(1, 0, NO_ENTRY), composite_row(2, 1, NO_ENTRY)]);
    h.segments = vec![vec![1], vec![0]];
    let blocks = h.build();
    let probe = Counter::default();
    let v = AdvancedView::parse_probed(blocks.slices(), &probe).unwrap();
    probe.reset();
    let e = v.nesting_depth(0).unwrap_err();
    assert!(matches!(e, ArenaError::CompositeCycle { .. }), "{e:?}");
    let (tables, _) = probe.totals();
    assert!(tables[3] <= 2, "each list is read at most once, not followed round and round: {tables:?}");
}

#[test]
fn nesting_is_limited_to_the_numeric_policy_depth_counting_the_outermost_as_one() {
    // a spline under three composites is depth 4, the limit
    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    for n in 2..=4 {
        b.push_composite(id(n), 0, &[id(n - 1)], None, false).unwrap();
    }
    let ok = b.clone().finish().unwrap();
    assert_eq!(view(&ok).nesting_depth(3).unwrap(), 4);
    // one more composite makes it 5
    b.push_composite(id(5), 0, &[id(4)], None, false).unwrap();
    let e = b.finish().unwrap_err();
    assert!(matches!(e, ArenaError::NestingTooDeep { .. }), "{e:?}");
    assert_eq!(e.rule(), "ARN-22");
}

#[test]
fn a_reader_checks_nesting_depth_of_a_chain_from_another_writer() {
    let mut rows = vec![spline_row(1, 3, 0, 0, NO_ENTRY, 0)];
    let mut segs = Vec::new();
    for n in 1..=4u32 {
        rows.push(composite_row(n + 1, n - 1, NO_ENTRY));
        segs.push(vec![n - 1]);
    }
    let mut h = Hand::new(rows);
    h.knots.push(CUBIC_KNOTS.to_vec());
    h.cps.push((CUBIC_X.to_vec(), CUBIC_Y.to_vec()));
    h.segments = segs;
    let blocks = h.build();
    let v = view(&blocks);
    assert_eq!(v.nesting_depth(3).unwrap(), 4);
    assert!(matches!(v.nesting_depth(4).unwrap_err(), ArenaError::NestingTooDeep { .. }));
}

#[test]
fn a_segment_naming_a_record_that_does_not_exist_is_refused() {
    let mut h = Hand::new(vec![composite_row(1, 0, NO_ENTRY)]);
    h.segments = vec![vec![5]];
    let blocks = h.build();
    assert_eq!(view(&blocks).curve(0).unwrap_err(), ArenaError::SegmentOutOfRange { record: 0, segment: 5 });
    let mut b = AdvancedBuilder::new();
    b.push_composite(id(1), 0, &[id(77)], None, false).unwrap();
    assert_eq!(b.finish().unwrap_err(), ArenaError::UnknownSegment { id: 77 });
}

#[test]
fn trims_are_two_numbers_per_segment_and_finite() {
    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    spline_at(&mut b, 2);
    let e = b.push_composite(id(3), 0, &[id(1), id(2)], Some(&[[0.0, 1.0]]), false).unwrap_err();
    assert_eq!(e, ArenaError::TrimCount { values: 2, segments: 2 });
    assert_eq!(e.rule(), "ARN-23");

    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    assert!(b.push_composite(id(3), 0, &[id(1)], Some(&[[0.0, f64::NAN]]), false).is_err());

    let mut h = Hand::new(vec![composite_row(1, 0, 0)]);
    h.segments = vec![vec![0, 0]];
    h.trims = vec![vec![0.0, 1.0, 0.0]];
    let blocks = h.build();
    assert_eq!(view(&blocks).curve(0).unwrap_err(), ArenaError::TrimCount { values: 3, segments: 2 });
}

#[test]
fn an_empty_segment_list_and_an_empty_builder_are_refused() {
    let mut b = AdvancedBuilder::new();
    assert!(b.push_composite(id(1), 0, &[], None, false).is_err());
    assert_eq!(AdvancedBuilder::new().finish().unwrap_err(), ArenaError::EmptyBlock);
}

#[test]
fn the_same_id_twice_is_refused() {
    let mut b = AdvancedBuilder::new();
    spline_at(&mut b, 1);
    spline_at(&mut b, 1);
    assert_eq!(b.finish().unwrap_err(), ArenaError::DuplicateId { id: 1 });
}

// ---- hostile input ------------------------------------------------------------------------------

#[test]
fn overwriting_any_word_of_any_block_never_panics() {
    let good = one_of_each();
    let parts =
        [&good.records, &good.knots, &good.weights, &good.control_points, &good.segments, &good.trims];
    for (which, part) in parts.iter().enumerate() {
        for word in [0u32, u32::MAX, 0x8000_0000, 3] {
            for at in (0..part.len()).step_by(4) {
                let mut bytes = part.as_bytes().to_vec();
                bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
                let hit = aligned(&bytes);
                let mut s = good.slices();
                match which {
                    0 => s.records = hit.as_bytes(),
                    1 => s.knots = hit.as_bytes(),
                    2 => s.weights = hit.as_bytes(),
                    3 => s.control_points = hit.as_bytes(),
                    4 => s.segments = hit.as_bytes(),
                    _ => s.trims = hit.as_bytes(),
                }
                if let Ok(v) = AdvancedView::parse(s) {
                    for i in 0..v.len() + 1 {
                        let _ = v.curve(i);
                        let _ = v.nesting_depth(i);
                    }
                }
            }
        }
    }
}

#[test]
fn every_truncation_of_every_block_is_refused_without_a_panic() {
    let good = one_of_each();
    let parts =
        [&good.records, &good.knots, &good.weights, &good.control_points, &good.segments, &good.trims];
    for (which, part) in parts.iter().enumerate() {
        for n in 0..part.len() {
            let cut = aligned(&part.as_bytes()[..n]);
            let mut s = good.slices();
            match which {
                0 => s.records = cut.as_bytes(),
                1 => s.knots = cut.as_bytes(),
                2 => s.weights = cut.as_bytes(),
                3 => s.control_points = cut.as_bytes(),
                4 => s.segments = cut.as_bytes(),
                _ => s.trims = cut.as_bytes(),
            }
            assert!(AdvancedView::parse(s).is_err(), "block {which} cut to {n} bytes was accepted");
        }
    }
}

#[test]
fn an_arena_block_of_the_wrong_kind_in_a_slot_is_refused() {
    let good = one_of_each();
    let mut s = good.slices();
    s.knots = good.weights.as_bytes();
    assert!(matches!(AdvancedView::parse(s), Err(ArenaError::UnknownKind { .. })));
}
