//! Capability-owned block kinds (issue 216, EPIC R, ADR 0018): kinds 9 and 10 are reserved for capability 21
//! and are not in `BlockKind::ALL`, so a reader that does not implement 21 skips them from the directory
//! entry alone, a reader that does is handed them, and a block of a reserved kind that names any other
//! capability (or none) is refused. DIR-02 is not touched: a kind the format defines still names none.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_container::directory::BlockKind;
use pvec_container::integrity::Integrity;
use pvec_container::negotiate::{Effect, Outcome, ReaderCapabilities, Refusal, SkipReason, negotiate};
use pvec_container::reader::{BlockId, ContainerReader, ReadError, ReaderConfig};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::RawBlock;
use support::art::{self, Art, SEALED, rebuild, rebuild_raw};
use support::fixtures::fixtures;

const RASTER_INDEX: u16 = 9;
const RASTER_TILES: u16 = 10;

fn cap(id: u32) -> CapabilityId {
    CapabilityId::new(id)
}

/// An opaque block of `kind` that names `capability`: the payload is never read at this stage.
fn block(kind: u16, capability: u16, fill: u8) -> RawBlock {
    RawBlock {
        kind,
        subtype: 0,
        cell: 0,
        count: 1,
        bounds: None,
        flags: 0,
        codec: 0,
        capability,
        uncompressed_length: 64,
        stored: (0..64u8).map(|i| fill ^ i).collect(),
    }
}

/// `base` plus kind 9 and kind 10 entries naming capability 21, which the manifest declares optional at 1.0.
fn raster_file(base: &Art) -> Art {
    rebuild(base, Integrity::Full, |m, blocks| {
        m.capabilities.push(Declared {
            id: cap(21),
            requirement: Requirement::Optional,
            version: Version::new(1, 0, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
        blocks.push(block(RASTER_INDEX, 21, 0xA0));
        blocks.push(block(RASTER_TILES, 21, 0xB0));
    })
}

fn without_21() -> ReaderCapabilities {
    ReaderCapabilities::v1().without(cap(21))
}

fn with_21() -> ReaderCapabilities {
    ReaderCapabilities::v1().with(cap(21), 1, 0)
}

type Reader = ContainerReader<Recording<MemorySource<Vec<u8>>>>;

fn open_with(bytes: Vec<u8>, caps: ReaderCapabilities) -> Result<Reader, ReadError> {
    let cfg = ReaderConfig { capabilities: caps, ..ReaderConfig::default() };
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), cfg)
}

fn geometry(reader: &Reader) -> Vec<(u16, u16, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        if matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records | BlockKind::Arena)) {
            let b = reader.read_block(BlockId(i as u32)).expect("a geometry block reads");
            out.push((e.kind, e.subtype, e.cell, b.as_bytes().to_vec()));
        }
    }
    out
}

#[test]
fn a_reader_without_21_opens_degraded_lists_21_and_skips_both_entries_naming_it() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = raster_file(&base);
    let reader =
        open_with(art.bytes.clone(), without_21()).expect("an optional capability does not stop the open");
    let partial = reader.partial_open().expect("a report");
    let c = partial.capabilities.iter().find(|c| c.id == cap(21)).expect("21 is listed");
    assert_eq!((c.reason, c.effect), (SkipReason::NotImplemented, Effect::Unrastered));
    assert_eq!(partial.blocks.len(), 2);
    for (b, kind) in partial.blocks.iter().zip([RASTER_INDEX, RASTER_TILES]) {
        assert_eq!(
            (b.kind, b.capability, b.reason),
            (kind, Some(cap(21)), SkipReason::UnknownBlockKind),
            "the entry names the capability it belongs to"
        );
    }
    assert_eq!(partial.skipped_ids(), vec![21]);
    assert!(partial.describe().contains("21"), "{}", partial.describe());
}

#[test]
fn a_skipped_raster_entry_is_never_fetched_and_the_geometry_is_bit_identical() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = raster_file(&base);
    let reader = open_with(art.bytes.clone(), without_21()).unwrap();
    reader.validate().expect("the file validates around the raster entries");
    let ids: Vec<BlockId> = (0..art.entries.len() as u32).map(BlockId).collect();
    let live: Vec<BlockId> = ids
        .iter()
        .copied()
        .filter(|b| {
            !art.entries[b.0 as usize].kind.eq(&RASTER_INDEX)
                && art.entries[b.0 as usize].kind != RASTER_TILES
        })
        .collect();
    reader.read_blocks(&live).unwrap();
    for e in art.entries.iter().filter(|e| e.kind == RASTER_INDEX || e.kind == RASTER_TILES) {
        assert!(
            !reader.source().touched(e.offset, u64::from(e.stored_length)),
            "a request touched the payload of kind {}",
            e.kind
        );
    }
    let alone = open_with(base.bytes.clone(), without_21()).unwrap();
    assert_eq!(geometry(&reader), geometry(&alone), "the geometry is the same with and without the entries");
}

#[test]
fn a_reader_that_declares_21_is_handed_the_entries_and_nothing_is_skipped() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = raster_file(&base);
    assert_eq!(negotiate(&art.manifest, &art.entries, &with_21()), Outcome::Open);
    let reader = open_with(art.bytes.clone(), with_21()).unwrap();
    assert!(reader.partial_open().is_none());
    let i = art.entries.iter().position(|e| e.kind == RASTER_INDEX).expect("the entry");
    let got = reader.read_block(BlockId(i as u32)).expect("a handed block is readable");
    assert_eq!(got.as_bytes().len(), 64);
    reader.validate().expect("and the file validates with the handed blocks in it");
}

#[test]
fn a_newer_major_of_21_is_skipped_for_that_capability_only() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = rebuild(&raster_file(&base), Integrity::Full, |m, _| {
        for d in &mut m.capabilities {
            if d.id == cap(21) {
                d.version = Version::new(2, 0, 0);
            }
        }
    });
    let reader = open_with(art.bytes, with_21()).unwrap();
    let partial = reader.partial_open().expect("a report");
    assert!(partial.blocks.iter().all(|b| b.reason == SkipReason::NewerMajor), "{partial:?}");
    assert_eq!(partial.skipped_ids(), vec![21]);
}

#[test]
fn a_reserved_kind_with_capability_zero_or_another_capability_is_refused() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    for (kind, capability) in
        [(RASTER_INDEX, 0u16), (RASTER_TILES, 0), (RASTER_INDEX, 14), (RASTER_TILES, 99)]
    {
        let art = rebuild(&base, Integrity::Full, |m, blocks| {
            for id in [14u32, 99] {
                m.capabilities.push(Declared {
                    id: cap(id),
                    requirement: Requirement::Optional,
                    version: Version::new(1, 0, 0),
                });
            }
            m.capabilities.sort_by_key(|d| d.id);
            m.capabilities.dedup_by_key(|d| d.id);
            blocks.push(block(kind, capability, 0xC0));
        });
        for caps in [without_21(), with_21()] {
            match negotiate(&art.manifest, &art.entries, &caps) {
                Outcome::Refuse(r @ Refusal::OwnedKindMismatch { .. }) => {
                    assert_eq!(r.rule(), "NEGOTIATE-06");
                }
                other => panic!("kind {kind} capability {capability}: {other:?}"),
            }
        }
    }
}

#[test]
fn an_unreserved_unknown_kind_is_still_skipped_whatever_declared_capability_it_names() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let art = rebuild(&base, Integrity::Full, |m, blocks| {
        m.capabilities.push(Declared {
            id: cap(99),
            requirement: Requirement::Optional,
            version: Version::new(3, 1, 0),
        });
        blocks.push(block(99, 99, 0xD0));
    });
    let reader = open_with(art.bytes, ReaderCapabilities::v1()).unwrap();
    let p = reader.partial_open().expect("a report");
    assert_eq!((p.blocks[0].kind, p.blocks[0].reason), (99, SkipReason::UnknownBlockKind));
}

#[test]
fn dir_02_still_refuses_capability_21_on_a_kind_the_format_defines() {
    let base = art::from_input_with(&fixtures()[2], SEALED);
    let bytes = rebuild_raw(&base, Integrity::Full, |m, blocks| {
        m.capabilities.push(Declared {
            id: cap(21),
            requirement: Requirement::Optional,
            version: Version::new(1, 0, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
        blocks[0].capability = 21;
    });
    let e = open_with(bytes, with_21()).err().expect("refused");
    assert_eq!(e.rule(), "DIR-02", "{e}");
}
