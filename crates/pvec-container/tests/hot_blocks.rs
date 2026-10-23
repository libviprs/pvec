//! Hot type-batched immediate geometry blocks (issue P4.2): borrowed columns, alignment and padding,
//! bit-exact coordinates, refused run offsets and canonical ordering.

use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::hot::*;
use pvec_entity::EntityId;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

/// Two polylines, ids 3 and 7, styles 10 and 11: (0,0) (1,0) (1,1) and (2,2) (3,3). Written by hand from
/// the table in `docs/CONTAINER.md` chapter 2, not produced by the code under test. The four bytes at
/// 60..64 are the padding that brings the first `f64` column to a multiple of 8.
#[rustfmt::skip]
const POLYLINES: [u8; 144] = [
    0x50, 0x56, 0x48, 0x31,                         // magic PVH1
    0x01, 0x00,                                     // version 1
    0x02, 0x00,                                     // kind 2 (polyline)
    0x02, 0x00, 0x00, 0x00,                         // entity_count 2
    0x05, 0x00, 0x00, 0x00,                         // vertex_count 5
    0x90, 0x00, 0x00, 0x00,                         // block_length 144
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    0x03, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, // ids 3, 7
    0x0a, 0x00, 0x00, 0x00, 0x0b, 0x00, 0x00, 0x00, // styles 10, 11
    0x00, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, // run offsets 0, 3, 5
    0x00, 0x00, 0x00, 0x00,                         // padding to 64
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // x 0.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // x 1.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // x 1.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // x 2.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x40, // x 3.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // y 0.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // y 0.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // y 1.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // y 2.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x40, // y 3.0
];

fn polylines_via_builder(order: &[(u32, u32)]) -> AlignedBytes {
    let runs: [(u32, u32, &[[f64; 2]]); 2] =
        [(3, 10, &[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]), (7, 11, &[[2.0, 2.0], [3.0, 3.0]])];
    let mut b = HotBlockBuilder::new(HotKind::Polyline);
    for &(i, _) in order {
        let (n, s, pts) = runs.iter().find(|r| r.0 == i).unwrap();
        b.push_polyline(id(*n), *s, pts).unwrap();
    }
    b.finish().unwrap()
}

fn patched(at: usize, patch: &[u8]) -> AlignedBytes {
    let mut v = POLYLINES.to_vec();
    v[at..at + patch.len()].copy_from_slice(patch);
    AlignedBytes::from_bytes(&v)
}

fn refused(bytes: &AlignedBytes) -> HotError {
    match HotBlockView::parse(bytes.as_bytes()) {
        Ok(_) => panic!("the block was read, expected a refusal"),
        Err(e) => e,
    }
}

// A block of every kind, odd and even sizes, for the alignment and pointer tests.
fn every_kind(n: u32) -> Vec<(HotKind, AlignedBytes)> {
    let mut out = Vec::new();
    for kind in HotKind::ALL {
        let mut b = HotBlockBuilder::new(kind);
        for i in 0..n {
            let f = f64::from(i) + 0.5;
            match kind {
                HotKind::Line => b.push_line(id(i), i, [f, f + 1.0], [f + 2.0, f + 3.0]).unwrap(),
                HotKind::Polyline => {
                    let pts: Vec<[f64; 2]> =
                        (0..2 + i % 3).map(|k| [f + f64::from(k), f - f64::from(k)]).collect();
                    b.push_polyline(id(i), i, &pts).unwrap()
                }
                HotKind::CircleArc => b.push_circle_arc(id(i), i, [f, f], f + 1.0, [0.25, 1.5]).unwrap(),
                HotKind::EllipseArc => {
                    b.push_ellipse_arc(id(i), i, [f, f], [f + 2.0, f + 1.0], [0.1, 0.2, 0.3]).unwrap()
                }
            }
        }
        out.push((kind, b.finish().unwrap()));
    }
    out
}

#[test]
fn the_byte_literal_polyline_block_parses_to_the_columns_written_by_hand() {
    let buf = AlignedBytes::from_bytes(&POLYLINES);
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    assert_eq!(v.kind(), HotKind::Polyline);
    assert_eq!(v.len(), 2);
    assert_eq!(v.entity_ids(), &[3, 7]);
    assert_eq!(v.styles(), &[10, 11]);
    assert_eq!(v.run_offsets(), &[0, 3, 5]);
    assert_eq!(v.column("x").unwrap(), &[0.0, 1.0, 1.0, 2.0, 3.0]);
    assert_eq!(v.column("y").unwrap(), &[0.0, 0.0, 1.0, 2.0, 3.0]);
    let (x, y) = v.polyline(1).unwrap();
    assert_eq!((x, y), (&[2.0, 3.0][..], &[2.0, 3.0][..]));
    assert!(v.polyline(2).is_none());
}

#[test]
fn the_builder_emits_exactly_the_byte_literal_including_the_padding() {
    let built = polylines_via_builder(&[(3, 10), (7, 11)]);
    assert_eq!(built.as_bytes(), &POLYLINES[..]);
    assert_eq!(
        &built.as_bytes()[60..64],
        &[0, 0, 0, 0],
        "four zero bytes of padding before the first f64 column"
    );
}

#[test]
fn a_non_zero_padding_byte_is_refused() {
    for at in 60..64 {
        assert!(matches!(refused(&patched(at, &[1])), HotError::NonZeroPadding), "byte {at}");
    }
}

#[test]
fn a_non_zero_reserved_byte_is_refused() {
    for at in 20..32 {
        assert!(matches!(refused(&patched(at, &[1])), HotError::NonZeroReserved), "byte {at}");
    }
}

#[test]
fn every_column_starts_at_its_natural_alignment_for_every_kind() {
    for n in [1u32, 2, 3, 5, 8] {
        for (kind, buf) in every_kind(n) {
            let base = buf.as_bytes().as_ptr() as usize;
            assert_eq!(base % 8, 0, "the buffer itself starts on 8");
            assert_eq!(buf.as_bytes().len() % 8, 0, "{kind:?} n={n}: a block is a multiple of 8 long");
            let v = HotBlockView::parse(buf.as_bytes()).unwrap();
            for c in [v.entity_ids(), v.styles(), v.run_offsets()] {
                if !c.is_empty() {
                    assert_eq!((c.as_ptr() as usize - base) % 4, 0, "{kind:?} n={n}: a u32 column");
                }
            }
            assert_eq!(v.f64_columns().len(), kind.f64_columns().len());
            for c in v.f64_columns() {
                assert_eq!((c.as_ptr() as usize - base) % 8, 0, "{kind:?} n={n}: an f64 column");
            }
        }
    }
}

#[test]
fn column_slices_are_borrowed_from_the_block_buffer() {
    for (kind, buf) in every_kind(5) {
        let bytes = buf.as_bytes();
        let (lo, hi) = (bytes.as_ptr() as usize, bytes.as_ptr() as usize + bytes.len());
        let v = HotBlockView::parse(bytes).unwrap();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        let mut take = |ptr: usize, len: usize, size: usize, what: &str| {
            assert!(len > 0, "{kind:?}: {what} is empty");
            assert!(ptr >= lo && ptr + len * size <= hi, "{kind:?}: {what} points outside the block buffer");
            spans.push((ptr, ptr + len * size));
        };
        take(v.entity_ids().as_ptr() as usize, v.entity_ids().len(), 4, "ids");
        take(v.styles().as_ptr() as usize, v.styles().len(), 4, "styles");
        if kind == HotKind::Polyline {
            take(v.run_offsets().as_ptr() as usize, v.run_offsets().len(), 4, "run offsets");
        }
        for (name, c) in kind.f64_columns().iter().zip(v.f64_columns()) {
            take(c.as_ptr() as usize, c.len(), 8, name);
            assert_eq!(v.column(name).unwrap().as_ptr(), c.as_ptr(), "{name} by name is the same column");
        }
        spans.sort();
        for w in spans.windows(2) {
            assert!(w[0].1 <= w[1].0, "{kind:?}: two columns overlap");
        }
    }
}

#[test]
fn a_view_outlives_nothing_it_was_not_borrowed_from() {
    // The view is Copy and holds only slices of the buffer; copying it copies no data.
    let buf = AlignedBytes::from_bytes(&POLYLINES);
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    let w = v;
    assert_eq!(v.entity_ids().as_ptr(), w.entity_ids().as_ptr());
}

#[test]
fn coordinates_round_trip_bit_for_bit_including_the_last_mantissa_bit() {
    fn up(x: f64) -> f64 {
        f64::from_bits(x.to_bits() + 1)
    }
    let base = [1.0, 0.1, core::f64::consts::PI, 549_755_813_887.5, 1.0e-300, -123.456];
    let mut vals: Vec<f64> = Vec::new();
    for v in base {
        vals.push(v);
        vals.push(up(v));
    }
    let bits = |s: &[f64]| s.iter().map(|v| v.to_bits()).collect::<Vec<_>>();

    let mut b = HotBlockBuilder::new(HotKind::Line);
    for (i, w) in vals.chunks(4).enumerate() {
        let mut q = [0.0; 4];
        q[..w.len()].copy_from_slice(w);
        b.push_line(id(i as u32), 0, [q[0], q[1]], [q[2], q[3]]).unwrap();
    }
    let buf = b.finish().unwrap();
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    let mut flat = Vec::new();
    for i in 0..v.len() {
        for c in v.f64_columns() {
            flat.push(c[i]);
        }
    }
    let mut want = Vec::new();
    for w in vals.chunks(4) {
        let mut q = [0.0; 4];
        q[..w.len()].copy_from_slice(w);
        want.extend_from_slice(&q);
    }
    assert_eq!(bits(&flat), bits(&want));

    let pts: Vec<[f64; 2]> = vals.chunks(2).map(|c| [c[0], c[1]]).collect();
    let mut b = HotBlockBuilder::new(HotKind::Polyline);
    b.push_polyline(id(0), 0, &pts).unwrap();
    let buf = b.finish().unwrap();
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    let (x, y) = v.polyline(0).unwrap();
    assert_eq!(bits(x), bits(&pts.iter().map(|p| p[0]).collect::<Vec<_>>()));
    assert_eq!(bits(y), bits(&pts.iter().map(|p| p[1]).collect::<Vec<_>>()));

    let mut b = HotBlockBuilder::new(HotKind::EllipseArc);
    b.push_ellipse_arc(id(0), 0, [vals[0], vals[1]], [vals[2], vals[3]], [vals[4], vals[5], vals[6]])
        .unwrap();
    let buf = b.finish().unwrap();
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    let got: Vec<u64> = v.f64_columns().iter().map(|c| c[0].to_bits()).collect();
    assert_eq!(got, bits(&vals[..7]));
}

#[test]
fn a_run_offset_column_that_decreases_is_refused_not_read() {
    // offsets sit at 48, 52, 56: 0, 3, 5. Make them 0, 6, 5.
    assert!(matches!(
        refused(&patched(52, &6u32.to_le_bytes())),
        HotError::RunOffsetsNotMonotonic { index: 2 }
    ));
    // and 0, 3, 5 with the first one raised past the second
    assert!(matches!(
        refused(&patched(48, &4u32.to_le_bytes())),
        HotError::RunOffsetsBadBounds | HotError::RunOffsetsNotMonotonic { .. }
    ));
}

#[test]
fn run_offsets_that_do_not_start_at_zero_or_end_at_the_vertex_count_are_refused() {
    assert!(matches!(refused(&patched(48, &1u32.to_le_bytes())), HotError::RunOffsetsBadBounds));
    assert!(matches!(refused(&patched(56, &4u32.to_le_bytes())), HotError::RunOffsetsBadBounds));
    assert!(matches!(refused(&patched(56, &6u32.to_le_bytes())), HotError::RunOffsetsBadBounds));
}

#[test]
fn a_polyline_run_of_fewer_than_two_vertices_is_refused() {
    // 0, 1, 5: the first run has one vertex
    assert!(matches!(refused(&patched(52, &1u32.to_le_bytes())), HotError::BadRunLength { index: 0 }));
}

#[test]
fn entities_are_ordered_by_id_whatever_order_they_were_pushed_in() {
    let mut a = HotBlockBuilder::new(HotKind::Line);
    for n in [5u32, 1, 9, 3] {
        a.push_line(id(n), n * 2, [f64::from(n) * 10.0, 0.0], [0.0, 1.0]).unwrap();
    }
    let buf = a.finish().unwrap();
    let v = HotBlockView::parse(buf.as_bytes()).unwrap();
    assert_eq!(v.entity_ids(), &[1, 3, 5, 9]);
    assert_eq!(v.styles(), &[2, 6, 10, 18], "the style column moves with its entity");
    assert_eq!(v.column("x0").unwrap(), &[10.0, 30.0, 50.0, 90.0], "so do the coordinates");
}

#[test]
fn the_same_entities_in_any_push_order_give_the_same_bytes() {
    let forward = polylines_via_builder(&[(3, 10), (7, 11)]);
    let backward = polylines_via_builder(&[(7, 11), (3, 10)]);
    assert_eq!(forward.as_bytes(), backward.as_bytes());
    assert_eq!(forward.as_bytes(), &POLYLINES[..]);
}

#[test]
fn a_repeated_id_is_refused_by_the_builder() {
    let mut b = HotBlockBuilder::new(HotKind::Line);
    b.push_line(id(4), 0, [0.0, 0.0], [1.0, 1.0]).unwrap();
    b.push_line(id(4), 0, [2.0, 2.0], [3.0, 3.0]).unwrap();
    assert!(matches!(b.finish(), Err(HotError::DuplicateId { id: 4 })));
}

#[test]
fn ids_that_are_not_ascending_in_a_block_are_refused_by_the_reader() {
    assert!(matches!(refused(&patched(36, &3u32.to_le_bytes())), HotError::IdsNotAscending { index: 1 }));
    assert!(matches!(refused(&patched(36, &1u32.to_le_bytes())), HotError::IdsNotAscending { index: 1 }));
    assert!(matches!(
        refused(&patched(36, &0x1000_0000u32.to_le_bytes())),
        HotError::EntityIdOutOfRange { id: 0x1000_0000 }
    ));
}

#[test]
fn the_builder_refuses_what_it_cannot_write() {
    let mut b = HotBlockBuilder::new(HotKind::Line);
    assert!(matches!(
        b.push_polyline(id(0), 0, &[[0.0, 0.0], [1.0, 1.0]]),
        Err(HotError::WrongKind { block: HotKind::Line, pushed: HotKind::Polyline })
    ));
    assert!(matches!(b.push_line(id(0), 0, [f64::NAN, 0.0], [1.0, 1.0]), Err(HotError::NonFinite { .. })));
    assert!(matches!(
        b.push_line(id(0), 0, [0.0, f64::INFINITY], [1.0, 1.0]),
        Err(HotError::NonFinite { .. })
    ));
    assert!(matches!(HotBlockBuilder::new(HotKind::Line).finish(), Err(HotError::EmptyBlock)));
    let mut p = HotBlockBuilder::new(HotKind::Polyline);
    assert!(matches!(p.push_polyline(id(0), 0, &[[0.0, 0.0]]), Err(HotError::BadRunLength { .. })));
}

#[test]
fn a_block_cut_short_at_any_length_is_refused_without_panicking() {
    for len in 0..POLYLINES.len() {
        let buf = AlignedBytes::from_bytes(&POLYLINES[..len]);
        match refused(&buf) {
            HotError::Truncated { .. } | HotError::LengthMismatch { .. } => {}
            other => panic!("length {len}: {other:?}"),
        }
    }
    let mut longer = POLYLINES.to_vec();
    longer.extend_from_slice(&[0; 8]);
    assert!(matches!(refused(&AlignedBytes::from_bytes(&longer)), HotError::LengthMismatch { .. }));
}

#[test]
fn a_lying_count_is_refused_before_it_sizes_anything() {
    assert!(matches!(
        refused(&patched(8, &0x0fff_ffffu32.to_le_bytes())),
        HotError::Truncated { .. } | HotError::LengthMismatch { .. }
    ));
    assert!(matches!(refused(&patched(8, &0x1000_0000u32.to_le_bytes())), HotError::TooManyEntities { .. }));
    assert!(matches!(refused(&patched(8, &0u32.to_le_bytes())), HotError::EmptyBlock));
    assert!(matches!(refused(&patched(12, &u32::MAX.to_le_bytes())), HotError::TooManyVertices { .. }));
}

#[test]
fn the_header_fields_are_checked() {
    assert!(matches!(refused(&patched(0, b"PVX1")), HotError::BadMagic));
    assert!(matches!(refused(&patched(4, &2u16.to_le_bytes())), HotError::UnsupportedVersion { found: 2 }));
    assert!(matches!(refused(&patched(6, &9u16.to_le_bytes())), HotError::UnknownKind { id: 9 }));
    assert!(matches!(refused(&patched(6, &0u16.to_le_bytes())), HotError::UnknownKind { id: 0 }));
}

#[test]
fn a_non_finite_coordinate_in_a_block_is_refused() {
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            refused(&patched(64, &v.to_le_bytes())),
            HotError::NonFinite { column: "x", index: 0 }
        ));
        assert!(matches!(
            refused(&patched(136, &v.to_le_bytes())),
            HotError::NonFinite { column: "y", index: 4 }
        ));
    }
}

#[test]
fn a_buffer_that_is_not_8_aligned_is_refused_not_misread() {
    let mut raw = vec![0u8; POLYLINES.len() + 16];
    let base = raw.as_ptr() as usize;
    let off = if (base + 1) % 8 != 0 { 1 } else { 2 };
    raw[off..off + POLYLINES.len()].copy_from_slice(&POLYLINES);
    let slice = &raw[off..off + POLYLINES.len()];
    assert_ne!(slice.as_ptr() as usize % 8, 0);
    assert!(matches!(HotBlockView::parse(slice), Err(HotError::Misaligned)));
}

#[test]
fn kinds_have_stable_ids_and_column_names() {
    let ids: Vec<u16> = HotKind::ALL.iter().map(|k| k.id()).collect();
    assert_eq!(ids, [1, 2, 3, 4]);
    for k in HotKind::ALL {
        assert_eq!(HotKind::from_id(k.id()), Some(k));
    }
    assert_eq!(HotKind::Line.f64_columns(), ["x0", "y0", "x1", "y1"]);
    assert_eq!(HotKind::Polyline.f64_columns(), ["x", "y"]);
    assert_eq!(HotKind::CircleArc.f64_columns().len(), 5);
    assert_eq!(HotKind::EllipseArc.f64_columns().len(), 7);
    assert_eq!(HotError::BadMagic.rule(), "HOT-02");
}

#[test]
fn the_hot_header_table_covers_exactly_the_fixed_header() {
    let mut at = 0;
    for f in HOT_HEADER_FIELDS {
        assert_eq!(f.offset, at, "{}", f.name);
        at += f.size;
    }
    assert_eq!(at, HOT_HEADER_LEN);
    assert_eq!(HOT_HEADER_LEN, 32);
    assert_eq!(HOT_MAGIC, *b"PVH1");
    assert_eq!(&POLYLINES[..HOT_HEADER_LEN][..4], &HOT_MAGIC);
}
