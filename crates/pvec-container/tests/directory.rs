//! The block directory (issue P4.4): one 64 byte entry per block, hand-written bytes first.

use pvec_container::Bounds;
use pvec_container::directory::*;

fn entry() -> BlockEntry {
    BlockEntry {
        offset: 4096,
        stored_length: 800,
        uncompressed_length: 800,
        count: 25,
        cell: 6,
        bounds: Some(Bounds { min: [-1.5, 2.0], max: [10.0, 20.25] }),
        kind: BlockKind::Hot.id(),
        subtype: 3,
        flags: FLAG_MUST_UNDERSTAND | FLAG_HAS_BOUNDS,
        codec: CODEC_NONE,
    }
}

/// Written by hand from the table in `docs/CONTAINER.md` section 4.2, not by the code under test.
const ENTRY_BYTES: [u8; 64] = [
    0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // offset = 4096
    0x20, 0x03, 0x00, 0x00, // stored_length = 800
    0x20, 0x03, 0x00, 0x00, // uncompressed_length = 800
    0x19, 0x00, 0x00, 0x00, // count = 25
    0x06, 0x00, 0x00, 0x00, // cell = 6
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xf8, 0xbf, // min_x = -1.5
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x40, // min_y = 2.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x24, 0x40, // max_x = 10.0
    0x00, 0x00, 0x00, 0x00, 0x00, 0x40, 0x34, 0x40, // max_y = 20.25
    0x01, 0x00, // kind = 1 (hot)
    0x03, 0x00, // subtype = 3
    0x03, // flags
    0x00, // codec
    0x00, 0x00, // reserved
];

fn table(entries: &[[u8; 64]]) -> Vec<u8> {
    entries.iter().flatten().copied().collect()
}

fn at(offset: u64, len: u32) -> BlockEntry {
    BlockEntry { offset, stored_length: len, uncompressed_length: len, ..entry() }
}

fn parse(entries: &[BlockEntry], artifact_len: u64) -> Result<Vec<BlockEntry>, DirectoryError> {
    let t: Vec<[u8; 64]> = entries.iter().map(BlockEntry::encode).collect();
    parse_directory(&table(&t), 64, artifact_len)
}

#[test]
fn the_entry_table_covers_the_64_bytes_with_no_gaps() {
    let mut at = 0;
    for f in BLOCK_ENTRY_FIELDS {
        assert_eq!(f.offset, at, "{}", f.name);
        at += f.size;
    }
    assert_eq!(at, ENTRY_LEN);
    assert_eq!(ENTRY_LEN, 64);
}

#[test]
fn an_entry_encodes_to_the_bytes_written_by_hand() {
    assert_eq!(entry().encode(), ENTRY_BYTES);
}

#[test]
fn the_hand_written_bytes_parse_to_the_entry() {
    assert_eq!(parse_directory(&ENTRY_BYTES, 64, 8192).unwrap(), vec![entry()]);
}

#[test]
fn a_directory_round_trips_and_an_empty_one_parses() {
    let es = [at(64, 800), at(864, 16), at(880, 8)];
    assert_eq!(parse(&es, 4096).unwrap(), es.to_vec());
    assert_eq!(parse_directory(&[], 64, 4096).unwrap(), vec![]);
}

#[test]
fn every_block_kind_has_a_stable_id() {
    let ids: Vec<u16> = BlockKind::ALL.iter().map(|k| k.id()).collect();
    assert_eq!(ids, [1, 2, 3, 4]);
    for k in BlockKind::ALL {
        assert_eq!(BlockKind::from_id(k.id()), Some(k));
    }
    assert_eq!(BlockKind::from_id(0), None);
    assert_eq!(BlockKind::from_id(5), None);
}

#[test]
fn dir_01_a_table_that_is_not_whole_entries_is_refused() {
    assert_eq!(parse_directory(&ENTRY_BYTES[..63], 64, 8192), Err(DirectoryError::BadLength { length: 63 }));
}

#[test]
fn dir_02_a_reserved_field_must_be_zero() {
    let mut b = ENTRY_BYTES;
    b[62] = 1;
    assert_eq!(parse_directory(&b, 64, 8192), Err(DirectoryError::NonZeroReserved { index: 0 }));
}

#[test]
fn dir_03_an_undefined_flag_bit_is_refused() {
    let mut b = ENTRY_BYTES;
    b[60] = 0x07;
    assert_eq!(parse_directory(&b, 64, 8192), Err(DirectoryError::BadFlags { index: 0, bits: 0x07 }));
}

#[test]
fn dir_04_an_unknown_kind_is_refused_when_it_must_be_understood_and_kept_when_it_may_be_skipped() {
    let mut b = ENTRY_BYTES;
    b[56] = 99;
    assert_eq!(parse_directory(&b, 64, 8192), Err(DirectoryError::UnknownRequiredKind { index: 0, id: 99 }));
    b[60] = FLAG_HAS_BOUNDS;
    let kept = parse_directory(&b, 64, 8192).unwrap();
    assert_eq!(kept[0].kind, 99);
    assert_eq!(kept[0].block_kind(), None);
}

#[test]
fn dir_05_an_unknown_codec_is_refused() {
    let mut b = ENTRY_BYTES;
    b[61] = 9;
    assert_eq!(parse_directory(&b, 64, 8192), Err(DirectoryError::UnknownCodec { index: 0, id: 9 }));
}

#[test]
fn dir_06_a_block_outside_the_artifact_or_before_the_header_is_refused() {
    let past = parse(&[at(4096, 800)], 4800).unwrap_err();
    assert_eq!(past, DirectoryError::OutOfRange { index: 0, offset: 4096, length: 800, artifact_len: 4800 });
    assert!(matches!(parse(&[at(32, 8)], 4800), Err(DirectoryError::OutOfRange { index: 0, .. })));
    let wrap = BlockEntry { offset: u64::MAX - 7, stored_length: 16, uncompressed_length: 16, ..entry() };
    assert!(matches!(parse(&[wrap], 4800), Err(DirectoryError::OutOfRange { index: 0, .. })));
}

#[test]
fn dir_07_a_block_that_is_not_8_aligned_is_refused() {
    assert_eq!(parse(&[at(100, 8)], 4800), Err(DirectoryError::Misaligned { index: 0, offset: 100 }));
}

#[test]
fn dir_08_blocks_that_overlap_or_run_backwards_are_refused() {
    assert_eq!(parse(&[at(64, 800), at(800, 8)], 4800), Err(DirectoryError::Overlap { index: 1 }));
    assert_eq!(parse(&[at(1024, 8), at(64, 8)], 4800), Err(DirectoryError::Overlap { index: 1 }));
}

#[test]
fn dir_09_stored_and_decoded_lengths_must_agree_while_there_is_no_codec() {
    let e = BlockEntry { uncompressed_length: 801, ..at(64, 800) };
    assert_eq!(parse(&[e], 4800), Err(DirectoryError::LengthMismatch { index: 0 }));
}

#[test]
fn dir_10_bounds_must_be_valid_and_zero_unless_the_flag_says_otherwise() {
    let nan = BlockEntry { bounds: Some(Bounds { min: [f64::NAN, 0.0], max: [1.0, 1.0] }), ..at(64, 8) };
    assert_eq!(parse(&[nan], 4800), Err(DirectoryError::BadBounds { index: 0 }));
    let inverted = BlockEntry { bounds: Some(Bounds { min: [2.0, 0.0], max: [1.0, 1.0] }), ..at(64, 8) };
    assert_eq!(parse(&[inverted], 4800), Err(DirectoryError::BadBounds { index: 0 }));
    let mut b = ENTRY_BYTES;
    b[60] = FLAG_MUST_UNDERSTAND;
    assert_eq!(
        parse_directory(&b, 64, 8192),
        Err(DirectoryError::BadBounds { index: 0 }),
        "bounds without the flag"
    );
}

#[test]
fn every_directory_error_reports_a_listed_rule() {
    assert!(DIRECTORY_RULES.len() >= 10, "{} rules listed", DIRECTORY_RULES.len());
    let ids: Vec<&str> = DIRECTORY_RULES.iter().map(|r| r.0).collect();
    for e in [
        DirectoryError::BadLength { length: 1 },
        DirectoryError::NonZeroReserved { index: 0 },
        DirectoryError::BadFlags { index: 0, bits: 4 },
        DirectoryError::UnknownRequiredKind { index: 0, id: 9 },
        DirectoryError::UnknownCodec { index: 0, id: 9 },
        DirectoryError::OutOfRange { index: 0, offset: 1, length: 1, artifact_len: 1 },
        DirectoryError::Misaligned { index: 0, offset: 1 },
        DirectoryError::Overlap { index: 0 },
        DirectoryError::LengthMismatch { index: 0 },
        DirectoryError::BadBounds { index: 0 },
    ] {
        assert!(ids.contains(&e.rule()), "{e} reports {}, which is not listed", e.rule());
    }
}

#[test]
fn no_input_makes_the_directory_parser_panic() {
    let mut state = 0x1234_5678_9abc_def1u64;
    for round in 0..2000 {
        let mut b = ENTRY_BYTES.to_vec();
        for _ in 0..1 + round % 5 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state % 64) as usize;
            b[at] = (state >> 32) as u8;
        }
        let _ = parse_directory(&b, 64, 1 << 20);
        let _ = parse_directory(&b[..(state % 65) as usize], 64, 1 << 20);
    }
}
