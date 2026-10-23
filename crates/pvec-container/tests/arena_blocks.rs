//! Arena blocks and offset tables (issue P4.3, acceptance criteria 1 and 5): entries validated before
//! a slice is handed out, borrowed payloads, natural alignment, and the read cost of one curve.

use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::arena::*;
use std::cell::Cell;

fn aligned(bytes: &[u8]) -> AlignedBytes {
    AlignedBytes::from_bytes(bytes)
}

/// A knots arena with three table entries, written by hand from the table in `docs/CONTAINER.md`
/// chapter 3, not produced by the code under test: entries (0, 4), (4, 2) and (0, 4) again, so two
/// entries share one range, over the six values 0.0, 0.0, 1.0, 1.0, 0.5, 0.25.
#[rustfmt::skip]
const KNOTS: [u8; 104] = [
    0x50, 0x56, 0x41, 0x31,                         // magic PVA1
    0x01, 0x00,                                     // version 1
    0x01, 0x00,                                     // payload kind 1 (knots)
    0x03, 0x00, 0x00, 0x00,                         // entry_count 3
    0x06, 0x00, 0x00, 0x00,                         // element_count 6
    0x68, 0x00, 0x00, 0x00,                         // block_length 104
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // reserved
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, // entry 0: start 0, len 4
    0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, // entry 1: start 4, len 2
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, // entry 2: start 0, len 4
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // 0.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // 1.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f, // 1.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe0, 0x3f, // 0.5
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f, // 0.25
];

fn patched(at: usize, patch: &[u8]) -> AlignedBytes {
    let mut v = KNOTS.to_vec();
    v[at..at + patch.len()].copy_from_slice(patch);
    aligned(&v)
}

fn refused(bytes: &AlignedBytes) -> ArenaError {
    match ArenaView::parse(bytes.as_bytes()) {
        Ok(_) => panic!("the block was read, expected a refusal"),
        Err(e) => e,
    }
}

#[test]
fn a_hand_written_knots_arena_is_read_back_value_for_value() {
    let buf = aligned(&KNOTS);
    let v = ArenaView::parse(buf.as_bytes()).expect("the literal parses");
    assert_eq!(v.kind(), PayloadKind::Knots);
    assert_eq!(v.entries(), 3);
    assert_eq!(v.arena_len(), 6);
    assert_eq!(v.entry(1).unwrap(), OffsetEntry { start: 4, len: 2 });
    assert_eq!(v.slice(0).unwrap(), &[0.0, 0.0, 1.0, 1.0]);
    assert_eq!(v.slice(1).unwrap(), &[0.5, 0.25]);
    assert_eq!(v.slice(2).unwrap(), v.slice(0).unwrap(), "two entries may share one range");
}

#[test]
fn a_slice_is_borrowed_from_the_block_buffer_and_never_copied() {
    let buf = aligned(&KNOTS);
    let bytes = buf.as_bytes();
    let v = ArenaView::parse(bytes).unwrap();
    let lo = bytes.as_ptr() as usize;
    let hi = lo + bytes.len();
    for i in 0..3 {
        let s = v.slice(i).unwrap();
        let p = s.as_ptr() as usize;
        assert!(p >= lo && p + s.len() * 8 <= hi, "entry {i} points outside the block buffer");
    }
    // entry 0 and entry 2 name the same range, so they are the same memory
    assert_eq!(v.slice(0).unwrap().as_ptr(), v.slice(2).unwrap().as_ptr());
}

#[test]
fn an_entry_past_the_end_of_the_arena_is_refused_before_any_slice_exists() {
    // entry 1 becomes (start 4, len 3): 4 + 3 = 7 > 6
    let buf = patched(32 + 12, &[0x03, 0x00, 0x00, 0x00]);
    let v = ArenaView::parse(buf.as_bytes()).expect("the table is checked per entry, as it is used");
    assert_eq!(v.slice(0).unwrap().len(), 4, "a good entry beside it still reads");
    assert_eq!(v.slice(1), Err(ArenaError::RangeOutOfArena { start: 4, len: 3, arena_len: 6 }));
    assert_eq!(v.entry(1), Err(ArenaError::RangeOutOfArena { start: 4, len: 3, arena_len: 6 }));
}

#[test]
fn an_entry_whose_start_plus_len_overflows_is_refused_not_wrapped() {
    // entry 0 becomes (start 0xffff_fffe, len 4): the sum wraps to 2 in u32 arithmetic
    let mut v = KNOTS.to_vec();
    v[32..36].copy_from_slice(&0xffff_fffe_u32.to_le_bytes());
    let buf = aligned(&v);
    let view = ArenaView::parse(buf.as_bytes()).unwrap();
    let err = view.slice(0).unwrap_err();
    assert_eq!(err, ArenaError::RangeOutOfArena { start: 0xffff_fffe, len: 4, arena_len: 6 });
    assert_eq!(err.rule(), "ARN-11");
}

#[test]
fn the_offset_entry_range_check_is_exact_at_the_boundary() {
    let e = |start, len| OffsetEntry { start, len };
    assert_eq!(e(2, 4).range(6), Some(2..6), "ending exactly at the arena is fine");
    assert_eq!(e(2, 5).range(6), None, "one past is not");
    assert_eq!(e(6, 0).range(6), Some(6..6), "an empty range at the end is inside");
    assert_eq!(e(7, 0).range(6), None);
    assert_eq!(e(u32::MAX, 1).range(u32::MAX), None, "the sum overflows u32");
    assert_eq!(e(u32::MAX, u32::MAX).range(u32::MAX), None);
}

#[test]
fn an_index_past_the_table_and_the_no_entry_marker_are_refused() {
    let buf = aligned(&KNOTS);
    let v = ArenaView::parse(buf.as_bytes()).unwrap();
    assert_eq!(v.slice(3), Err(ArenaError::EntryOutOfRange { index: 3, entries: 3 }));
    assert_eq!(v.slice(NO_ENTRY), Err(ArenaError::EntryOutOfRange { index: NO_ENTRY, entries: 3 }));
    assert_eq!(v.slice(3).unwrap_err().rule(), "ARN-10");
}

#[test]
fn an_accessor_for_another_payload_kind_is_refused() {
    let buf = aligned(&KNOTS);
    let v = ArenaView::parse(buf.as_bytes()).unwrap();
    assert_eq!(v.slice_xy(0), Err(ArenaError::WrongPayload { held: PayloadKind::Knots }));
    assert_eq!(v.slice_u32(0), Err(ArenaError::WrongPayload { held: PayloadKind::Knots }));
}

#[test]
fn header_refusals_name_their_rule() {
    assert_eq!(refused(&patched(0, b"PVAX")), ArenaError::BadMagic);
    assert_eq!(refused(&patched(4, &[2, 0])), ArenaError::UnsupportedVersion { found: 2 });
    assert_eq!(refused(&patched(6, &[9, 0])), ArenaError::UnknownKind { id: 9 });
    assert_eq!(refused(&patched(24, &[1])), ArenaError::NonZeroReserved);
    assert_eq!(refused(&patched(16, &[0x70])), ArenaError::LengthMismatch { declared: 0x70, actual: 104 });
    for (e, rule) in [
        (ArenaError::BadMagic, "ARN-02"),
        (ArenaError::UnsupportedVersion { found: 2 }, "ARN-03"),
        (ArenaError::UnknownKind { id: 9 }, "ARN-04"),
        (ArenaError::NonZeroReserved, "ARN-05"),
        (ArenaError::LengthMismatch { declared: 1, actual: 2 }, "ARN-07"),
        (ArenaError::Truncated { needed: 2, got: 1 }, "ARN-01"),
    ] {
        assert_eq!(e.rule(), rule);
    }
}

#[test]
fn a_lying_entry_count_cannot_size_anything() {
    // 2^28 entries in a 104 byte block: refused as truncated before the table is touched
    let buf = patched(8, &[0x00, 0x00, 0x00, 0x10]);
    assert!(matches!(refused(&buf), ArenaError::Truncated { .. } | ArenaError::LengthMismatch { .. }));
    let buf = patched(12, &[0xff, 0xff, 0xff, 0xff]);
    assert!(matches!(refused(&buf), ArenaError::Truncated { .. } | ArenaError::LengthMismatch { .. }));
}

#[test]
fn a_buffer_that_is_not_8_aligned_is_refused_not_copied() {
    // AlignedBytes starts on 8, so a slice one byte in does not
    let owned = aligned(&[&[0u8][..], &KNOTS[..]].concat());
    let off = &owned.as_bytes()[1..];
    assert_eq!(ArenaView::parse(off).unwrap_err(), ArenaError::Misaligned);
    assert_eq!(ArenaError::Misaligned.rule(), "ARN-08");
}

#[test]
fn every_truncation_of_the_block_is_refused_without_a_panic() {
    assert!(ArenaView::parse(aligned(&KNOTS).as_bytes()).is_ok(), "the whole block must parse first");
    for n in 0..KNOTS.len() {
        let buf = aligned(&KNOTS[..n]);
        assert!(ArenaView::parse(buf.as_bytes()).is_err(), "a {n} byte prefix was accepted");
    }
}

// ---- the writer ---------------------------------------------------------------------------------

/// What the builder must write for the payloads (0, 0, 1, 1) and (0.5, 0.25): the same block as `KNOTS`
/// without its third entry, 96 bytes.
#[rustfmt::skip]
const KNOTS_TWO_ENTRIES: [u8; 96] = [
    0x50, 0x56, 0x41, 0x31, 0x01, 0x00, 0x01, 0x00,
    0x02, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00,
    0x60, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf0, 0x3f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xe0, 0x3f,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xd0, 0x3f,
];

#[test]
fn the_builder_writes_the_hand_written_block_byte_for_byte() {
    let mut b = ArenaBuilder::new(PayloadKind::Knots);
    assert_eq!(b.push_f64(&[0.0, 0.0, 1.0, 1.0]).unwrap(), 0);
    assert_eq!(b.push_f64(&[0.5, 0.25]).unwrap(), 1);
    // an identical payload is not stored again: it is table index 0
    assert_eq!(b.push_f64(&[0.0, 0.0, 1.0, 1.0]).unwrap(), 0);
    let out = b.finish().unwrap();
    assert_eq!(out.as_bytes(), &KNOTS_TWO_ENTRIES[..]);
}

#[test]
fn values_one_ulp_apart_come_back_bit_for_bit() {
    let a = 1.0_f64;
    let b = f64::from_bits(a.to_bits() + 1);
    let mut w = ArenaBuilder::new(PayloadKind::Weights);
    let i = w.push_f64(&[a, b]).unwrap();
    let out = w.finish().unwrap();
    let v = ArenaView::parse(out.as_bytes()).unwrap();
    let s = v.slice(i).unwrap();
    assert_eq!(s[0].to_bits(), a.to_bits());
    assert_eq!(s[1].to_bits(), b.to_bits());
}

#[test]
fn control_points_are_two_equal_columns_and_a_length_mismatch_is_refused() {
    let mut b = ArenaBuilder::new(PayloadKind::ControlPoints);
    let i = b.push_xy(&[0.0, 1.0, 2.0], &[5.0, 6.0, 7.0]).unwrap();
    assert!(b.push_xy(&[0.0, 1.0], &[5.0]).is_err());
    let out = b.finish().unwrap();
    let v = ArenaView::parse(out.as_bytes()).unwrap();
    let (x, y) = v.slice_xy(i).unwrap();
    assert_eq!((x, y), (&[0.0, 1.0, 2.0][..], &[5.0, 6.0, 7.0][..]));
    assert_eq!(v.slice(i), Err(ArenaError::WrongPayload { held: PayloadKind::ControlPoints }));
}

#[test]
fn a_builder_refuses_the_wrong_push_for_its_kind() {
    assert!(ArenaBuilder::new(PayloadKind::Knots).push_f64(&[1.0]).is_ok());
    assert!(ArenaBuilder::new(PayloadKind::Segments).push_u32(&[1]).is_ok());
    assert!(ArenaBuilder::new(PayloadKind::Knots).push_u32(&[1]).is_err());
    assert!(ArenaBuilder::new(PayloadKind::Segments).push_f64(&[1.0]).is_err());
    assert!(ArenaBuilder::new(PayloadKind::Weights).push_xy(&[1.0], &[1.0]).is_err());
}

#[test]
fn every_column_of_every_arena_starts_at_its_natural_alignment() {
    for kind in PayloadKind::ALL {
        let mut b = ArenaBuilder::new(kind);
        // odd and even payload lengths, so the segment arena needs and does not need tail padding
        let idx: Vec<u32> = (0..3u32)
            .map(|n| {
                let len = (n + 1) as usize;
                let f: Vec<f64> = (0..len).map(|k| f64::from(n) + k as f64).collect();
                match kind {
                    PayloadKind::ControlPoints => b.push_xy(&f, &f).unwrap(),
                    PayloadKind::Segments => b.push_u32(&vec![n; len]).unwrap(),
                    _ => b.push_f64(&f).unwrap(),
                }
            })
            .collect();
        let out = b.finish().unwrap();
        assert_eq!(out.len() % 8, 0, "{kind:?}: the block is a multiple of 8 bytes");
        assert_eq!(out.as_bytes().as_ptr() as usize % 8, 0);
        let v = ArenaView::parse(out.as_bytes()).unwrap();
        for &i in &idx {
            match kind {
                PayloadKind::ControlPoints => {
                    let (x, y) = v.slice_xy(i).unwrap();
                    assert_eq!(x.as_ptr() as usize % 8, 0);
                    assert_eq!(y.as_ptr() as usize % 8, 0);
                }
                PayloadKind::Segments => assert_eq!(v.slice_u32(i).unwrap().as_ptr() as usize % 4, 0),
                _ => assert_eq!(v.slice(i).unwrap().as_ptr() as usize % 8, 0),
            }
        }
    }
}

#[test]
fn non_zero_padding_after_an_odd_segment_arena_is_refused() {
    let mut b = ArenaBuilder::new(PayloadKind::Segments);
    b.push_u32(&[1, 2, 3]).unwrap();
    let out = b.finish().unwrap();
    let mut bytes = out.as_bytes().to_vec();
    assert_eq!(bytes.len(), 32 + 8 + 16, "three u32 are 12 bytes, padded to 16");
    let last = bytes.len() - 1;
    bytes[last] = 1;
    assert_eq!(refused(&aligned(&bytes)), ArenaError::NonZeroPadding);
    assert_eq!(ArenaError::NonZeroPadding.rule(), "ARN-06");
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

#[test]
fn one_slice_is_one_table_read_and_one_payload_read() {
    let buf = aligned(&KNOTS);
    let probe = Counter::default();
    let v = ArenaView::parse_probed(buf.as_bytes(), Some(&probe)).unwrap();
    assert_eq!(probe.tables[0].get() + probe.payloads[0].get(), 0, "parsing reads no entry and no payload");
    v.slice(0).unwrap();
    assert_eq!((probe.tables[0].get(), probe.payloads[0].get()), (1, 1));
    v.slice(1).unwrap();
    assert_eq!((probe.tables[0].get(), probe.payloads[0].get()), (2, 2));
}

#[test]
fn the_cost_of_a_slice_does_not_depend_on_the_size_of_the_arena() {
    let mut costs = Vec::new();
    for n in [1usize, 100, 10_000] {
        let mut b = ArenaBuilder::new(PayloadKind::Knots);
        let mut last = 0;
        for i in 0..n {
            last = b.push_f64(&[i as f64, i as f64 + 1.0, i as f64 + 2.0]).unwrap();
        }
        let out = b.finish().unwrap();
        let probe = Counter::default();
        let v = ArenaView::parse_probed(out.as_bytes(), Some(&probe)).unwrap();
        v.slice(last).unwrap();
        costs.push((probe.tables[0].get(), probe.payloads[0].get()));
    }
    assert!(costs.iter().all(|c| *c == (1, 1)), "{costs:?}");
}

#[test]
fn a_refused_entry_reads_no_payload() {
    let buf = patched(32 + 12, &[0x03, 0x00, 0x00, 0x00]);
    let probe = Counter::default();
    let v = ArenaView::parse_probed(buf.as_bytes(), Some(&probe)).unwrap();
    assert!(v.slice(1).is_err());
    assert_eq!(probe.payloads[0].get(), 0, "the range was checked before the arena was touched");
}

// ---- hostile input ------------------------------------------------------------------------------

#[test]
fn overwriting_any_word_with_zero_or_all_ones_never_panics() {
    for word in [0u32, u32::MAX, 0x8000_0000, 7] {
        for at in (0..KNOTS.len()).step_by(4) {
            let mut v = KNOTS.to_vec();
            v[at..at + 4].copy_from_slice(&word.to_le_bytes());
            let buf = aligned(&v);
            if let Ok(view) = ArenaView::parse(buf.as_bytes()) {
                for i in 0..view.entries() as u32 + 2 {
                    let _ = view.slice(i);
                    let _ = view.slice_xy(i);
                    let _ = view.slice_u32(i);
                    let _ = view.entry(i);
                }
            }
        }
    }
}
