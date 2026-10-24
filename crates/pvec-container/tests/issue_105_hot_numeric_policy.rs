//! Issue 105: the hot block builder, the container writer and the hot block parser apply the numeric
//! policy (NP-01, NP-02, NP-03, NP-06, NP-07), not just `is_finite`.
//!
//! Every value class from the issue is covered twice: the writer must refuse to produce it, and the
//! parser must refuse a block that carries it, naming the HOT rule and the NP rule behind it.

mod support;

use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::hot::*;
use pvec_container::directory::{BlockKind, parse_directory};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError};
use pvec_container::{Bounds, Header, Manifest};
use pvec_entity::EntityId;
use support::*;

const SUBNORMAL: f64 = f64::from_bits(1); // the smallest subnormal
const PAST_2_40: f64 = 2199023255552.0; // 2^41

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn meta(i: u32) -> EntityMeta {
    EntityMeta { id: eid(i), style: 0, cell: 0, bounds: Bounds { min: [0.0, 0.0], max: [1.0, 1.0] } }
}

/// What the rule tables of `docs/CONTAINER.md` chapter 2 say each class is refused under.
#[derive(Clone, Copy, Debug)]
struct Class {
    name: &'static str,
    hot: &'static str,
    np: &'static str,
}

const SUBNORMAL_C: Class = Class { name: "subnormal", hot: "HOT-21", np: "NP-01" };
const RANGE_C: Class = Class { name: "past 2^40", hot: "HOT-22", np: "NP-02" };
const NEG_ZERO_C: Class = Class { name: "negative zero", hot: "HOT-23", np: "NP-03" };
const RADIUS_C: Class = Class { name: "radius", hot: "HOT-24", np: "NP-07" };
const LENGTH_C: Class = Class { name: "zero length", hot: "HOT-25", np: "NP-06" };

/// A refusal names the HOT rule, and its text names the NP rule it comes from.
fn assert_class(e: &HotError, c: Class) {
    assert_eq!(e.rule(), c.hot, "{}: {e:?}", c.name);
    assert!(e.to_string().contains(c.np), "{}: the message does not name {}: {e}", c.name, c.np);
}

fn write_err(push: impl FnOnce(&mut ContainerWriter) -> Result<(), WriteError>) -> WriteError {
    let mut w = ContainerWriter::new(minimal().config);
    if let Err(e) = push(&mut w) {
        return e;
    }
    w.finish().expect_err("the writer produced a file it should have refused")
}

fn assert_write_refused(e: WriteError, c: Class) {
    assert_eq!(e.rule(), "WRT-09", "{}: {e:?}", c.name);
    match e {
        WriteError::Hot(h) => assert_class(&h, c),
        other => panic!("{}: {other:?}", c.name),
    }
}

// ---- the writer -----------------------------------------------------------------------------------

#[test]
fn writer_refuses_a_zero_radius_np07() {
    let e = write_err(|w| w.push_circle_arc(meta(0), [0.0, 0.0], 0.0, [0.0, 1.0]));
    assert_write_refused(e, RADIUS_C);
}

#[test]
fn writer_refuses_a_negative_radius_np07() {
    let e = write_err(|w| w.push_circle_arc(meta(0), [0.0, 0.0], -1.0, [0.0, 1.0]));
    assert_write_refused(e, RADIUS_C);
}

#[test]
fn writer_refuses_a_degenerate_ellipse_semi_axis_np07() {
    let e = write_err(|w| w.push_ellipse_arc(meta(0), [0.0, 0.0], [2.0, 0.0], [0.0, 0.0, 1.0]));
    assert_write_refused(e, RADIUS_C);
    let e = write_err(|w| w.push_ellipse_arc(meta(0), [0.0, 0.0], [2.0, -1.0], [0.0, 0.0, 1.0]));
    assert_write_refused(e, RADIUS_C);
}

#[test]
fn writer_refuses_a_coordinate_past_2_pow_40_np02_in_every_kind() {
    let e = write_err(|w| w.push_line(meta(0), [PAST_2_40, 0.0], [1.0, 1.0]));
    assert_write_refused(e, RANGE_C);
    let e = write_err(|w| w.push_polyline(meta(0), &[[0.0, 0.0], [0.0, -PAST_2_40]]));
    assert_write_refused(e, RANGE_C);
    let e = write_err(|w| w.push_circle_arc(meta(0), [PAST_2_40, 0.0], 1.0, [0.0, 1.0]));
    assert_write_refused(e, RANGE_C);
    let e = write_err(|w| w.push_ellipse_arc(meta(0), [0.0, 0.0], [2.0, 1.0], [0.0, 0.0, PAST_2_40]));
    assert_write_refused(e, RANGE_C);
}

#[test]
fn writer_accepts_a_coordinate_exactly_at_2_pow_40() {
    let mut w = ContainerWriter::new(minimal().config);
    w.push_line(meta(0), [1099511627776.0, -1099511627776.0], [0.0, 1.0]).unwrap();
    w.finish().unwrap();
}

#[test]
fn writer_refuses_a_subnormal_np01_in_every_kind() {
    let e = write_err(|w| w.push_line(meta(0), [SUBNORMAL, 0.0], [1.0, 1.0]));
    assert_write_refused(e, SUBNORMAL_C);
    let e = write_err(|w| w.push_polyline(meta(0), &[[0.0, 0.0], [1.0, SUBNORMAL]]));
    assert_write_refused(e, SUBNORMAL_C);
    let e = write_err(|w| w.push_circle_arc(meta(0), [0.0, 0.0], 1.0, [SUBNORMAL, 1.0]));
    assert_write_refused(e, SUBNORMAL_C);
}

#[test]
fn writer_refuses_a_zero_length_line_np06() {
    let e = write_err(|w| w.push_line(meta(0), [2.0, 3.0], [2.0, 3.0]));
    assert_write_refused(e, LENGTH_C);
}

#[test]
fn writer_refuses_a_zero_length_line_written_with_negative_zero_np06() {
    let e = write_err(|w| w.push_line(meta(0), [0.0, 0.0], [-0.0, -0.0]));
    assert_write_refused(e, LENGTH_C);
}

#[test]
fn writer_refuses_a_polyline_with_a_repeated_vertex_np06() {
    let e = write_err(|w| w.push_polyline(meta(0), &[[0.0, 0.0], [1.0, 1.0], [1.0, 1.0], [2.0, 0.0]]));
    assert_write_refused(e, LENGTH_C);
}

#[test]
fn writer_still_refuses_nan_as_hot_16_control() {
    let e = write_err(|w| w.push_line(meta(0), [f64::NAN, 0.0], [1.0, 1.0]));
    match e {
        WriteError::Hot(h) => assert_eq!(h.rule(), "HOT-16"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn writer_stores_negative_zero_as_positive_zero_np03() {
    let mut w = ContainerWriter::new(minimal().config);
    w.push_line(meta(0), [-0.0, 1.0], [2.0, -0.0]).unwrap();
    w.push_circle_arc(meta(1), [-0.0, -0.0], 1.0, [-0.0, 1.0]).unwrap();
    w.push_polyline(meta(2), &[[-0.0, 0.0], [1.0, -0.0]]).unwrap();
    w.push_ellipse_arc(meta(3), [-0.0, 0.0], [2.0, 1.0], [-0.0, -0.0, 1.0]).unwrap();
    let bytes = w.finish().unwrap();

    let len = bytes.len() as u64;
    let header = Header::parse(&bytes[..bytes.len().min(4096)], len).unwrap();
    let (m0, m1) =
        (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
    let manifest = Manifest::parse(&bytes[m0..m1], &header, len).unwrap();
    let d = manifest.directories[0];
    let entries = parse_directory(
        &bytes[d.offset as usize..(d.offset + d.length) as usize],
        header.header_len as u64,
        len,
    )
    .unwrap();
    let hot: Vec<_> = entries.iter().filter(|e| e.kind == BlockKind::Hot.id()).collect();
    assert_eq!(hot.len(), 4);
    for e in hot {
        let block =
            AlignedBytes::from_bytes(&bytes[e.offset as usize..e.offset as usize + e.stored_length as usize]);
        let v = HotBlockView::parse(block.as_bytes()).unwrap();
        for col in v.f64_columns() {
            for x in *col {
                assert!(x.to_bits() != (-0.0f64).to_bits(), "{:?} holds -0.0", v.kind());
            }
        }
    }
}

// ---- the builder ----------------------------------------------------------------------------------

#[test]
fn builder_refuses_each_class_with_its_own_rule() {
    let mut b = HotBlockBuilder::new(HotKind::CircleArc);
    assert_class(&b.push_circle_arc(id(0), 0, [0.0, 0.0], 0.0, [0.0, 1.0]).unwrap_err(), RADIUS_C);
    assert_class(&b.push_circle_arc(id(0), 0, [0.0, 0.0], -1.0, [0.0, 1.0]).unwrap_err(), RADIUS_C);
    assert_class(&b.push_circle_arc(id(0), 0, [PAST_2_40, 0.0], 1.0, [0.0, 1.0]).unwrap_err(), RANGE_C);
    assert_class(&b.push_circle_arc(id(0), 0, [SUBNORMAL, 0.0], 1.0, [0.0, 1.0]).unwrap_err(), SUBNORMAL_C);
    assert!(b.is_empty(), "a refused entity must not be half pushed");
    let mut b = HotBlockBuilder::new(HotKind::Line);
    assert_class(&b.push_line(id(0), 0, [1.0, 1.0], [1.0, 1.0]).unwrap_err(), LENGTH_C);
    assert!(b.is_empty());
    let mut b = HotBlockBuilder::new(HotKind::Polyline);
    assert_class(&b.push_polyline(id(0), 0, &[[0.0, 0.0], [0.0, 0.0]]).unwrap_err(), LENGTH_C);
    assert!(b.is_empty());
}

#[test]
fn builder_normalises_negative_zero() {
    let mut b = HotBlockBuilder::new(HotKind::Line);
    b.push_line(id(0), 0, [-0.0, 1.0], [2.0, -0.0]).unwrap();
    let block = b.finish().unwrap();
    let v = HotBlockView::parse(block.as_bytes()).unwrap();
    for col in v.f64_columns() {
        assert!(col.iter().all(|x| x.to_bits() != (-0.0f64).to_bits()));
    }
}

#[test]
fn a_closed_polyline_and_a_distant_repeat_are_still_legal() {
    // NP-06 is about consecutive vertices: a ring whose last vertex is its first is fine.
    let mut b = HotBlockBuilder::new(HotKind::Polyline);
    b.push_polyline(id(0), 0, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]).unwrap();
    b.finish().unwrap();
}

// ---- the parser -----------------------------------------------------------------------------------

/// A valid one-entity block of `kind` and the byte offset of each of its `f64` columns.
fn block(kind: HotKind) -> (Vec<u8>, Vec<usize>) {
    let mut b = HotBlockBuilder::new(kind);
    match kind {
        HotKind::Line => b.push_line(id(1), 0, [0.0, 0.0], [3.0, 4.0]).unwrap(),
        HotKind::Polyline => b.push_polyline(id(1), 0, &[[0.0, 0.0], [3.0, 4.0]]).unwrap(),
        HotKind::CircleArc => b.push_circle_arc(id(1), 0, [1.0, 2.0], 5.0, [0.0, 1.0]).unwrap(),
        HotKind::EllipseArc => b.push_ellipse_arc(id(1), 0, [1.0, 2.0], [5.0, 3.0], [0.0, 0.0, 1.0]).unwrap(),
    }
    let bytes = b.finish().unwrap().as_bytes().to_vec();
    // 32 header, then ids and styles (4 each), then for a polyline the 2 run offsets (8)
    let start = if kind == HotKind::Polyline { 48 } else { 40 };
    let len = if kind == HotKind::Polyline { 2 } else { 1 };
    let cols = (0..kind.f64_columns().len()).map(|c| start + 8 * len * c).collect();
    (bytes, cols)
}

fn with(kind: HotKind, patches: &[(usize, usize, f64)]) -> Result<(), HotError> {
    let (mut bytes, cols) = block(kind);
    for &(c, e, v) in patches {
        let at = cols[c] + 8 * e;
        bytes[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }
    let aligned = AlignedBytes::from_bytes(&bytes);
    HotBlockView::parse(aligned.as_bytes()).map(|_| ())
}

fn refusal(r: Result<(), HotError>, c: Class) {
    assert_class(&r.expect_err(&format!("the parser accepted a {} value", c.name)), c);
}

#[test]
fn the_valid_blocks_used_here_parse() {
    for k in HotKind::ALL {
        with(k, &[]).unwrap();
    }
}

#[test]
fn parser_refuses_a_zero_and_a_negative_radius_np07() {
    refusal(with(HotKind::CircleArc, &[(2, 0, 0.0)]), RADIUS_C);
    refusal(with(HotKind::CircleArc, &[(2, 0, -1.0)]), RADIUS_C);
    refusal(with(HotKind::EllipseArc, &[(2, 0, 0.0)]), RADIUS_C);
    refusal(with(HotKind::EllipseArc, &[(3, 0, -2.0)]), RADIUS_C);
}

#[test]
fn parser_refuses_a_coordinate_past_2_pow_40_np02_in_every_kind() {
    refusal(with(HotKind::Line, &[(0, 0, PAST_2_40)]), RANGE_C);
    refusal(with(HotKind::Polyline, &[(1, 0, -PAST_2_40)]), RANGE_C);
    refusal(with(HotKind::CircleArc, &[(0, 0, PAST_2_40)]), RANGE_C);
    refusal(with(HotKind::CircleArc, &[(4, 0, PAST_2_40)]), RANGE_C);
    refusal(with(HotKind::EllipseArc, &[(6, 0, PAST_2_40)]), RANGE_C);
}

#[test]
fn parser_refuses_a_subnormal_np01_in_every_kind() {
    refusal(with(HotKind::Line, &[(0, 0, SUBNORMAL)]), SUBNORMAL_C);
    refusal(with(HotKind::Polyline, &[(0, 0, f64::MIN_POSITIVE / 4.0)]), SUBNORMAL_C);
    refusal(with(HotKind::CircleArc, &[(3, 0, SUBNORMAL)]), SUBNORMAL_C);
    refusal(with(HotKind::EllipseArc, &[(4, 0, -SUBNORMAL)]), SUBNORMAL_C);
}

#[test]
fn parser_refuses_negative_zero_np03_so_the_bytes_stay_canonical() {
    refusal(with(HotKind::Line, &[(0, 0, -0.0)]), NEG_ZERO_C);
    refusal(with(HotKind::Polyline, &[(1, 1, -0.0)]), NEG_ZERO_C);
    refusal(with(HotKind::CircleArc, &[(1, 0, -0.0)]), NEG_ZERO_C);
    refusal(with(HotKind::EllipseArc, &[(5, 0, -0.0)]), NEG_ZERO_C);
}

#[test]
fn parser_refuses_a_zero_length_line_np06() {
    // end point patched onto the start point (0, 0)
    refusal(with(HotKind::Line, &[(2, 0, 0.0), (3, 0, 0.0)]), LENGTH_C);
}

#[test]
fn parser_refuses_a_polyline_with_a_repeated_vertex_np06() {
    // x = [0, 3], y = [0, 4]: the second vertex becomes (0, 0), the first
    refusal(with(HotKind::Polyline, &[(0, 1, 0.0), (1, 1, 0.0)]), LENGTH_C);
}

#[test]
fn parser_still_refuses_nan_as_hot_16_control() {
    let e = with(HotKind::Line, &[(0, 0, f64::NAN)]).unwrap_err();
    assert_eq!(e.rule(), "HOT-16");
}
