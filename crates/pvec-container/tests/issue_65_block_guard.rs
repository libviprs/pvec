//! Issue P8.6 (pvec#65): a bigger leaf must not make a block the reader's limits refuse.
//!
//! The index leaf holds many more entities after tuning, and one hot block holds a kind's entities of one
//! leaf. A leaf of long polylines (survey contours are 100 to 1,000 vertices each) can then be tens of
//! megabytes in one block, over the 64 MiB the browser preset allows once it is a few hundred vertices
//! apiece. The writer cuts a leaf's blocks of one kind at 16 MiB, so a file it writes is readable by every
//! limit preset. The reader's limits are not touched: this test reads under a ceiling of exactly 16 MiB.
#![allow(missing_docs)]

use pvec_container::directory::BlockKind;
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Limits, Metadata, ReaderConfig};
use pvec_entity::EntityId;

const CEILING: u32 = 16 << 20;

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Metre,
        metadata: Metadata {
            producer_id: "p8.6".into(),
            producer_version: "block-guard".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

/// `n` polylines of 700,000 vertices, about 11 MB each, all in leaf 1.
fn big_leaf(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::new(config());
    for id in 0..n {
        let line: Vec<[f64; 2]> =
            (0..700_000).map(|i| [f64::from(i) * 0.001, f64::from(id) + f64::from(i % 7)]).collect();
        let bounds = Bounds { min: [0.0, f64::from(id)], max: [700.0, f64::from(id) + 6.0] };
        w.push_polyline(EntityMeta { id: EntityId::new(id).unwrap(), style: 0, cell: 1, bounds }, &line)
            .unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn a_leaf_of_big_polylines_is_cut_into_blocks_the_reader_accepts_under_a_16_mib_ceiling() {
    let bytes = big_leaf(3);
    let limits = Limits { max_block_bytes: CEILING, ..Limits::DEFAULT };
    let r =
        ContainerReader::open(MemorySource::new(bytes), ReaderConfig { limits, ..ReaderConfig::default() })
            .expect("opens");
    let hot: Vec<BlockId> = r
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot))
        .map(|(i, _)| BlockId(i as u32))
        .collect();
    assert!(hot.len() >= 2, "{} hot block: 33 MB went into one block", hot.len());
    for e in r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)) {
        assert!(e.uncompressed_length <= CEILING, "a hot block of {} bytes", e.uncompressed_length);
        assert_eq!(e.cell, 1, "the cut blocks stay in the leaf");
    }
    let total: u32 =
        r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).map(|e| e.count).sum();
    assert_eq!(total, 3, "every entity is in exactly one block");
    r.read_blocks(&hot).expect("every hot block is readable under the ceiling");
}

#[test]
fn a_leaf_that_fits_is_one_block_per_kind_as_before() {
    let bytes = big_leaf(1);
    let r = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("opens");
    let hot = r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).count();
    assert_eq!(hot, 1);
}
