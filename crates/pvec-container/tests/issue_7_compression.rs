//! Per-block compression (issue P4.6): the codecs, what the writer does with them, and what the reader
//! does when a compressed block lies. Compression is the place a hostile file is cheapest to make
//! expensive, so most of this is about bounds: the decoder stops at the declared length, the declared
//! length is held to the expansion ratio before it is reserved, and a corrupt block is caught before it
//! reaches the decompressor.

#[path = "support/counting_alloc.rs"]
mod counting_alloc;
mod support;

use pvec_container::compress::{Codec, CodecError, CodecPolicy, compress, decompress, pack};
use pvec_container::directory::BlockKind;
use pvec_container::limits::{LimitKind, Limits};
use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::MemorySource;
use std::panic::{AssertUnwindSafe, catch_unwind};
use support::art::{self, Art, SEALED, sealed_with};
use support::fixtures::{Rng, fixtures};

#[global_allocator]
static A: counting_alloc::Counting = counting_alloc::Counting;

const REAL: [Codec; 2] = [Codec::Lz4, Codec::Deflate];

/// Bytes with structure at three levels of entropy, so a codec has something to find and something not
/// to find.
fn sample(rng: &mut Rng, len: usize, kind: usize) -> Vec<u8> {
    match kind % 4 {
        0 => vec![0; len],
        1 => (0..len).map(|i| (i % 7) as u8).collect(),
        2 => (0..len).map(|i| if i % 8 < 4 { (i / 8) as u8 } else { 0 }).collect(),
        _ => (0..len).map(|_| rng.next() as u8).collect(),
    }
}

fn read_all(art: &Art, limits: Limits) -> Result<Vec<Vec<u8>>, ReadError> {
    let cfg = ReaderConfig { limits, ..ReaderConfig::default() };
    let reader = ContainerReader::open(MemorySource::new(art.bytes.clone()), cfg)?;
    (0..art.entries.len())
        .map(|i| reader.read_block(BlockId(i as u32)).map(|b| b.as_bytes().to_vec()))
        .collect()
}

// ---------------------------------------------------------------- the codecs

#[test]
fn codec_ids_are_the_ones_the_document_says_and_round_trip_through_from_id() {
    assert_eq!(Codec::None.id(), 0);
    assert_eq!(Codec::Lz4.id(), 1);
    assert_eq!(Codec::Deflate.id(), 2);
    for c in Codec::ALL {
        assert_eq!(Codec::from_id(c.id()), Some(c));
    }
    assert_eq!(Codec::from_id(3), None);
    assert_eq!(Codec::from_id(255), None);
}

#[test]
fn every_codec_round_trips_every_kind_of_input_at_every_size() {
    let mut rng = Rng(0xc0de_c0de);
    for codec in REAL {
        for len in [0usize, 1, 2, 7, 8, 15, 16, 17, 63, 64, 255, 256, 1000, 4096, 65_536, 300_000] {
            for kind in 0..4 {
                let data = sample(&mut rng, len, kind);
                let stored = compress(codec, &data);
                let back = decompress(codec, &stored, data.len())
                    .unwrap_or_else(|e| panic!("{codec:?} {len} bytes of kind {kind}: {e}"));
                assert_eq!(back, data, "{codec:?} {len} bytes of kind {kind}");
            }
        }
    }
}

#[test]
fn a_compressed_block_of_structured_data_is_smaller_and_noise_is_not_made_smaller_by_much() {
    let mut rng = Rng(5);
    for codec in REAL {
        let zeros = compress(codec, &vec![0u8; 100_000]);
        assert!(zeros.len() < 1000, "{codec:?} stored 100,000 zeros in {} bytes", zeros.len());
        let noise = sample(&mut rng, 100_000, 3);
        let stored = compress(codec, &noise);
        assert!(stored.len() > 90_000, "noise cannot be compressed: {codec:?} gave {}", stored.len());
    }
}

#[test]
fn the_decoder_stops_at_the_declared_length_and_says_so() {
    for codec in REAL {
        let bomb = compress(codec, &vec![0u8; 64 << 20]);
        assert!(bomb.len() < 1 << 20, "{codec:?}: the bomb is {} bytes", bomb.len());
        let before = counting_alloc::requested();
        let e = decompress(codec, &bomb, 1024).unwrap_err();
        let used = counting_alloc::requested() - before;
        assert_eq!(e, CodecError::Overrun { declared: 1024 }, "{codec:?}");
        assert!(used < 256 << 10, "{codec:?}: expanding a 64 MiB stream into 1 KiB requested {used} bytes");
        assert_eq!(e.rule(), "CODEC-01");
    }
}

#[test]
fn a_stream_that_ends_short_of_the_declared_length_is_an_error_and_not_a_short_block() {
    for codec in REAL {
        let stored = compress(codec, &vec![7u8; 1000]);
        let e = decompress(codec, &stored, 2000).unwrap_err();
        assert!(
            matches!(e, CodecError::Short { declared: 2000, got: 1000 } | CodecError::Malformed),
            "{codec:?} {e:?}"
        );
    }
}

#[test]
fn a_decoder_never_panics_on_garbage_truncation_or_a_flipped_bit_and_never_allocates_past_the_declared_length()
 {
    let mut rng = Rng(0xbad_5eed);
    for codec in REAL {
        let data = sample(&mut rng, 5000, 2);
        let good = compress(codec, &data);
        for round in 0..3000 {
            let mut bytes = good.clone();
            match round % 4 {
                0 => {
                    let at = (rng.next() as usize) % bytes.len();
                    bytes[at] ^= 1 << (rng.next() % 8);
                }
                1 => bytes.truncate((rng.next() as usize) % bytes.len()),
                2 => bytes = (0..(rng.next() % 300)).map(|_| rng.next() as u8).collect(),
                _ => bytes.extend((0..(rng.next() % 40)).map(|_| rng.next() as u8)),
            }
            let declared = [0usize, 1, 100, 5000, 6000][(rng.next() % 5) as usize];
            let before = counting_alloc::requested();
            let r = catch_unwind(AssertUnwindSafe(|| decompress(codec, &bytes, declared)));
            let used = counting_alloc::requested() - before;
            let r = r.unwrap_or_else(|_| panic!("{codec:?} panicked on round {round}"));
            if let Ok(out) = r {
                assert_eq!(
                    out.len(),
                    declared,
                    "a block that decodes decodes to exactly the declared length"
                );
            }
            assert!(
                used <= 8 * declared + (64 << 10),
                "{codec:?}: {used} bytes requested for a declared {declared}"
            );
        }
    }
}

#[test]
fn the_none_codec_is_the_identity() {
    let data = vec![1u8, 2, 3, 4, 5];
    assert_eq!(compress(Codec::None, &data), data);
    assert_eq!(decompress(Codec::None, &data, 5).unwrap(), data);
    assert!(decompress(Codec::None, &data, 4).is_err(), "a length that disagrees is an error even for none");
}

// ---------------------------------------------------------------- what the writer does

#[test]
fn a_compressed_artifact_reads_back_to_the_bytes_of_the_uncompressed_one_for_every_fixture_and_codec() {
    for input in fixtures() {
        let plain = art::from_input_with(&input, SEALED);
        let want = read_all(&plain, Limits::DEFAULT).expect("the plain artifact reads");
        for codec in REAL {
            let packed = art::from_input_with(&input, sealed_with(CodecPolicy::uniform(codec)));
            assert_eq!(packed.entries.len(), plain.entries.len(), "{}", input.name);
            let got = read_all(&packed, Limits::DEFAULT)
                .unwrap_or_else(|e| panic!("{} {codec:?}: {e}", input.name));
            assert_eq!(got, want, "{} with {codec:?}", input.name);
            for (p, q) in packed.entries.iter().zip(&plain.entries) {
                assert_eq!(
                    p.uncompressed_length, q.stored_length,
                    "the descriptor records the decoded length"
                );
            }
        }
    }
}

#[test]
fn the_codec_is_recorded_in_each_descriptor_and_a_block_that_does_not_shrink_stays_stored() {
    let art = art::from_input_with(&fixtures()[2], sealed_with(CodecPolicy::uniform(Codec::Lz4)));
    assert!(art.entries.iter().any(|e| e.codec == Codec::Lz4.id()), "something was compressed");
    for e in &art.entries {
        match Codec::from_id(e.codec).expect("a known codec") {
            Codec::None => assert_eq!(e.stored_length, e.uncompressed_length),
            _ => {
                assert!(e.stored_length < e.uncompressed_length, "a compressed block is smaller than it was")
            }
        }
    }
    // a block that would not shrink is stored as it is, not made larger by trying
    let mut rng = Rng(77);
    let noise = sample(&mut rng, 200, 3);
    for codec in REAL {
        let (used, stored) = pack(codec, noise.clone());
        assert_eq!((used, &stored), (Codec::None, &noise), "{codec:?}: noise stays stored");
        let (used, stored) = pack(codec, vec![0u8; 4096]);
        assert_eq!(used, codec);
        assert!(stored.len() < 100);
    }
}

#[test]
fn different_kinds_of_block_may_use_different_codecs_and_an_uncompressed_block_stays_legal() {
    let policy =
        CodecPolicy { hot: Codec::Lz4, records: Codec::None, arena: Codec::Deflate, components: Codec::None };
    let art = art::from_input_with(&fixtures()[2], sealed_with(policy));
    let codecs: Vec<(BlockKind, u8)> =
        art.entries.iter().map(|e| (e.block_kind().unwrap(), e.codec)).collect();
    assert!(
        codecs.iter().all(|(k, c)| *c == Codec::None.id() || *c == policy.for_kind(*k).id()),
        "{codecs:?}"
    );
    assert!(codecs.iter().any(|(k, c)| *k == BlockKind::Hot && *c == Codec::Lz4.id()), "{codecs:?}");
    assert!(codecs.iter().any(|(k, c)| *k == BlockKind::Arena && *c == Codec::Deflate.id()), "{codecs:?}");
    assert!(codecs.iter().all(|(k, c)| *k != BlockKind::Records || *c == 0), "{codecs:?}");
    let plain = art::from_input_with(&fixtures()[2], SEALED);
    assert_eq!(read_all(&art, Limits::DEFAULT).unwrap(), read_all(&plain, Limits::DEFAULT).unwrap());
}

#[test]
fn a_compressed_artifact_is_byte_identical_whatever_order_it_was_written_in() {
    let input = &fixtures()[2];
    let options = sealed_with(CodecPolicy::uniform(Codec::Deflate));
    let first = input.write_with(options).unwrap();
    for seed in 1..6 {
        assert_eq!(input.shuffled(seed).write_with(options).unwrap(), first, "seed {seed}");
    }
}

// ---------------------------------------------------------------- what the reader does with a lie

#[test]
fn a_block_declaring_a_decompressed_length_above_the_ratio_is_refused_without_the_size_being_reserved() {
    for codec in REAL {
        let art = art::from_input_with(&fixtures()[0], art::sealed_with(CodecPolicy::uniform(codec)));
        let i = art.entries.iter().position(|e| e.codec == codec.id()).expect("a compressed block");
        let stored = u64::from(art.entries[i].stored_length);
        let hostile = art.clone().with_u32(art.entry_field(i, "uncompressed_length"), u32::MAX);
        let source = MemorySource::new(hostile.bytes.clone());
        let before = counting_alloc::requested();
        let err = ContainerReader::open(source, ReaderConfig::default()).err().expect("refused at open");
        let used = counting_alloc::requested() - before;
        let l = *err.limit().expect("a LimitExceeded");
        assert_eq!(l.limit, LimitKind::ExpansionRatio);
        assert_eq!(l.declared, u64::from(u32::MAX));
        assert_eq!(l.ceiling, stored * u64::from(Limits::DEFAULT.max_expansion_ratio));
        assert!(used < 64 << 10, "{used} bytes requested for a block declared as 4 GiB");
    }
}

#[test]
fn a_stream_that_expands_past_what_the_descriptor_declares_is_stopped_at_the_declared_length() {
    for codec in REAL {
        let art = art::from_input_with(&fixtures()[0], art::UNSEALED_WITH_CODEC(codec));
        let i = art.entries.iter().position(|e| e.codec == codec.id()).expect("a compressed block");
        let real = art.entries[i].uncompressed_length;
        // the declaration is within the ratio, and a good deal less than the stream really holds
        let hostile = art.clone().with_u32(art.entry_field(i, "uncompressed_length"), real / 2);
        let cfg = ReaderConfig::default();
        let reader = ContainerReader::open(MemorySource::new(hostile.bytes.clone()), cfg).expect("opens");
        let live = reader.accountant().live();
        let before = counting_alloc::requested();
        let e = reader.read_block(BlockId(i as u32)).unwrap_err();
        let used = counting_alloc::requested() - before;
        assert!(
            matches!(
                e,
                ReadError::Codec(CodecError::Overrun { .. }) | ReadError::Codec(CodecError::Malformed)
            ),
            "{codec:?} {e:?}"
        );
        assert!(
            used < 8 * real as usize + (64 << 10),
            "{codec:?}: {used} bytes requested for a declared {}",
            real / 2
        );
        assert_eq!(reader.accountant().live(), live, "a block that did not decode holds no memory");
    }
}

#[test]
fn a_stream_that_ends_short_of_what_the_descriptor_declares_is_an_error() {
    for codec in REAL {
        let art = art::from_input_with(&fixtures()[0], art::UNSEALED_WITH_CODEC(codec));
        let i = art.entries.iter().position(|e| e.codec == codec.id()).expect("a compressed block");
        let real = art.entries[i].uncompressed_length;
        let hostile = art.clone().with_u32(art.entry_field(i, "uncompressed_length"), real + 16);
        let reader = ContainerReader::open(MemorySource::new(hostile.bytes.clone()), ReaderConfig::default());
        let e = reader.expect("opens").read_block(BlockId(i as u32)).unwrap_err();
        assert!(matches!(e, ReadError::Codec(_)), "{codec:?} {e:?}");
    }
}

#[test]
fn a_single_corrupted_byte_inside_a_compressed_block_is_caught_before_it_reaches_the_decompressor() {
    for codec in REAL {
        let art = art::from_input_with(&fixtures()[4], sealed_with(CodecPolicy::uniform(codec)));
        let tiny = art::lines_with(40, 1, sealed_with(CodecPolicy::uniform(codec)));
        for art in [art, tiny] {
            let (i, e) = art
                .entries
                .iter()
                .enumerate()
                .find(|(_, e)| e.codec == codec.id())
                .expect("a compressed block");
            for at in e.offset as usize..(e.offset + u64::from(e.stored_length)) as usize {
                let mut bytes = art.bytes.clone();
                bytes[at] ^= 0x10;
                let reader =
                    ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("opens");
                let err = reader.read_block(BlockId(i as u32)).unwrap_err();
                assert!(
                    matches!(err, ReadError::Integrity(_)),
                    "{codec:?}: byte {at} of block {i} reached the decoder or the parser: {err:?}"
                );
                assert_eq!(err.rule(), "INTEGRITY-03");
            }
        }
    }
}

#[test]
fn without_a_checksum_the_same_corruption_reaches_the_decoder_which_is_what_the_checksum_is_for() {
    let art = art::lines_with(40, 1, art::UNSEALED_WITH_CODEC(Codec::Lz4));
    let e = &art.entries[0];
    assert_eq!(e.codec, Codec::Lz4.id());
    let mut reached = 0;
    for at in e.offset as usize..(e.offset + u64::from(e.stored_length)) as usize {
        let mut bytes = art.bytes.clone();
        bytes[at] ^= 0x10;
        let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("opens");
        if matches!(reader.read_block(BlockId(0)), Err(ReadError::Codec(_)) | Ok(_)) {
            reached += 1;
        }
    }
    assert!(reached > 0, "some corrupted streams are handed to the decoder when nothing checks them first");
}

#[test]
fn a_compressed_block_costs_its_stored_bytes_in_flight_and_its_decoded_bytes_live() {
    let art = art::from_input_with(&fixtures()[0], sealed_with(CodecPolicy::uniform(Codec::Deflate)));
    let i = art.entries.iter().position(|e| e.codec == Codec::Deflate.id()).expect("a compressed block");
    let (stored, decoded) =
        (u64::from(art.entries[i].stored_length), u64::from(art.entries[i].uncompressed_length));
    let reader =
        ContainerReader::open(MemorySource::new(art.bytes.clone()), ReaderConfig::default()).unwrap();
    let before = reader.accountant().live();
    let block = reader.read_block(BlockId(i as u32)).unwrap();
    assert_eq!(reader.accountant().live() - before, decoded, "what stays live is the decoded block");
    assert!(
        reader.accountant().peak() >= before + stored + decoded,
        "the stored bytes were in flight with it"
    );
    drop(block);
}

#[test]
fn the_codecs_are_pure_rust_with_no_native_source_and_no_library_discovered_on_the_build_machine() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"))
        .expect("Cargo.lock");
    let names: Vec<&str> =
        lock.lines().filter_map(|l| l.strip_prefix("name = \"")).map(|l| l.trim_end_matches('"')).collect();
    for banned in [
        "cc",
        "cmake",
        "pkg-config",
        "bindgen",
        "libz-sys",
        "lz4-sys",
        "zstd-sys",
        "lzma-sys",
        "libz-ng-sys",
        "flate2",
    ] {
        assert!(!names.contains(&banned), "Cargo.lock names {banned}, which builds or links native code");
    }
    assert!(names.iter().all(|n| !n.ends_with("-sys")), "a -sys crate is in Cargo.lock: {names:?}");
    for wanted in ["lz4_flex", "miniz_oxide", "xxhash-rust"] {
        assert!(names.contains(&wanted), "the codec and checksum crates are {wanted}");
    }
}

#[test]
fn the_expansion_ratio_is_exact_at_the_ceiling_and_a_refusal_reserves_nothing() {
    let art = art::from_input_with(&fixtures()[0], art::sealed_with(CodecPolicy::uniform(Codec::Lz4)));
    let i = art.entries.iter().position(|e| e.codec == Codec::Lz4.id()).expect("a compressed block");
    let ceiling = u64::from(art.entries[i].stored_length) * u64::from(Limits::DEFAULT.max_expansion_ratio);
    let at = art.entry_field(i, "uncompressed_length");
    // exactly at the ceiling the descriptor is admitted (the stream will not decode to that, which is the
    // decoder's business)
    let admitted = art.clone().with_u32(at, ceiling as u32);
    assert!(ContainerReader::open(MemorySource::new(admitted.bytes), ReaderConfig::default()).is_ok());
    let over = art.clone().with_u32(at, ceiling as u32 + 1);
    let err =
        ContainerReader::open(MemorySource::new(over.bytes), ReaderConfig::default()).err().expect("refused");
    assert_eq!(
        err.limit().map(|l| (l.limit, l.declared, l.ceiling)),
        Some((LimitKind::ExpansionRatio, ceiling + 1, ceiling))
    );
    // and nothing is reserved for a declaration that is admitted and then fails to decode
    let reader = ContainerReader::open(
        MemorySource::new(art.clone().with_u32(at, ceiling as u32).bytes),
        ReaderConfig::default(),
    )
    .unwrap();
    let live = reader.accountant().live();
    assert!(reader.read_block(BlockId(i as u32)).is_err());
    assert_eq!(reader.accountant().live(), live);
}
