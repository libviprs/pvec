//! A chunk of the advanced curve set as one contiguous run (issue pvec#159): its record block and its five
//! arenas are adjacent in the file, so a reader that wants the chunk asks for one range and not two.
//! These are written against the real writer and read back through the real reader.

mod support;

use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta};
use pvec_container::{Bounds, ContainerReader, ReaderConfig};
use support::fixtures::{config, eid};

type Mem = Recording<MemorySource<Vec<u8>>>;

fn open(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

/// 60 splines in chunks of 1 KiB, so several chunks.
fn chunked() -> Vec<u8> {
    let mut w = ContainerWriter::new(config(&[], None));
    w.advanced_chunk_bytes(1024);
    for id in 1..=60u32 {
        let x0 = f64::from(id) * 10.0;
        let spec = SplineSpec {
            degree: 1,
            knots: &[0.0, 0.0, 1.0, 1.0],
            x: &[x0, x0 + 5.0],
            y: &[0.0, 5.0],
            weights: None,
            closed: false,
            periodic: false,
        };
        let meta = EntityMeta {
            id: eid(id),
            style: 0,
            cell: 0,
            bounds: Bounds { min: [x0, 0.0], max: [x0 + 5.0, 5.0] },
        };
        w.push_spline(meta, spec).unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn the_six_blocks_of_a_chunk_are_consecutive_entries_record_block_first() {
    let r = open(chunked());
    let chunks = r.advanced_chunk_count().unwrap();
    assert!(chunks > 2, "the fixture has {chunks} chunks");
    let entries = r.entries();
    for (c, six) in r.advanced_record_blocks().unwrap().iter().enumerate() {
        let at = six.0 as usize;
        assert_eq!(entries[at].block_kind(), Some(BlockKind::Records), "chunk {c}");
        let run = &entries[at + 1..at + 6];
        assert!(run.iter().all(|e| e.block_kind() == Some(BlockKind::Arena)), "chunk {c}: {run:?}");
        assert_eq!(run.iter().map(|e| e.subtype).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5], "chunk {c}");
        assert!(run.iter().all(|e| e.cell == entries[at].cell), "chunk {c} is in one cell");
    }
}

#[test]
fn the_six_blocks_of_a_chunk_have_no_more_than_padding_between_them() {
    let r = open(chunked());
    let entries = r.entries();
    for (c, rec) in r.advanced_record_blocks().unwrap().iter().enumerate() {
        let at = rec.0 as usize;
        assert_eq!(entries[at].block_kind(), Some(BlockKind::Records), "chunk {c}");
        assert_eq!(entries[at + 5].block_kind(), Some(BlockKind::Arena), "chunk {c}");
        for w in entries[at..at + 6].windows(2) {
            let end = w[0].offset + u64::from(w[0].stored_length);
            assert!(w[1].offset >= end && w[1].offset - end < 8, "chunk {c} is not one run");
        }
    }
}

#[test]
fn reading_a_chunk_is_one_range_of_about_its_own_size_and_no_other_chunks_bytes() {
    let r = open(chunked());
    for c in 0..r.advanced_chunk_count().unwrap() {
        let first = r.advanced_record_blocks().unwrap()[c].0 as usize;
        let own: u64 = (first..first + 6).map(|i| u64::from(r.entries()[i].stored_length)).sum();
        let (before, bytes0) = (r.source().requests().len(), r.source().bytes_requested());
        r.advanced(c).unwrap().expect("the chunk").view().unwrap();
        assert_eq!(r.source().requests().len() - before, 1, "chunk {c}");
        let read = r.source().bytes_requested() - bytes0;
        assert!(read >= own && read < own + 48, "chunk {c}: {read} bytes for {own} of blocks");
    }
}

#[test]
fn blocks_that_are_not_advanced_keep_the_order_of_kind_then_cell_then_subtype() {
    let r = open(chunked());
    let rest: Vec<(u16, u32, u16)> = r
        .entries()
        .iter()
        .filter(|e| !matches!(e.block_kind(), Some(BlockKind::Records | BlockKind::Arena)))
        .map(|e| (e.kind, e.cell, e.subtype))
        .collect();
    assert!(rest.windows(2).all(|w| w[0] < w[1]), "{rest:?}");
}
