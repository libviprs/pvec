//! Artifact integrity (issue P4.6): a per-block checksum over the stored bytes, so a corrupt block is
//! caught before it is decompressed or parsed, and one SHA-256 over the whole file, so a streamed artifact
//! can be pinned. The digest covers every byte with its own 32 bytes read as zero, which is what makes it
//! a digest of the canonical byte stream and not of a part of it.

mod support;

use pvec_container::integrity::{
    DigestStatus, Integrity, IntegrityError, artifact_digest, block_checksum, sha256,
};
use pvec_container::limits::Limits;
use pvec_container::reader::{
    AsyncContainerReader, BlockId, ContainerReader, ReadError, ReaderConfig, Validated,
};
use pvec_container::source::{BlockSource, Bytes, MemorySource, Recording, SourceError};
use pvec_container::spec::m;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};
use support::art::{self, Art, SEALED, UNSEALED};
use support::fixtures::{Rng, fixtures};
use support::sha256::sha256_hex;

fn block_on<F: Future>(f: F) -> F::Output {
    let mut f = pin!(f);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(v) = f.as_mut().poll(&mut cx) {
            return v;
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn open(bytes: Vec<u8>) -> Result<ContainerReader<MemorySource<Vec<u8>>>, ReadError> {
    ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default())
}

/// Open, validate every block and verify the digest: what a consumer that wants to trust a file does.
fn full_check(bytes: Vec<u8>) -> Result<(Validated, DigestStatus), ReadError> {
    let reader = open(bytes)?;
    let validated = reader.validate()?;
    let status = reader.verify_artifact_digest()?;
    Ok((validated, status))
}

// ---------------------------------------------------------------- the hashes

#[test]
fn the_xxh3_64_checksum_matches_the_published_vector_for_an_empty_input() {
    assert_eq!(block_checksum(b""), 0x2D06_8005_38D3_94C2);
    assert_ne!(block_checksum(b"a"), block_checksum(b"b"));
    assert_eq!(block_checksum(&[7u8; 1000]), block_checksum(&[7u8; 1000]));
}

#[test]
fn the_sha256_matches_the_published_vectors_and_an_independent_implementation_at_every_length() {
    assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex(&sha256(b"abc")), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hex(&sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let mut rng = Rng(11);
    for len in (0..300).chain([1000, 4096, 100_000]) {
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        assert_eq!(hex(&sha256(&data)), sha256_hex(&data), "{len} bytes");
    }
}

#[test]
fn the_artifact_digest_is_the_sha256_of_the_file_with_its_own_32_bytes_read_as_zero() {
    let data: Vec<u8> = (0..200u32).map(|i| (i * 7) as u8).collect();
    let mut zeroed = data.clone();
    zeroed[50..82].fill(0);
    assert_eq!(artifact_digest(&data, 50), sha256(&zeroed));
    assert_eq!(artifact_digest(&zeroed, 50), sha256(&zeroed), "whatever those bytes hold");
}

// ---------------------------------------------------------------- what the writer records

#[test]
fn the_default_writer_records_a_checksum_table_between_the_blocks_and_the_directory_and_a_digest() {
    for input in fixtures() {
        let art = Art::new(input.write().unwrap());
        let n = art.entries.len() as u64;
        let table = art.manifest.block_checksums.expect("a checksum table");
        assert_eq!(table.length, 8 * n, "{}: one 8 byte sum per block", input.name);
        assert_eq!(table.offset % 8, 0);
        let last = art.entries.last().unwrap();
        assert!(
            table.offset >= last.offset + u64::from(last.stored_length),
            "{}: after the blocks",
            input.name
        );
        assert_eq!(
            table.offset + table.length,
            art.manifest.directories[0].offset,
            "{}: right before the directory",
            input.name
        );
        for (i, e) in art.entries.iter().enumerate() {
            let stored = &art.bytes[e.offset as usize..(e.offset + u64::from(e.stored_length)) as usize];
            let at = (table.offset as usize) + 8 * i;
            let sum = u64::from_le_bytes(art.bytes[at..at + 8].try_into().unwrap());
            assert_eq!(sum, block_checksum(stored), "{}: block {i}", input.name);
        }
        let digest = art.manifest.artifact_digest.expect("a digest");
        let at = art.header.manifest_offset as usize + m::ARTIFACT_DIGEST;
        let mut zeroed = art.bytes.clone();
        zeroed[at..at + 32].fill(0);
        assert_eq!(
            hex(&digest),
            sha256_hex(&zeroed),
            "{}: the digest is the SHA-256 of the file with the digest zeroed",
            input.name
        );
        assert_eq!(&art.bytes[at..at + 32], &digest[..]);
    }
}

#[test]
fn the_integrity_option_chooses_what_is_recorded() {
    let input = &fixtures()[0];
    let none = Art::new(input.write_with(UNSEALED).unwrap());
    assert!(none.manifest.block_checksums.is_none() && none.manifest.artifact_digest.is_none());
    let sums = Art::new(input.write_with(art::with_integrity(Integrity::Checksums)).unwrap());
    assert!(sums.manifest.block_checksums.is_some() && sums.manifest.artifact_digest.is_none());
    let full = Art::new(input.write_with(SEALED).unwrap());
    assert!(full.manifest.block_checksums.is_some() && full.manifest.artifact_digest.is_some());
    assert_eq!(none.entries.len(), full.entries.len());
    // the blocks are the same bytes either way, only the layout around them moves
    for ((a, b), (x, y)) in none
        .entries
        .iter()
        .zip(&full.entries)
        .zip(art::raw_blocks(&none).iter().zip(art::raw_blocks(&full).iter()))
    {
        assert_eq!((a.kind, a.subtype, a.count), (b.kind, b.subtype, b.count));
        assert_eq!(x.stored, y.stored);
    }
}

// ---------------------------------------------------------------- what the reader does

#[test]
fn a_sealed_artifact_opens_validates_and_verifies_on_every_fixture_blocking_and_async() {
    for input in fixtures() {
        let art = Art::new(input.write().unwrap());
        let (validated, status) =
            full_check(art.bytes.clone()).unwrap_or_else(|e| panic!("{}: {e}", input.name));
        assert_eq!(status, DigestStatus::Verified, "{}", input.name);
        assert_eq!(validated.blocks, art.entries.len());
        let source = Recording::new(MemorySource::new(art.bytes.clone()));
        let a = block_on(AsyncContainerReader::open(source, ReaderConfig::default())).unwrap();
        assert_eq!(block_on(a.validate()).unwrap(), validated);
        assert_eq!(block_on(a.verify_artifact_digest()).unwrap(), DigestStatus::Verified);
    }
}

#[test]
fn opening_checks_nothing_it_does_not_have_to_and_verifying_the_digest_is_explicit() {
    let art = art::lines_with(8, 2, SEALED);
    let at = art.header.manifest_offset as usize + m::ARTIFACT_DIGEST;
    let mut bytes = art.bytes.clone();
    bytes[at] ^= 1;
    let reader = open(bytes).expect("a cold open does not hash the file");
    assert!(reader.read_block(BlockId(0)).is_ok(), "a block is read against its own checksum");
    let e = reader.verify_artifact_digest().unwrap_err();
    assert!(matches!(e, ReadError::Integrity(IntegrityError::ArtifactDigest { .. })), "{e:?}");
    assert_eq!(e.rule(), "INTEGRITY-01");
}

#[test]
fn a_flipped_bit_anywhere_in_a_sealed_file_is_caught_by_something() {
    let art = art::lines_with(3, 2, SEALED);
    for at in 0..art.len() {
        let mut bytes = art.bytes.clone();
        bytes[at] ^= 0x04;
        assert!(full_check(bytes).is_err(), "a flipped bit at byte {at} of {} went unnoticed", art.len());
    }
}

#[test]
fn a_corrupt_byte_in_any_block_is_the_block_checksum_and_names_the_block() {
    for art in [art::from_input_with(&fixtures()[4], SEALED), art::lines_with(6, 3, SEALED)] {
        for (i, e) in art.entries.iter().enumerate() {
            for at in e.offset as usize..(e.offset + u64::from(e.stored_length)) as usize {
                let mut bytes = art.bytes.clone();
                bytes[at] ^= 0x01;
                let reader = open(bytes).expect("a corrupt block does not stop the file opening");
                match reader.read_block(BlockId(i as u32)) {
                    Err(ReadError::Integrity(IntegrityError::BlockChecksum { block, expected, found })) => {
                        assert_eq!(block, i as u32);
                        assert_ne!(expected, found);
                    }
                    other => panic!("byte {at} of block {i}: {other:?}"),
                }
                // and the other blocks are untouched by it
                for j in (0..art.entries.len()).filter(|j| *j != i) {
                    assert!(
                        reader.read_block(BlockId(j as u32)).is_ok(),
                        "block {j} was refused for a fault in block {i}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_digest_that_is_present_but_zero_or_wrong_is_refused() {
    let art = art::lines_with(4, 1, SEALED);
    let at = art.header.manifest_offset as usize + m::ARTIFACT_DIGEST;
    for fill in [0x00u8, 0xA5] {
        let mut bytes = art.bytes.clone();
        bytes[at..at + 32].fill(fill);
        let e = open(bytes).unwrap().verify_artifact_digest().unwrap_err();
        assert_eq!(e.rule(), "INTEGRITY-01", "fill {fill:#x}");
    }
}

#[test]
fn the_digest_sees_what_the_block_checksums_do_not_the_manifest_and_the_directory() {
    let art = art::lines_with(4, 2, SEALED);
    // a character of the producer id, and the cell of the second block: no block byte changes
    let producer = art.header.manifest_offset as usize
        + 128
        + 16 * art.manifest.capabilities.len()
        + 24 * art.manifest.directories.len();
    let cell = art.entry_field(1, "cell");
    for at in [producer, cell] {
        let mut bytes = art.bytes.clone();
        bytes[at] ^= 0x01;
        let reader = open(bytes).expect("still a well-formed file");
        for i in 0..art.entries.len() {
            reader.read_block(BlockId(i as u32)).expect("every block still matches its checksum");
        }
        assert_eq!(reader.verify_artifact_digest().unwrap_err().rule(), "INTEGRITY-01", "byte {at}");
    }
}

#[test]
fn a_checksum_table_that_is_not_where_or_what_it_should_be_is_refused_at_open() {
    let art = art::lines_with(4, 3, SEALED);
    let table = art.manifest.block_checksums.unwrap();
    let off = art.header.manifest_offset as usize + m::BLOCK_CHECKSUMS_OFFSET;
    let len = art.header.manifest_offset as usize + m::BLOCK_CHECKSUMS_LENGTH;
    let first = art.entries[0].offset;
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("one entry short", art.clone().with_u64(len, table.length - 8).bytes),
        ("one entry long", art.clone().with_u64(len, table.length + 8).bytes),
        ("not a whole number of entries", art.clone().with_u64(len, table.length - 3).bytes),
        ("over the first block", art.clone().with_u64(off, first).with_u64(len, table.length).bytes),
        ("over the directory", art.clone().with_u64(off, art.manifest.directories[0].offset).bytes),
    ];
    for (what, bytes) in cases {
        let e = open(bytes).err().unwrap_or_else(|| panic!("{what}: a bad table was accepted"));
        let rule = e.rule();
        assert!(
            ["INTEGRITY-02", "PointerOutOfRange", "PointerIntoManifest"].contains(&rule.as_str()),
            "{what}: {rule}"
        );
    }
}

#[test]
fn a_flipped_bit_in_the_table_is_a_mismatch_for_that_block() {
    let art = art::lines_with(4, 3, SEALED);
    let table = art.manifest.block_checksums.unwrap();
    let mut bytes = art.bytes.clone();
    bytes[table.offset as usize + 8 + 3] ^= 0x80;
    let reader = open(bytes).unwrap();
    assert!(reader.read_block(BlockId(0)).is_ok());
    assert!(matches!(
        reader.read_block(BlockId(1)),
        Err(ReadError::Integrity(IntegrityError::BlockChecksum { block: 1, .. }))
    ));
}

#[test]
fn a_file_with_no_integrity_has_nothing_to_verify_unless_the_reader_requires_it() {
    for options in [UNSEALED, art::with_integrity(Integrity::Checksums)] {
        let art = art::lines_with(4, 1, options);
        let reader = open(art.bytes.clone()).unwrap();
        assert_eq!(reader.verify_artifact_digest().unwrap(), DigestStatus::NotWritten);
        let strict = ReaderConfig { require_integrity: true, ..ReaderConfig::default() };
        let e = ContainerReader::open(MemorySource::new(art.bytes.clone()), strict).err().expect("refused");
        assert_eq!(e.rule(), "INTEGRITY-04");
    }
    let sealed = art::lines_with(4, 1, SEALED);
    let strict = ReaderConfig { require_integrity: true, ..ReaderConfig::default() };
    assert!(ContainerReader::open(MemorySource::new(sealed.bytes), strict).is_ok());
}

// ---------------------------------------------------------------- what it costs

/// An artifact of `total` bytes: the real one with a gap of zeros between its blocks and the checksum
/// table, which sits right before the directory as the writer puts it. Nothing is allocated for the gap.
struct Sparse {
    prefix: Vec<u8>,
    tail_at: u64,
    tail: Vec<u8>,
    total: u64,
}

impl Sparse {
    fn new(art: &Art, total: u64) -> Sparse {
        let table = art.manifest.block_checksums.expect("sealed");
        let tail_len = art.bytes.len() as u64 - table.offset;
        let tail_at = total - tail_len;
        let mut prefix = art.bytes[..table.offset as usize].to_vec();
        let moved = (art.header.manifest_offset as usize + m::BLOCK_CHECKSUMS_OFFSET, tail_at);
        prefix[moved.0..moved.0 + 8].copy_from_slice(&moved.1.to_le_bytes());
        let dir_at = tail_at + (art.manifest.directories[0].offset - table.offset);
        let ptr = art.pointer_offset_field();
        prefix[ptr..ptr + 8].copy_from_slice(&dir_at.to_le_bytes());
        Sparse { prefix, tail_at, tail: art.bytes[table.offset as usize..].to_vec(), total }
    }
}

impl BlockSource for Sparse {
    fn len(&self) -> u64 {
        self.total
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let end = offset + u64::from(len);
        if end > self.total {
            return Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len: self.total });
        }
        let mut out = vec![0u8; len as usize];
        for (base, data) in [(0u64, &self.prefix), (self.tail_at, &self.tail)] {
            let (lo, hi) = (offset.max(base), end.min(base + data.len() as u64));
            if lo < hi {
                out[(lo - offset) as usize..(hi - offset) as usize]
                    .copy_from_slice(&data[(lo - base) as usize..(hi - base) as usize]);
            }
        }
        Ok(Bytes::new(out))
    }
}

#[test]
fn a_sealed_1gib_artifact_still_opens_and_reads_one_block_in_three_requests_under_1mib() {
    let art = art::lines_with(40, 6, SEALED);
    let source = Recording::new(Sparse::new(&art, 1 << 30));
    let reader = ContainerReader::open(source, ReaderConfig::default()).expect("opens");
    let block = reader.read_block(BlockId(2)).expect("one block, checked against its checksum");
    assert_eq!(block.as_bytes().len(), art.entries[2].uncompressed_length as usize);
    let s = reader.stats();
    assert!(s.requests <= 3, "{} requests: header prefix, checksums and directory, block", s.requests);
    assert!(s.bytes < 1 << 20, "{} bytes transferred", s.bytes);
}

#[test]
fn the_digest_is_hashed_in_bounded_chunks_and_every_byte_of_the_file_is_read_once() {
    let art = art::lines_with(2000, 4, SEALED);
    assert!(art.len() > 2 << 20, "the artifact is {} bytes", art.len());
    let source = Recording::new(MemorySource::new(art.bytes.clone()));
    let reader = ContainerReader::open(source, ReaderConfig::default()).unwrap();
    let before = reader.source().requests().len();
    let peak_before = reader.accountant().peak();
    assert_eq!(reader.verify_artifact_digest().unwrap(), DigestStatus::Verified);
    let asked: Vec<_> = reader.source().requests()[before..].to_vec();
    assert!(asked.iter().all(|r| r.len <= 1 << 20), "a chunk over 1 MiB: {asked:?}");
    assert_eq!(asked.iter().map(|r| u64::from(r.len)).sum::<u64>(), art.len() as u64, "each byte once");
    assert!(
        reader.accountant().peak() <= peak_before.max(art.len() as u64)
            && reader.accountant().peak() < 3 << 20
    );
    let tight = Limits { max_alloc_bytes: 256 << 10, ..Limits::DEFAULT };
    let cfg = ReaderConfig { limits: tight, ..ReaderConfig::default() };
    let r = ContainerReader::open(MemorySource::new(art.bytes.clone()), cfg).unwrap();
    assert!(
        r.verify_artifact_digest().is_err(),
        "a budget under one chunk refuses the verification, it does not overrun"
    );
}
