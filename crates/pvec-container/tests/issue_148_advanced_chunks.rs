//! The advanced curve set as K chunks (issue pvec#148): each chunk is a record block and its five arenas,
//! sized in bytes, and a composite and everything it names are in one chunk. These are written against
//! the real writer and read back through the real reader.

mod support;

use pvec_container::blocks::advanced::{CurveView, SplineSpec};
use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta};
use pvec_container::{BlockId, Bounds, ContainerReader, ReaderConfig};
use std::collections::BTreeSet;
use support::fixtures::{config, eid};

type Mem = Recording<MemorySource<Vec<u8>>>;

const CHUNK: u32 = 1024;

fn open(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

fn meta(id: u32, min: [f64; 2], max: [f64; 2]) -> EntityMeta {
    EntityMeta { id: eid(id), style: 0, cell: 0, bounds: Bounds { min, max } }
}

fn spline(w: &mut ContainerWriter, id: u32) {
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
    w.push_spline(meta(id, [x0, 0.0], [x0 + 5.0, 5.0]), spec).unwrap();
}

/// 60 splines (ids 1..=60) and two composites: 1000 names 1 and 60, 1001 names 30 and 1000.
fn writer(order: &[u32]) -> ContainerWriter {
    let mut w = ContainerWriter::new(config(&[], None));
    w.advanced_chunk_bytes(CHUNK);
    for id in order {
        spline(&mut w, *id);
    }
    w.push_composite(meta(1000, [10.0, 0.0], [605.0, 5.0]), &[eid(1), eid(60)], None, false).unwrap();
    w.push_composite(meta(1001, [10.0, 0.0], [605.0, 5.0]), &[eid(30), eid(1000)], None, false).unwrap();
    w
}

fn ids() -> Vec<u32> {
    (1..=60).collect()
}

fn records(r: &ContainerReader<Mem>) -> Vec<usize> {
    r.entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.block_kind() == Some(BlockKind::Records))
        .map(|(i, _)| i)
        .collect()
}

fn chunk_ids(r: &ContainerReader<Mem>, chunk: usize) -> Vec<u32> {
    let set = r.advanced(chunk).unwrap().expect("the chunk exists");
    let view = set.view().unwrap();
    (0..view.len()).map(|i| view.record(i).unwrap().id).collect()
}

#[test]
fn a_set_bigger_than_the_chunk_size_is_written_as_several_chunks() {
    let r = open(writer(&ids()).finish().unwrap());
    assert!(r.advanced_chunk_count().unwrap() > 1, "the set is one chunk");
    assert_eq!(r.advanced_chunk_count().unwrap(), records(&r).len());
    let arenas = r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Arena)).count();
    assert!(arenas <= 5 * records(&r).len() && arenas >= records(&r).len());
}

#[test]
fn every_curve_is_in_exactly_one_chunk_and_rows_ascend_inside_a_chunk() {
    let r = open(writer(&ids()).finish().unwrap());
    let mut seen = BTreeSet::new();
    for c in 0..r.advanced_chunk_count().unwrap() {
        let in_chunk = chunk_ids(&r, c);
        assert!(in_chunk.windows(2).all(|w| w[0] < w[1]), "chunk {c} rows are not ascending: {in_chunk:?}");
        for id in in_chunk {
            assert!(seen.insert(id), "{id} is in two chunks");
        }
    }
    assert_eq!(seen.len(), 62);
}

#[test]
fn a_composite_and_everything_it_names_are_in_one_chunk() {
    let r = open(writer(&ids()).finish().unwrap());
    let mut found = false;
    for c in 0..r.advanced_chunk_count().unwrap() {
        let in_chunk = chunk_ids(&r, c);
        if in_chunk.contains(&1001) {
            for id in [1, 30, 60, 1000, 1001] {
                assert!(in_chunk.contains(&id), "{id} is not with 1001 in chunk {c}");
            }
            let set = r.advanced(c).unwrap().unwrap();
            let view = set.view().unwrap();
            let row = in_chunk.iter().position(|i| *i == 1001).unwrap();
            assert!(matches!(view.curve(row).unwrap(), CurveView::Composite(_)));
            found = true;
        }
    }
    assert!(found);
}

#[test]
fn a_chunk_is_a_record_block_and_five_arenas_in_one_cell_and_the_arenas_are_one_run() {
    let r = open(writer(&ids()).finish().unwrap());
    let rec = records(&r);
    for (c, at) in rec.iter().enumerate() {
        let cell = r.entries()[*at].cell;
        assert_eq!(cell as usize, c, "chunk k is in cell k");
        let arenas: Vec<_> = r
            .entries()
            .iter()
            .filter(|e| e.block_kind() == Some(BlockKind::Arena) && e.cell == cell)
            .collect();
        assert_eq!(arenas.iter().map(|e| e.subtype).collect::<Vec<_>>(), vec![1, 2, 3, 4, 5], "chunk {c}");
        for w in arenas.windows(2) {
            let end = w[0].offset + u64::from(w[0].stored_length);
            assert!(w[1].offset >= end && w[1].offset - end < 8, "chunk {c} arenas are not contiguous");
        }
    }
}

#[test]
fn a_chunk_entry_carries_the_union_of_its_curves_bounds() {
    let r = open(writer(&ids()).finish().unwrap());
    for (c, at) in records(&r).iter().enumerate() {
        let b = r.entries()[*at].bounds.expect("a record block has bounds");
        let in_chunk = chunk_ids(&r, c);
        let lows = in_chunk.iter().map(|i| if *i >= 1000 { 10.0 } else { f64::from(*i) * 10.0 });
        let highs = in_chunk.iter().map(|i| if *i >= 1000 { 605.0 } else { f64::from(*i) * 10.0 + 5.0 });
        assert_eq!(b.min[0], lows.fold(f64::MAX, f64::min), "chunk {c}");
        assert_eq!(b.max[0], highs.fold(f64::MIN, f64::max), "chunk {c}");
    }
}

#[test]
fn reading_one_chunk_reads_less_than_the_set() {
    let bytes = writer(&ids()).finish().unwrap();
    let r = open(bytes);
    let before = r.source().bytes_requested();
    r.advanced(0).unwrap().unwrap().view().unwrap();
    let one = r.source().bytes_requested() - before;
    let total: u64 = r
        .entries()
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Records | BlockKind::Arena)))
        .map(|e| u64::from(e.stored_length))
        .sum();
    assert!(one * 2 < total, "one chunk cost {one} of {total}");
}

#[test]
fn a_second_read_of_a_chunk_is_a_cache_hit() {
    let r = open(writer(&ids()).finish().unwrap());
    let a = r.advanced(1).unwrap().unwrap();
    let requests = r.stats().requests;
    let b = r.advanced(1).unwrap().unwrap();
    assert_eq!(r.stats().requests, requests);
    assert_eq!(a.view().unwrap().len(), b.view().unwrap().len());
}

#[test]
fn a_chunk_past_the_end_is_none_and_a_file_without_curves_has_no_chunks() {
    let r = open(writer(&ids()).finish().unwrap());
    let n = r.advanced_chunk_count().unwrap();
    assert!(r.advanced(n).unwrap().is_none());
    let mut w = ContainerWriter::new(config(&[], None));
    w.push_line(meta(1, [0.0, 0.0], [1.0, 1.0]), [0.0, 0.0], [1.0, 1.0]).unwrap();
    let plain = open(w.finish().unwrap());
    assert_eq!(plain.advanced_chunk_count().unwrap(), 0);
    assert!(plain.advanced(0).unwrap().is_none());
}

#[test]
fn a_composite_bigger_than_the_chunk_size_stays_whole() {
    let mut w = ContainerWriter::new(config(&[], None));
    w.advanced_chunk_bytes(64);
    for id in 1..=10 {
        spline(&mut w, id);
    }
    let segs: Vec<_> = (1..=10).map(eid).collect();
    w.push_composite(meta(500, [10.0, 0.0], [105.0, 5.0]), &segs, None, false).unwrap();
    let r = open(w.finish().unwrap());
    let mut sizes = Vec::new();
    for c in 0..r.advanced_chunk_count().unwrap() {
        sizes.push(chunk_ids(&r, c).len());
    }
    assert!(sizes.contains(&11), "{sizes:?}");
}

#[test]
fn the_order_of_the_pushes_is_not_visible() {
    let forward = writer(&ids()).finish().unwrap();
    let mut shuffled = ids();
    shuffled.reverse();
    shuffled.rotate_left(17);
    assert_eq!(forward, writer(&shuffled).finish().unwrap());
}

#[test]
fn a_small_set_is_one_chunk_with_the_default_size() {
    let mut w = ContainerWriter::new(config(&[], None));
    for id in 1..=5 {
        spline(&mut w, id);
    }
    let r = open(w.finish().unwrap());
    assert_eq!(r.advanced_chunk_count().unwrap(), 1);
    assert_eq!(chunk_ids(&r, 0), vec![1, 2, 3, 4, 5]);
    let _ = BlockId(0);
}
