//! The artifact manifest (issue P4.1).

mod common;

use common::*;
use pvec_capabilities::{CapabilityId, Declaration, Requirement, Version};
use pvec_container::*;
use pvec_numeric::NumericError;

const FIXED: usize = 128;

fn m(bytes: &[u8]) -> Result<Manifest, ContainerError> {
    parse_manifest(bytes)
}

fn put_f64(offset: usize, v: f64) -> Vec<u8> {
    with(&MANIFEST_BYTES, offset, &v.to_le_bytes())
}

#[test]
fn the_fixture_manifest_parses_to_the_expected_fields() {
    assert_eq!(m(&MANIFEST_BYTES).unwrap(), expected_manifest());
}

#[test]
fn the_encoding_of_the_expected_manifest_is_the_fixture_byte_for_byte() {
    assert_eq!(expected_manifest().encode(), MANIFEST_BYTES.to_vec());
}

#[test]
fn a_manifest_round_trips() {
    let man = expected_manifest();
    assert_eq!(m(&man.encode()).unwrap(), man);
}

#[test]
fn every_fixed_manifest_field_is_read_from_its_offset() {
    let b = MANIFEST_BYTES.to_vec();
    let flag_off = m(&with(&b, 2, &0u16.to_le_bytes()));
    // Clearing the digest flag while the digest bytes are set is refused: the flag is at offset 2.
    assert!(matches!(flag_off, Err(ContainerError::NonZeroReserved { .. })));
    let cleared = with(&with(&b, 2, &0u16.to_le_bytes()), 80, &[0; 32]);
    assert_eq!(m(&cleared).unwrap().artifact_digest, None);
    // unit (offset 0) with its own factor (offset 8)
    let cm = with(&with(&b, 0, &3u16.to_le_bytes()), 8, &0.01f64.to_le_bytes());
    assert_eq!(m(&cm).unwrap().unit, LengthUnit::Centimetre);
    // bounds at 16, 24, 32, 40
    assert_eq!(m(&put_f64(16, -7.0)).unwrap().bounds.min[0], -7.0);
    assert_eq!(m(&put_f64(24, -8.0)).unwrap().bounds.min[1], -8.0);
    assert_eq!(m(&put_f64(32, 9.0)).unwrap().bounds.max[0], 9.0);
    assert_eq!(m(&put_f64(40, 10.0)).unwrap().bounds.max[1], 10.0);
    // min_reader at 48, 50, 52
    assert_eq!(m(&with(&b, 48, &1u16.to_le_bytes())).unwrap().min_reader, Version::new(1, 0, 0));
    assert_eq!(m(&with(&b, 50, &4u16.to_le_bytes())).unwrap().min_reader, Version::new(1, 4, 0));
    assert_eq!(m(&with(&b, 52, &9u16.to_le_bytes())).unwrap().min_reader, Version::new(1, 0, 9));
    // block checksum table at 64 and 72
    let bc = m(&with(&with(&b, 64, &1024u64.to_le_bytes()), 72, &64u64.to_le_bytes())).unwrap();
    assert_eq!(bc.block_checksums, Some(ByteRange { offset: 1024, length: 64 }));
    // digest at 80
    let d = m(&with(&b, 80, &[0xee])).unwrap().artifact_digest.unwrap();
    assert_eq!(d[0], 0xee);
    assert_eq!(d[1], 1);
}

#[test]
fn the_block_checksum_table_is_absent_when_both_fields_are_zero() {
    let b = with(&with(&MANIFEST_BYTES, 64, &[0; 8]), 72, &[0; 8]);
    assert_eq!(m(&b).unwrap().block_checksums, None);
}

#[test]
fn an_unset_digest_flag_means_no_digest() {
    let b = with(&with(&MANIFEST_BYTES, 2, &0u16.to_le_bytes()), 80, &[0; 32]);
    assert_eq!(m(&b).unwrap().artifact_digest, None);
}

#[test]
fn undefined_flag_bits_and_reserved_fields_must_be_zero() {
    for bit in 1..16 {
        let flags = 1u16 | (1 << bit);
        assert_eq!(
            m(&with(&MANIFEST_BYTES, 2, &flags.to_le_bytes())),
            Err(ContainerError::InvalidFlags { bits: flags }),
            "bit {bit}"
        );
    }
    for offset in [54usize, 60, 124] {
        let r = m(&with(&MANIFEST_BYTES, offset, &[1]));
        assert!(matches!(r, Err(ContainerError::NonZeroReserved { .. })), "offset {offset}: {r:?}");
    }
    // reserved bytes inside a capability entry (11 and 12 of 16) and inside both directory entries
    for offset in [FIXED + 11, FIXED + 12, FIXED + 32 + 4, FIXED + 32 + 24 + 4] {
        let r = m(&with(&MANIFEST_BYTES, offset, &[1]));
        assert!(matches!(r, Err(ContainerError::NonZeroReserved { .. })), "offset {offset}: {r:?}");
    }
}

#[test]
fn every_unit_has_an_exact_factor_and_an_iso_declaration() {
    let expected = [
        (LengthUnit::Metre, 1u16, 1.0, "SI_UNIT($,.METRE.)"),
        (LengthUnit::Millimetre, 2, 0.001, "SI_UNIT(.MILLI.,.METRE.)"),
        (LengthUnit::Centimetre, 3, 0.01, "SI_UNIT(.CENTI.,.METRE.)"),
        (LengthUnit::Kilometre, 4, 1000.0, "SI_UNIT(.KILO.,.METRE.)"),
        (LengthUnit::Inch, 5, 0.0254, "CONVERSION_BASED_UNIT('INCH',0.0254)"),
        (LengthUnit::Foot, 6, 0.3048, "CONVERSION_BASED_UNIT('FOOT',0.3048)"),
    ];
    assert_eq!(LengthUnit::ALL.len(), expected.len());
    for (unit, code, factor, iso) in expected {
        assert_eq!(unit.code(), code);
        assert_eq!(LengthUnit::from_code(code), Some(unit));
        assert_eq!(unit.metres_per_unit(), factor);
        assert_eq!(unit.iso_declaration(), iso);
        let b = with(&with(&MANIFEST_BYTES, 0, &code.to_le_bytes()), 8, &factor.to_le_bytes());
        assert_eq!(m(&b).unwrap().unit, unit);
    }
}

#[test]
fn an_unknown_unit_code_is_refused() {
    for code in [0u16, 7, 100, u16::MAX] {
        assert_eq!(
            m(&with(&MANIFEST_BYTES, 0, &code.to_le_bytes())),
            Err(ContainerError::UnknownLengthUnit { code })
        );
        assert_eq!(LengthUnit::from_code(code), None);
    }
}

#[test]
fn a_factor_that_is_not_the_units_own_is_refused() {
    for f in [0.002f64, 1.0, 0.0, -0.001, f64::NAN, f64::from_bits(0.001f64.to_bits() + 1)] {
        assert_eq!(m(&put_f64(8, f)), Err(ContainerError::UnitFactorMismatch), "{f}");
    }
}

#[test]
fn a_nan_bound_fails_parsing_on_every_field() {
    for (offset, field) in [(16usize, "min_x"), (24, "min_y"), (32, "max_x"), (40, "max_y")] {
        match m(&put_f64(offset, f64::NAN)) {
            Err(ContainerError::Bound { field: f, rule }) => {
                assert_eq!(f, field);
                assert_eq!(rule, NumericError::NotFinite);
            }
            other => panic!("{field}: {other:?}"),
        }
    }
}

#[test]
fn an_infinite_subnormal_or_out_of_range_bound_is_refused() {
    for (v, rule) in [
        (f64::INFINITY, NumericError::NotFinite),
        (f64::NEG_INFINITY, NumericError::NotFinite),
        (f64::MIN_POSITIVE / 2.0, NumericError::Subnormal),
        (1_100_000_000_000.0, NumericError::OutOfRange),
        (-1_100_000_000_000.0, NumericError::OutOfRange),
    ] {
        for offset in [16usize, 24, 32, 40] {
            match m(&put_f64(offset, v)) {
                Err(ContainerError::Bound { rule: r, .. }) => assert_eq!(r, rule, "{v} at {offset}"),
                other => panic!("{v} at {offset}: {other:?}"),
            }
        }
    }
}

#[test]
fn min_above_max_is_refused_on_each_axis_and_equal_is_accepted() {
    // x: min_x (16) above max_x (32); y: min_y (24) above max_y (40)
    assert_eq!(m(&put_f64(16, 2000.5)), Err(ContainerError::InvertedBounds { axis: "x" }));
    assert_eq!(m(&put_f64(24, 1500.8)), Err(ContainerError::InvertedBounds { axis: "y" }));
    let degenerate = with(&put_f64(32, -100.5), 40, &(-50.25f64).to_le_bytes());
    let b = m(&degenerate).unwrap().bounds;
    assert_eq!((b.min, b.max), ([-100.5, -50.25], [-100.5, -50.25]));
}

#[test]
fn negative_zero_in_a_bound_is_read_as_positive_zero() {
    // NP-03: -0.0 is stored as +0.0, so the same bounds never mean two byte strings.
    let man = m(&with(&put_f64(32, 0.0), 16, &(-0.0f64).to_le_bytes())).unwrap();
    assert_eq!(man.bounds.min[0], 0.0);
    assert!(man.bounds.min[0].is_sign_positive());
}

#[test]
fn the_capability_manifest_feeds_negotiation_as_a_declaration() {
    let d: Declaration = expected_manifest().declaration();
    assert_eq!(d.min_reader, Version::new(1, 0, 0));
    assert_eq!(d.capabilities.len(), 2);
    assert_eq!(d.capabilities[0].id, CapabilityId::new(1));
    assert_eq!(d.capabilities[0].requirement, Requirement::Required);
    assert_eq!(d.capabilities[1].id, CapabilityId::new(13));
    assert_eq!(d.capabilities[1].requirement, Requirement::Optional);
    assert_eq!(d.capabilities[1].version, Version::new(1, 2, 3));
}

#[test]
fn capability_entries_are_read_from_their_offsets() {
    let b = MANIFEST_BYTES.to_vec();
    let c = m(&with(&b, FIXED, &5u32.to_le_bytes())).unwrap();
    assert_eq!(c.capabilities[0].id, CapabilityId::new(5));
    let c = m(&with(&b, FIXED + 4, &2u16.to_le_bytes())).unwrap();
    assert_eq!(c.capabilities[0].version.major, 2);
    let c = m(&with(&b, FIXED + 6, &3u16.to_le_bytes())).unwrap();
    assert_eq!(c.capabilities[0].version.minor, 3);
    let c = m(&with(&b, FIXED + 8, &4u16.to_le_bytes())).unwrap();
    assert_eq!(c.capabilities[0].version.patch, 4);
    let c = m(&with(&b, FIXED + 10, &[2])).unwrap();
    assert_eq!(c.capabilities[0].requirement, Requirement::Optional);
}

#[test]
fn a_zero_a_repeated_or_a_malformed_capability_is_refused() {
    assert_eq!(m(&with(&MANIFEST_BYTES, FIXED, &0u32.to_le_bytes())), Err(ContainerError::ZeroCapabilityId));
    assert_eq!(
        m(&with(&MANIFEST_BYTES, FIXED + 16, &1u32.to_le_bytes())),
        Err(ContainerError::DuplicateCapability { id: 1 })
    );
    for v in [0u8, 3, 255] {
        assert_eq!(
            m(&with(&MANIFEST_BYTES, FIXED + 10, &[v])),
            Err(ContainerError::InvalidRequirement { id: 1, value: v })
        );
    }
}

#[test]
fn an_unknown_capability_id_still_parses() {
    // Whether the reader implements it is negotiation's business (P4.7), not the manifest's.
    let c = m(&with(&MANIFEST_BYTES, FIXED, &9999u32.to_le_bytes())).unwrap();
    assert_eq!(c.capabilities[0].id, CapabilityId::new(9999));
}

#[test]
fn a_lying_capability_count_is_refused_without_allocating_it() {
    for count in [3u32, 1000, u32::MAX] {
        let r = m(&with(&MANIFEST_BYTES, 4, &count.to_le_bytes()));
        assert!(matches!(r, Err(ContainerError::Truncated { .. })), "{count}: {r:?}");
    }
}

#[test]
fn a_lying_directory_count_is_refused_without_allocating_it() {
    for count in [3u32, 1000, u32::MAX] {
        let r = m(&with(&MANIFEST_BYTES, 56, &count.to_le_bytes()));
        assert!(matches!(r, Err(ContainerError::Truncated { .. })), "{count}: {r:?}");
    }
}

#[test]
fn directory_pointers_are_read_from_their_offsets() {
    let base = FIXED + 32;
    let b = MANIFEST_BYTES.to_vec();
    assert_eq!(
        m(&with(&b, base, &2u32.to_le_bytes())).unwrap().directories[0].kind,
        DirectoryPointer::KIND_COMPONENT
    );
    assert_eq!(m(&with(&b, base + 8, &512u64.to_le_bytes())).unwrap().directories[0].offset, 512);
    assert_eq!(m(&with(&b, base + 24, &1u32.to_le_bytes())).unwrap().directories[1].kind, 1);
    assert_eq!(m(&with(&b, base + 40, &128u64.to_le_bytes())).unwrap().directories[1].length, 128);
}

#[test]
fn a_directory_pointer_past_the_end_of_the_file_is_refused_not_clamped() {
    let base = FIXED + 32;
    for (offset, length) in
        [(ARTIFACT_LEN, 1u64), (4096, ARTIFACT_LEN), (ARTIFACT_LEN - 10, 11), (u64::MAX, 1), (1, u64::MAX)]
    {
        let b =
            with(&with(&MANIFEST_BYTES, base + 8, &offset.to_le_bytes()), base + 16, &length.to_le_bytes());
        match m(&b) {
            Err(ContainerError::PointerOutOfRange { field, .. }) => assert_eq!(field, "directory"),
            other => panic!("{offset}+{length}: {other:?}"),
        }
    }
}

#[test]
fn a_directory_pointer_into_the_header_is_refused() {
    let base = FIXED + 32;
    let b = with(&MANIFEST_BYTES, base + 8, &16u64.to_le_bytes());
    assert!(matches!(m(&b), Err(ContainerError::PointerOutOfRange { field: "directory", .. })));
}

#[test]
fn the_block_checksum_table_pointer_is_validated_too() {
    for (offset, length) in [(ARTIFACT_LEN, 1u64), (7168, 2000), (u64::MAX, 2), (32, 8)] {
        let b = with(&with(&MANIFEST_BYTES, 64, &offset.to_le_bytes()), 72, &length.to_le_bytes());
        assert!(
            matches!(m(&b), Err(ContainerError::PointerOutOfRange { field: "block_checksums", .. })),
            "{offset}+{length}"
        );
    }
    // exactly at the end of the file is fine
    let b = with(&with(&MANIFEST_BYTES, 64, &(ARTIFACT_LEN - 8).to_le_bytes()), 72, &8u64.to_le_bytes());
    assert!(m(&b).is_ok());
    // an offset with no length (or the reverse) is a pointer, not "absent"
    let b = with(&with(&MANIFEST_BYTES, 64, &[0; 8]), 72, &8u64.to_le_bytes());
    assert!(matches!(m(&b), Err(ContainerError::PointerOutOfRange { .. })));
}

#[test]
fn a_manifest_whose_table_the_header_places_elsewhere_is_refused() {
    let h = Header { directory_table_offset: 232, ..expected_header() };
    assert_eq!(
        Manifest::parse(&MANIFEST_BYTES, &h, ARTIFACT_LEN),
        Err(ContainerError::DirectoryTableMismatch)
    );
    let h = Header { directory_table_length: 24, ..expected_header() };
    assert_eq!(
        Manifest::parse(&MANIFEST_BYTES, &h, ARTIFACT_LEN),
        Err(ContainerError::DirectoryTableMismatch)
    );
}

#[test]
fn a_manifest_shorter_than_the_header_says_is_truncated_at_every_length() {
    for n in 0..MANIFEST_BYTES.len() {
        let r = m(&MANIFEST_BYTES[..n]);
        assert!(matches!(r, Err(ContainerError::Truncated { .. })), "{n} bytes: {r:?}");
    }
}

#[test]
fn trailing_bytes_are_refused_at_the_same_minor_and_ignored_at_a_higher_one() {
    let mut longer = MANIFEST_BYTES.to_vec();
    longer.extend_from_slice(&[0xab; 16]);
    let h = Header { manifest_length: 240, ..expected_header() };
    assert_eq!(Manifest::parse(&longer, &h, ARTIFACT_LEN), Err(ContainerError::TrailingBytes { extra: 16 }));
    let h = Header { manifest_length: 240, minor: 1, ..expected_header() };
    assert_eq!(Manifest::parse(&longer, &h, ARTIFACT_LEN).unwrap(), expected_manifest());
}

#[test]
fn string_fields_are_bounded_and_utf8() {
    let b = with(&MANIFEST_BYTES, 112, &0u32.to_le_bytes());
    assert_eq!(m(&b), Err(ContainerError::MissingProducer));
    let b = with(&MANIFEST_BYTES, 112, &5000u32.to_le_bytes());
    assert_eq!(m(&b), Err(ContainerError::StringTooLong { field: "producer_id", length: 5000 }));
    let b = with(&MANIFEST_BYTES, 116, &5000u32.to_le_bytes());
    assert_eq!(m(&b), Err(ContainerError::StringTooLong { field: "producer_version", length: 5000 }));
    let b = with(&MANIFEST_BYTES, 120, &5000u32.to_le_bytes());
    assert_eq!(m(&b), Err(ContainerError::StringTooLong { field: "corpus_revision", length: 5000 }));
    let b = with(&MANIFEST_BYTES, 208, &[0xff]);
    assert_eq!(m(&b), Err(ContainerError::InvalidUtf8 { field: "producer_id" }));
}

#[test]
fn the_corpus_revision_is_optional_and_padding_must_be_zero() {
    let mut man = expected_manifest();
    man.metadata.corpus_revision = None;
    let bytes = man.encode();
    let h = Header { manifest_length: bytes.len() as u64, directory_table_length: 48, ..expected_header() };
    assert_eq!(Manifest::parse(&bytes, &h, ARTIFACT_LEN).unwrap(), man);
    // the pad byte after "r1" is at 208 + 8 + 5 + 2
    let b = with(&MANIFEST_BYTES, 223, &[1]);
    assert!(matches!(m(&b), Err(ContainerError::NonZeroReserved { .. })));
}

#[test]
fn no_input_makes_the_manifest_parser_panic() {
    for i in 0..MANIFEST_BYTES.len() {
        for v in [0u8, 1, 0x7f, 0x80, 0xff] {
            let _ = m(&with(&MANIFEST_BYTES, i, &[v]));
        }
    }
    for n in 0..300 {
        let _ = m(&vec![0xff; n]);
        let _ = m(&vec![0u8; n]);
    }
}
