//! The blocks of the advanced chunks a viewport needs (pvec#65 and pvec#192): a caller asks for the block ids of the
//! chunks it wants, puts them in the one `read_blocks` call that fetches its hot blocks, and the planner coalesces
//! the lot, where fetching chunk by chunk makes at least one request for each.

mod support;

use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterOptions};
use pvec_container::{Bounds, ContainerReader, ReadError, ReaderConfig};
use support::*;

/// Six splines, a chunk each (a chunk size of one byte cuts after every component).
fn six_chunks() -> Vec<u8> {
    let mut w = ContainerWriter::with_options(config(&[], None), WriterOptions::default());
    w.advanced_chunk_bytes(1);
    for id in 0..6u32 {
        let x = f64::from(id) * 10.0;
        let meta = EntityMeta {
            id: eid(id),
            style: 0,
            cell: 0,
            bounds: Bounds { min: [x, 0.0], max: [x + 5.0, 5.0] },
        };
        let spec = SplineSpec {
            degree: 1,
            knots: &[0.0, 0.0, 1.0, 1.0],
            x: &[x, x + 5.0],
            y: &[0.0, 5.0],
            weights: None,
            closed: false,
            periodic: false,
        };
        w.push_spline(meta, spec).expect("a spline");
    }
    w.finish().expect("writes")
}

fn open(bytes: Vec<u8>) -> ContainerReader<Recording<MemorySource<Vec<u8>>>> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).expect("opens")
}

#[test]
fn a_chunk_is_its_record_block_and_its_five_arenas() {
    let r = open(six_chunks());
    assert_eq!(r.advanced_chunk_count().unwrap(), 6);
    let ids = r.advanced_chunk_blocks(&[4, 1]).unwrap();
    assert_eq!(ids.len(), 12, "six blocks a chunk, in the order the chunks were named");
    let records = r.advanced_record_blocks().unwrap();
    assert_eq!((ids[0], ids[6]), (records[4], records[1]));
    for six in ids.chunks(6) {
        assert_eq!(r.entries()[six[0].0 as usize].block_kind(), Some(BlockKind::Records));
        for (subtype, id) in six[1..].iter().enumerate() {
            let e = &r.entries()[id.0 as usize];
            assert_eq!((e.block_kind(), e.subtype), (Some(BlockKind::Arena), subtype as u16 + 1));
        }
    }
}

#[test]
fn a_chunk_the_file_does_not_have_is_refused() {
    let r = open(six_chunks());
    assert!(matches!(r.advanced_chunk_blocks(&[0, 6]), Err(ReadError::NoSuchBlock { id: 6, blocks: 6 })));
    assert!(r.advanced_chunk_blocks(&[]).unwrap().is_empty());
}

#[test]
fn the_chunks_of_one_viewport_are_one_request_when_they_are_fetched_together() {
    let bytes = six_chunks();
    let together = open(bytes.clone());
    let ids = together.advanced_chunk_blocks(&[0, 1, 2, 3]).unwrap();
    together.read_blocks(&ids).unwrap();
    let one = together.source().requests().len();

    let apart = open(bytes);
    for c in 0..4 {
        apart.advanced(c).unwrap().expect("a chunk");
    }
    let four = apart.source().requests().len();
    assert!(one < four, "{one} requests together, {four} chunk by chunk");
}

#[test]
fn the_chunk_table_is_worked_out_once_and_not_for_every_chunk_asked_for() {
    // 2,000 chunks, 12,000 blocks: working the table out again for each of 6,000 calls costs seconds, and
    // looking it up costs milliseconds, so the bound is far from both
    let mut w = ContainerWriter::with_options(config(&[], None), WriterOptions::default());
    w.advanced_chunk_bytes(1);
    for id in 0..2000u32 {
        let x = f64::from(id);
        let meta = EntityMeta {
            id: eid(id),
            style: 0,
            cell: 0,
            bounds: Bounds { min: [x, 0.0], max: [x + 1.0, 1.0] },
        };
        let spec = SplineSpec {
            degree: 1,
            knots: &[0.0, 0.0, 1.0, 1.0],
            x: &[x, x + 1.0],
            y: &[0.0, 1.0],
            weights: None,
            closed: false,
            periodic: false,
        };
        w.push_spline(meta, spec).expect("a spline");
    }
    let r = open(w.finish().expect("writes"));
    let start = std::time::Instant::now();
    for i in 0..6000usize {
        assert_eq!(r.advanced_chunk_blocks(&[i % 2000]).unwrap().len(), 6);
    }
    let took = start.elapsed();
    assert!(took.as_millis() < 150, "6,000 lookups took {took:?}");
}
