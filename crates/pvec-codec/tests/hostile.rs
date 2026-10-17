//! Hostile and corrupt input to the decoder (P3.2): every refusal is typed and names its rule, no input
//! panics, and no count in a header can size an allocation before it has been checked against the
//! bytes that are actually there.
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_codec::*;

// Header layout of docs/GEOMETRY_ENCODING.md, as byte offsets.
const MAGIC: usize = 0;
const VERSION: usize = 4;
const FLAGS: usize = 6;
const TOTAL_LEN: usize = 8;
const ENTITY_COUNT: usize = 12;
const KIND_COUNT: usize = 16;
const RESERVED: usize = 18;
const DIRECTORY: usize = 24;

fn put_u16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn sample_bytes() -> Vec<u8> {
    encode(&entities(&basic_samples())).unwrap().bytes
}

fn single(name: &str) -> Vec<u8> {
    let samples = basic_samples();
    let c = samples.iter().find(|(n, _)| *n == name).unwrap().1.clone();
    encode(&[(id(0), c)]).unwrap().bytes
}

fn refused(bytes: &[u8]) -> DecodeError {
    decode(bytes).expect_err("this input must be refused")
}

#[test]
fn a_valid_block_decodes_and_is_the_baseline_for_every_mutation_below() {
    let b = sample_bytes();
    assert_eq!(decode(&b).unwrap().len(), basic_samples().len());
    assert_eq!(&b[MAGIC..MAGIC + 4], b"PVG1");
    assert_eq!(u16::from_le_bytes([b[VERSION], b[VERSION + 1]]), FORMAT_VERSION);
    assert_eq!(get_u32(&b, TOTAL_LEN) as usize, b.len());
    assert_eq!(get_u32(&b, ENTITY_COUNT) as usize, basic_samples().len());
}

#[test]
fn header_fields_are_checked() {
    let good = sample_bytes();
    let mut b = good.clone();
    b[MAGIC] ^= 0xff;
    assert_eq!(refused(&b), DecodeError::BadMagic);

    let mut b = good.clone();
    put_u16(&mut b, VERSION, FORMAT_VERSION + 1);
    assert_eq!(refused(&b), DecodeError::UnsupportedVersion(FORMAT_VERSION + 1));

    let mut b = good.clone();
    put_u16(&mut b, FLAGS, 1);
    assert_eq!(refused(&b), DecodeError::ReservedNotZero);

    let mut b = good.clone();
    b[RESERVED] = 1;
    assert_eq!(refused(&b), DecodeError::ReservedNotZero);

    let mut b = good.clone();
    put_u32(&mut b, TOTAL_LEN, good.len() as u32 + 8);
    assert!(matches!(refused(&b), DecodeError::LengthMismatch { .. }));

    let mut b = good.clone();
    b.extend_from_slice(&[0; 8]);
    assert!(matches!(refused(&b), DecodeError::LengthMismatch { .. }), "trailing bytes are refused");
}

#[test]
fn a_lying_entity_count_is_refused_before_anything_is_allocated_from_it() {
    let good = sample_bytes();
    for lie in [u32::MAX, 1 << 28, (1 << 28) - 1, good.len() as u32] {
        let mut b = good.clone();
        put_u32(&mut b, ENTITY_COUNT, lie);
        let e = refused(&b);
        assert!(
            matches!(e, DecodeError::CountMismatch { .. } | DecodeError::TooMany | DecodeError::Truncated),
            "{lie}: {e:?}"
        );
    }
    // Per-kind counts too: the first directory entry's count is at DIRECTORY + 4.
    for lie in [u32::MAX, 1 << 28, 1 << 27] {
        let mut b = good.clone();
        put_u32(&mut b, DIRECTORY + 4, lie);
        let e = refused(&b);
        assert!(
            matches!(e, DecodeError::TooMany | DecodeError::CountMismatch { .. } | DecodeError::Truncated),
            "{lie}: {e:?}"
        );
    }
}

#[test]
fn a_header_claiming_billions_of_entities_in_a_tiny_block_fails_fast() {
    // 32 bytes of directory, 2^28 - 1 lines claimed: the column sizes alone exceed the block.
    let mut b = vec![0u8; DIRECTORY + 8];
    b[MAGIC..MAGIC + 4].copy_from_slice(b"PVG1");
    put_u16(&mut b, VERSION, FORMAT_VERSION);
    let total = b.len() as u32;
    put_u32(&mut b, TOTAL_LEN, total);
    put_u32(&mut b, ENTITY_COUNT, (1 << 28) - 1);
    put_u16(&mut b, KIND_COUNT, 1);
    put_u16(&mut b, DIRECTORY, 0); // Line
    put_u32(&mut b, DIRECTORY + 4, (1 << 28) - 1);
    assert_eq!(refused(&b), DecodeError::Truncated);
    // and past the limit it is the limit that is named
    put_u32(&mut b, ENTITY_COUNT, 1 << 28);
    put_u32(&mut b, DIRECTORY + 4, 1 << 28);
    assert_eq!(refused(&b), DecodeError::TooMany);
}

#[test]
fn the_kind_directory_must_be_ascending_unique_known_and_non_empty() {
    let good = sample_bytes();
    let kinds = u16::from_le_bytes([good[KIND_COUNT], good[KIND_COUNT + 1]]) as usize;
    assert!(kinds >= 4);
    // swap the first two directory entries: no longer ascending
    let mut b = good.clone();
    let (a, c) = (DIRECTORY, DIRECTORY + 8);
    let tmp: Vec<u8> = b[a..a + 8].to_vec();
    b.copy_within(c..c + 8, a);
    b[c..c + 8].copy_from_slice(&tmp);
    assert_eq!(refused(&b), DecodeError::BadKindDirectory);

    // a duplicate
    let mut b = good.clone();
    b.copy_within(a..a + 2, c);
    assert_eq!(refused(&b), DecodeError::BadKindDirectory);

    // an unknown kind id
    let mut b = good.clone();
    put_u16(&mut b, a, 999);
    assert_eq!(refused(&b), DecodeError::UnknownKind(999));

    // a zero count
    let mut b = good.clone();
    put_u32(&mut b, a + 4, 0);
    assert_eq!(refused(&b), DecodeError::BadKindDirectory);

    // more kinds than exist
    let mut b = good.clone();
    put_u16(&mut b, KIND_COUNT, 5000);
    assert!(matches!(
        refused(&b),
        DecodeError::Truncated | DecodeError::BadKindDirectory | DecodeError::TooMany
    ));
}

#[test]
fn ids_must_be_below_the_limit_and_unique_within_the_block() {
    // One line: ids column starts right after the directory.
    let b0 = single("line");
    let ids_at = DIRECTORY + 8;
    assert_eq!(get_u32(&b0, ids_at), 0);
    let mut b = b0.clone();
    put_u32(&mut b, ids_at, 1 << 28);
    assert_eq!(refused(&b), DecodeError::BadId(1 << 28));

    let two = encode(&[(id(0), basic_samples()[0].1.clone()), (id(1), basic_samples()[0].1.clone())])
        .unwrap()
        .bytes;
    let mut b = two.clone();
    put_u32(&mut b, ids_at + 4, 0);
    assert_eq!(refused(&b), DecodeError::DuplicateId(id(0)));
}

#[test]
fn flag_bits_that_are_not_defined_are_refused() {
    // A segment: ... flags column is the last column, so it is the last 8 bytes of the block.
    let mut b = single("segment");
    let n = b.len();
    b[n - 8] |= 0b10;
    assert_eq!(refused(&b), DecodeError::BadFlags);
}

#[test]
fn polyline_offsets_must_start_at_zero_grow_and_describe_at_least_two_vertices() {
    let b0 = single("polyline-2");
    // ids (4 bytes, padded to 8), then the offset table (2 entries, padded), then xs, ys.
    let voff = DIRECTORY + 8 + 8;
    assert_eq!((get_u32(&b0, voff), get_u32(&b0, voff + 4)), (0, 2));
    let mut b = b0.clone();
    put_u32(&mut b, voff, 1);
    assert_eq!(refused(&b), DecodeError::BadOffsets);
    let mut b = b0.clone();
    put_u32(&mut b, voff + 4, 1); // one vertex is not a polyline
    assert!(matches!(refused(&b), DecodeError::BadOffsets | DecodeError::LengthMismatch { .. }));
    let mut b = b0.clone();
    put_u32(&mut b, voff + 4, u32::MAX); // a vertex total far beyond the block
    assert!(matches!(refused(&b), DecodeError::BadOffsets | DecodeError::Truncated | DecodeError::TooMany));

    // non-monotone with two polylines
    let samples = basic_samples();
    let p = samples.iter().find(|(n, _)| *n == "polyline-2").unwrap().1.clone();
    let b0 = encode(&[(id(0), p.clone()), (id(1), p)]).unwrap().bytes;
    let voff = DIRECTORY + 8 + 8; // two ids = 8 bytes, no padding needed
    assert_eq!((get_u32(&b0, voff), get_u32(&b0, voff + 4), get_u32(&b0, voff + 8)), (0, 2, 4));
    let mut b = b0.clone();
    put_u32(&mut b, voff + 4, 5);
    assert_eq!(refused(&b), DecodeError::BadOffsets);
}

#[test]
fn composite_records_with_unknown_tags_or_bad_lengths_are_refused() {
    let b0 = single("composite");
    // One composite: ids (4 bytes, padded to 8), flags (1, padded to 8), the offset table (2 entries) and
    // then the arena, so the table is at +16 after the directory and the arena right behind it.
    let table = DIRECTORY + 8 + 16;
    let len = get_u32(&b0, table + 4) as usize;
    assert_eq!(get_u32(&b0, table), 0);
    assert_eq!(table + 8 + len, b0.len(), "the arena runs to the end of the block");
    let arena = table + 8;
    assert_eq!(get_u32(&b0, arena), 3, "three segments");
    // first segment tag sits after the segment count (4) and padding (4)
    let tag = arena + 8;
    let mut b = b0.clone();
    put_u32(&mut b, tag, 77);
    assert_eq!(refused(&b), DecodeError::BadTag(77));
    let mut b = b0.clone();
    put_u32(&mut b, arena, 0);
    assert!(matches!(refused(&b), DecodeError::BadRecord | DecodeError::Geometry { .. }));
    let mut b = b0.clone();
    put_u32(&mut b, arena, u32::MAX);
    assert!(matches!(refused(&b), DecodeError::BadRecord | DecodeError::TooMany));
    // the record length in the offset table must match what the record consumes
    let mut b = b0.clone();
    put_u32(&mut b, table + 4, len as u32 - 8);
    assert!(matches!(
        refused(&b),
        DecodeError::LengthMismatch { .. } | DecodeError::BadRecord | DecodeError::Truncated
    ));
}

#[test]
fn malformed_geometry_is_refused_by_the_geometry_constructors_with_their_rule() {
    // A circle whose radius is zero, a direction that is not unit, a NaN centre.
    let b0 = single("circle");
    // columns after ids (4 bytes + 4 pad): cx, cy, ux, uy, r
    let col = |k: usize| DIRECTORY + 8 + 8 + 8 * k;
    let mut b = b0.clone();
    b[col(4)..col(4) + 8].copy_from_slice(&0.0f64.to_le_bytes());
    match refused(&b) {
        DecodeError::Geometry { id: i, error } => {
            assert_eq!(i, id(0));
            assert_eq!(error.rule(), "NP-07");
        }
        other => panic!("{other:?}"),
    }
    let mut b = b0.clone();
    b[col(2)..col(2) + 8].copy_from_slice(&2.0f64.to_le_bytes());
    match refused(&b) {
        DecodeError::Geometry { error, .. } => assert_eq!(error.rule(), "NP-10"),
        other => panic!("{other:?}"),
    }
    let mut b = b0.clone();
    b[col(0)..col(0) + 8].copy_from_slice(&f64::NAN.to_le_bytes());
    match refused(&b) {
        DecodeError::Geometry { error, .. } => assert_eq!(error.rule(), "NP-01"),
        other => panic!("{other:?}"),
    }
    let mut b = b0.clone();
    b[col(1)..col(1) + 8].copy_from_slice(&f64::INFINITY.to_le_bytes());
    assert!(matches!(refused(&b), DecodeError::Geometry { .. }));
}

#[test]
fn every_truncation_of_a_valid_block_is_refused_and_never_panics() {
    let b = sample_bytes();
    for n in 0..b.len() {
        assert!(decode(&b[..n]).is_err(), "a block cut at {n} of {} must not decode", b.len());
    }
}

#[test]
fn random_corruption_never_panics() {
    let good = sample_bytes();
    let mut r = Rng(0xC0DEC);
    let mut ok = 0;
    for _ in 0..4000 {
        let mut b = good.clone();
        for _ in 0..1 + r.below(4) {
            let at = r.below(b.len());
            b[at] = r.next_u64() as u8;
        }
        if let Ok(v) = decode(&b) {
            ok += 1;
            // anything that decodes is valid geometry and encodes again
            let re: Vec<_> = v.iter().map(|(i, c)| (*i, c.clone())).collect();
            assert!(encode(&re).is_ok());
        }
    }
    println!(
        "{ok} of 4000 mutated blocks still decoded (mutations inside a float column are still valid floats)"
    );
}

#[test]
fn random_bytes_and_random_headers_never_panic() {
    let mut r = Rng(0xBAD);
    for _ in 0..2000 {
        let n = r.below(400);
        let mut b: Vec<u8> = (0..n).map(|_| r.next_u64() as u8).collect();
        let _ = decode(&b);
        // with a plausible header in front
        if b.len() >= DIRECTORY {
            b[MAGIC..MAGIC + 4].copy_from_slice(b"PVG1");
            put_u16(&mut b, VERSION, FORMAT_VERSION);
            put_u16(&mut b, FLAGS, 0);
            let total = b.len() as u32;
            put_u32(&mut b, TOTAL_LEN, total);
            b[RESERVED..DIRECTORY].fill(0);
            let _ = decode(&b);
        }
    }
}

#[test]
fn errors_name_their_rule() {
    let e = refused(&[]);
    assert!(!e.rule().is_empty());
    let e = EncodeError::DuplicateId(id(1));
    assert!(!e.rule().is_empty() && e.to_string().contains("#1"));
}

#[test]
fn padding_must_be_zero_so_the_encoding_stays_canonical() {
    // One line: the id column is 4 bytes followed by 4 bytes of padding.
    let mut b = single("line");
    let pad = DIRECTORY + 8 + 4;
    assert_eq!(&b[pad..pad + 4], &[0, 0, 0, 0]);
    b[pad + 1] = 7;
    assert_eq!(refused(&b), DecodeError::NonZeroPadding);
    // reserved directory bytes too
    let mut b = single("line");
    b[DIRECTORY + 2] = 1;
    assert_eq!(refused(&b), DecodeError::ReservedNotZero);
}

#[test]
fn bytes_the_structure_does_not_use_are_refused_even_when_the_header_length_covers_them() {
    let mut b = single("circle");
    b.extend_from_slice(&[0; 8]);
    let n = b.len() as u32;
    put_u32(&mut b, TOTAL_LEN, n);
    assert!(matches!(refused(&b), DecodeError::LengthMismatch { .. }));
}

#[test]
fn a_composite_record_must_be_consumed_exactly_and_its_segment_flags_checked() {
    let b0 = single("composite");
    let arena = DIRECTORY + 8 + 16 + 8;
    // fewer segments than the record holds: the rest of the record is unexplained
    let mut b = b0.clone();
    put_u32(&mut b, arena, 2);
    assert_eq!(refused(&b), DecodeError::BadRecord);
    // the reversed word of the first segment (tag at arena + 8, flag word right after) is 0 or 1
    let mut b = b0.clone();
    put_u32(&mut b, arena + 12, 2);
    assert_eq!(refused(&b), DecodeError::BadFlags);
}
