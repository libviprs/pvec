//! The fixed header (issue P4.1): field offsets, magic, versions, pointer validation.

mod common;

use common::*;
use pvec_container::*;

fn parse(bytes: &[u8]) -> Result<Header, ContainerError> {
    Header::parse(bytes, ARTIFACT_LEN)
}

#[test]
fn the_fixture_header_parses_to_the_expected_fields() {
    assert_eq!(parse(&HEADER_BYTES).unwrap(), expected_header());
}

#[test]
fn the_header_fits_one_range_read_of_at_most_4_kib() {
    assert_eq!(Header::LEN, 64);
    assert_eq!(Header::MAX_LEN, 4096);
    assert_eq!(HEADER_BYTES.len(), Header::LEN);
    // The largest legal header, with the manifest right behind it.
    let mut b = padded_header(4096);
    b[16..18].copy_from_slice(&4096u16.to_le_bytes());
    b[24..32].copy_from_slice(&4096u64.to_le_bytes());
    b[40..48].copy_from_slice(&4320u64.to_le_bytes());
    assert_eq!(parse(&b).unwrap().header_len, 4096);
}

#[test]
fn every_header_field_is_read_from_its_offset() {
    // (offset, bytes to write, check). Offsets are literals here on purpose: they come from the
    // byte-literal fixture, not from the constants the parser uses.
    let b = padded_header(128);
    let minor = parse(&with(&b, 10, &7u16.to_le_bytes())).unwrap();
    assert_eq!(minor.minor, 7);
    let hl = parse(&with(&with(&b, 16, &72u16.to_le_bytes()), 24, &72u64.to_le_bytes())).unwrap();
    assert_eq!((hl.header_len, hl.manifest_offset), (72, 72));
    let ck = parse(&with(&with(&b, 20, &[2]), 21, &[1])).unwrap();
    assert_eq!(
        (ck.block_checksum, ck.artifact_checksum),
        (ChecksumAlgorithm::Sha256, ChecksumAlgorithm::Xxh3_64)
    );
    let mo = parse(&with(&b, 24, &100u64.to_le_bytes())).unwrap();
    assert_eq!(mo.manifest_offset, 100);
    let ml = parse(&with(&b, 32, &300u64.to_le_bytes())).unwrap();
    assert_eq!(ml.manifest_length, 300);
    let dto = parse(&with(&b, 40, &512u64.to_le_bytes())).unwrap();
    assert_eq!(dto.directory_table_offset, 512);
    let dtl = parse(&with(&b, 48, &96u64.to_le_bytes())).unwrap();
    assert_eq!(dtl.directory_table_length, 96);
    let major = parse(&with(&b, 8, &1u16.to_le_bytes())).unwrap();
    assert_eq!(major.major, 1);
}

#[test]
fn the_encoding_of_the_expected_header_is_the_fixture_byte_for_byte() {
    assert_eq!(expected_header().encode(), HEADER_BYTES);
}

#[test]
fn a_header_round_trips() {
    let h = expected_header();
    assert_eq!(parse(&h.encode()).unwrap(), h);
}

#[test]
fn a_magic_that_differs_in_one_byte_is_refused() {
    for i in 0..8 {
        let mut b = HEADER_BYTES;
        b[i] ^= 0x01;
        assert_eq!(parse(&b), Err(ContainerError::BadMagic), "byte {i}");
    }
}

#[test]
fn a_bad_magic_is_refused_from_the_first_eight_bytes_alone() {
    // Only eight bytes are supplied. If the parser needed more it would say Truncated; it says
    // BadMagic, so it decided before reading past the magic.
    let mut b = HEADER_BYTES;
    b[7] ^= 0xff;
    assert_eq!(parse(&b[..8]), Err(ContainerError::BadMagic));
}

#[test]
fn a_bad_magic_wins_over_every_later_defect() {
    let mut b = vec![0xaa; 64];
    b[0] = 0;
    assert_eq!(parse(&b), Err(ContainerError::BadMagic));
}

#[test]
fn a_truncated_prefix_is_refused_at_every_length() {
    for n in 0..Header::LEN {
        match parse(&HEADER_BYTES[..n]) {
            Err(ContainerError::Truncated { .. }) => {}
            other => panic!("{n} bytes: {other:?}"),
        }
    }
}

#[test]
fn a_prefix_shorter_than_the_declared_header_length_is_truncated() {
    let mut b = padded_header(80);
    b[16..18].copy_from_slice(&80u16.to_le_bytes());
    b[24..32].copy_from_slice(&80u64.to_le_bytes());
    assert!(parse(&b).is_ok());
    assert!(matches!(parse(&b[..79]), Err(ContainerError::Truncated { .. })));
}

#[test]
fn a_major_version_above_the_readers_is_unsupported() {
    for major in [2u16, 3, 255, u16::MAX] {
        assert_eq!(
            parse(&with(&HEADER_BYTES, 8, &major.to_le_bytes())),
            Err(ContainerError::UnsupportedVersion { found: major, supported: 1 }),
        );
    }
}

#[test]
fn major_version_zero_is_unsupported_too() {
    assert_eq!(
        parse(&with(&HEADER_BYTES, 8, &0u16.to_le_bytes())),
        Err(ContainerError::UnsupportedVersion { found: 0, supported: 1 }),
    );
}

#[test]
fn a_higher_minor_version_parses() {
    for minor in [1u16, 2, 1000, u16::MAX] {
        let h = parse(&with(&HEADER_BYTES, 10, &minor.to_le_bytes())).unwrap();
        assert_eq!(h.minor, minor);
        assert_eq!(h.major, 1);
    }
}

#[test]
fn a_grown_header_is_skipped_by_its_own_length() {
    let mut b = padded_header(256);
    b[10..12].copy_from_slice(&1u16.to_le_bytes());
    b[16..18].copy_from_slice(&200u16.to_le_bytes());
    b[24..32].copy_from_slice(&200u64.to_le_bytes());
    b[100] = 0x5a; // a field a later minor version adds
    let h = parse(&b).unwrap();
    assert_eq!((h.minor, h.header_len, h.manifest_offset), (1, 200, 200));
}

#[test]
fn the_header_length_is_bounded_below_by_the_fixed_header_and_above_by_4_kib() {
    for bad in [0u16, 1, 63, 4097, u16::MAX] {
        let b = with(&padded_header(4096), 16, &bad.to_le_bytes());
        assert_eq!(parse(&b), Err(ContainerError::BadHeaderLength { found: bad as u32 }), "{bad}");
    }
}

#[test]
fn a_big_endian_marker_is_refused() {
    let b = with(&HEADER_BYTES, 12, &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(parse(&b), Err(ContainerError::UnsupportedByteOrder { found: 0x0403_0201 }));
    let b = with(&HEADER_BYTES, 12, &[0, 0, 0, 0]);
    assert!(matches!(parse(&b), Err(ContainerError::UnsupportedByteOrder { .. })));
}

#[test]
fn an_unknown_checksum_algorithm_is_refused() {
    for id in [0u8, 3, 9, 255] {
        assert_eq!(
            parse(&with(&HEADER_BYTES, 20, &[id])),
            Err(ContainerError::UnknownChecksumAlgorithm { id })
        );
        assert_eq!(
            parse(&with(&HEADER_BYTES, 21, &[id])),
            Err(ContainerError::UnknownChecksumAlgorithm { id })
        );
    }
}

#[test]
fn reserved_header_bytes_must_be_zero() {
    for (offset, field) in [(18usize, "reserved"), (22, "reserved"), (56, "reserved")] {
        let r = parse(&with(&HEADER_BYTES, offset, &[1]));
        assert!(matches!(r, Err(ContainerError::NonZeroReserved { .. })), "{field} at {offset}: {r:?}");
    }
}

#[test]
fn a_manifest_pointer_past_the_end_of_the_file_is_refused_not_clamped() {
    for (offset, length) in [
        (ARTIFACT_LEN, 1u64),
        (64, ARTIFACT_LEN),
        (ARTIFACT_LEN - 100, 101),
        (u64::MAX, 1),
        (64, u64::MAX),
        (u64::MAX, u64::MAX),
    ] {
        let b = with(&with(&HEADER_BYTES, 24, &offset.to_le_bytes()), 32, &length.to_le_bytes());
        match parse(&b) {
            Err(ContainerError::PointerOutOfRange { field, .. }) => assert_eq!(field, "manifest"),
            other => panic!("manifest {offset}+{length}: {other:?}"),
        }
    }
}

#[test]
fn a_manifest_that_ends_exactly_at_the_end_of_the_file_is_accepted() {
    let b = with(&with(&HEADER_BYTES, 24, &(ARTIFACT_LEN - 224).to_le_bytes()), 32, &224u64.to_le_bytes());
    assert!(parse(&b).is_ok());
}

#[test]
fn a_directory_table_pointer_past_the_end_of_the_file_is_refused_not_clamped() {
    for (offset, length) in
        [(ARTIFACT_LEN, 1u64), (224, ARTIFACT_LEN), (ARTIFACT_LEN - 47, 48), (u64::MAX, 2), (224, u64::MAX)]
    {
        let b = with(&with(&HEADER_BYTES, 40, &offset.to_le_bytes()), 48, &length.to_le_bytes());
        match parse(&b) {
            Err(ContainerError::PointerOutOfRange { field, .. }) => assert_eq!(field, "directory_table"),
            other => panic!("table {offset}+{length}: {other:?}"),
        }
    }
}

#[test]
fn a_pointer_into_the_header_itself_is_refused() {
    let b = with(&HEADER_BYTES, 24, &32u64.to_le_bytes());
    assert!(matches!(parse(&b), Err(ContainerError::PointerOutOfRange { field: "manifest", .. })));
    let b = with(&HEADER_BYTES, 40, &8u64.to_le_bytes());
    assert!(matches!(parse(&b), Err(ContainerError::PointerOutOfRange { field: "directory_table", .. })));
}

#[test]
fn an_empty_manifest_is_refused() {
    let b = with(&HEADER_BYTES, 32, &0u64.to_le_bytes());
    assert!(matches!(parse(&b), Err(ContainerError::PointerOutOfRange { field: "manifest", .. })));
}

#[test]
fn a_manifest_above_the_prefix_bound_is_refused_even_inside_a_huge_file() {
    let len = (1u64 << 20) + 1;
    let b = with(&HEADER_BYTES, 32, &len.to_le_bytes());
    assert_eq!(Header::parse(&b, 1 << 40), Err(ContainerError::ManifestTooLarge { length: len }));
}

#[test]
fn a_directory_table_may_be_empty() {
    let b = with(&HEADER_BYTES, 48, &0u64.to_le_bytes());
    assert_eq!(parse(&b).unwrap().directory_table_length, 0);
}

#[test]
fn no_input_makes_the_header_parser_panic() {
    for i in 0..Header::LEN {
        for v in [0u8, 1, 0x7f, 0x80, 0xff] {
            let _ = parse(&with(&HEADER_BYTES, i, &[v]));
        }
    }
    for n in 0..70 {
        let _ = parse(&vec![0xff; n]);
        let _ = parse(&vec![0u8; n]);
    }
}
