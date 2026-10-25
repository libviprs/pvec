//! The advanced curve set as chunks, seen from the stream layer (issue pvec#148): `scan_advanced` numbers
//! rows per chunk, `advanced_rows` groups by chunk, and the spatial directory reports the chunks' bounds.

mod support;

use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_stream::{
    Aabb, DuplicationPolicy, EntityBox, PayloadRef, PlacedEntity, SpatialDirectory, advanced_rows,
    build_directory, gather, payload_bytes, place, scan_advanced, scan_payloads,
};
use std::collections::HashMap;
use support::fixtures::*;

type Mem = Recording<MemorySource<Vec<u8>>>;

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/148".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

fn open(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

fn spline(w: &mut ContainerWriter, id: u32, x: f64) -> Aabb {
    let bounds = Aabb::new([x, 100.0], [x + 50.0, 150.0]).unwrap();
    let meta = EntityMeta { id: eid(id), style: 0, cell: 0, bounds: to_bounds(bounds) };
    let spec = pvec_container::blocks::advanced::SplineSpec {
        degree: 1,
        knots: &[0.0, 0.0, 1.0, 1.0],
        x: &[x, x + 50.0],
        y: &[100.0, 150.0],
        weights: None,
        closed: false,
        periodic: false,
    };
    w.push_spline(meta, spec).unwrap();
    bounds
}

/// Lines in a directory, three splines on the left (9000..9002), three on the right (9100..9102), and a
/// composite (9200) of 9000 and 9100 across the middle. A chunk size of one byte makes every component
/// its own chunk: five chunks, the composite's with three curves.
fn build() -> (Vec<u8>, Vec<(u32, Aabb)>, Vec<Fx>) {
    let lines = rooms(3, 2, 200, 2000.0);
    let boxes: Vec<EntityBox> = lines.iter().map(|e| EntityBox { id: e.id, bounds: e.bounds() }).collect();
    let built = build_directory(&boxes).unwrap();
    let mut w = ContainerWriter::new(config());
    w.advanced_chunk_bytes(1);
    for (e, cell) in lines.iter().zip(built.cells.iter()) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds()) };
        w.push_line(meta, e.points[0], e.points[1]).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    let mut adv = Vec::new();
    for i in 0..3 {
        adv.push((9000 + i, spline(&mut w, 9000 + i, 100.0 + 200.0 * f64::from(i))));
        adv.push((9100 + i, spline(&mut w, 9100 + i, 3100.0 + 200.0 * f64::from(i))));
    }
    let both = Aabb::new([100.0, 100.0], [3150.0, 150.0]).unwrap();
    let meta = EntityMeta { id: eid(9200), style: 0, cell: 0, bounds: to_bounds(both) };
    w.push_composite(meta, &[eid(9000), eid(9100)], None, false).unwrap();
    adv.push((9200, both));
    (w.finish().unwrap(), adv, lines)
}

fn chunk_records(r: &ContainerReader<Mem>) -> Vec<BlockId> {
    r.entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.block_kind() == Some(BlockKind::Records))
        .map(|(i, _)| BlockId(i as u32))
        .collect()
}

#[test]
fn scan_advanced_numbers_rows_per_chunk_and_points_at_the_chunks_record_block() {
    let (bytes, _, _) = build();
    let r = open(bytes);
    let chunks = chunk_records(&r);
    assert_eq!(chunks.len(), 5, "9001, 9002, 9101, 9102 and the composite with its two segments");
    let slots = scan_advanced(&r).unwrap();
    assert_eq!(slots.len(), 7);
    for s in &slots {
        assert!(chunks.contains(&s.payload.block_id), "{s:?} names a block that is not a record block");
    }
    let composite = slots.iter().find(|s| s.id.index() == 9200).unwrap();
    for id in [9000, 9100] {
        let seg = slots.iter().find(|s| s.id.index() == id).unwrap();
        assert_eq!(seg.payload.block_id, composite.payload.block_id, "{id} is not with its composite");
    }
    let mut rows: HashMap<BlockId, Vec<u32>> = HashMap::new();
    for s in &slots {
        rows.entry(s.payload.block_id).or_default().push(s.payload.slot);
    }
    for (_, mut v) in rows {
        v.sort_unstable();
        assert_eq!(v, (0..v.len() as u32).collect::<Vec<_>>(), "rows are numbered from 0 in every chunk");
    }
}

#[test]
fn advanced_rows_come_grouped_by_chunk_and_a_viewport_names_only_the_chunks_it_touches() {
    let (bytes, adv, _) = build();
    let r = open(bytes);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let lines = rooms(3, 2, 200, 2000.0);
    let at: HashMap<_, PayloadRef> = scan_payloads(&r).unwrap().iter().map(|s| (s.id, s.payload)).collect();
    let mut placed: Vec<PlacedEntity> =
        lines.iter().map(|e| PlacedEntity { id: e.id, bounds: e.bounds(), payload: at[&e.id] }).collect();
    let adv_at: HashMap<_, PayloadRef> = scan_advanced(&r).unwrap().iter().map(|s| (s.id, s.payload)).collect();
    for (id, b) in &adv {
        placed.push(PlacedEntity { id: eid(*id), bounds: *b, payload: adv_at[&eid(*id)] });
    }
    let p = place(&dir, &placed, DuplicationPolicy::default(), payload_bytes(r.entries())).unwrap();

    let left = bx(0.0, 0.0, 700.0, 600.0);
    let groups = advanced_rows(&r, &gather(&dir, &p.refs, &left)).unwrap();
    let named: Vec<usize> = groups.iter().map(|(chunk, _)| *chunk).collect();
    assert!(named.windows(2).all(|w| w[0] < w[1]), "ascending by chunk: {named:?}");
    assert!(named.len() < 5, "the left viewport must not name every chunk: {named:?}");
    let composite_chunk = chunk_records(&r)
        .iter()
        .position(|b| *b == adv_at[&eid(9200)].block_id)
        .unwrap();
    assert!(named.contains(&composite_chunk), "the composite reaches the left");
    let right_only = chunk_records(&r).iter().position(|b| *b == adv_at[&eid(9102)].block_id).unwrap();
    assert!(!named.contains(&right_only), "9102 is far to the right");
    let far = bx(1500.0, 3500.0, 2000.0, 3900.0);
    assert!(advanced_rows(&r, &gather(&dir, &p.refs, &far)).unwrap().is_empty());
}

#[test]
fn the_directory_reports_the_union_of_the_chunks_bounds_and_each_chunk_its_own() {
    let (bytes, _, _) = build();
    let r = open(bytes);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    assert_eq!(dir.advanced_bounds(), Some(bx(100.0, 100.0, 3550.0, 150.0)));
    let per_chunk: Vec<Aabb> = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Records))
        .map(|e| Aabb::from_container(e.bounds.unwrap()).unwrap())
        .collect();
    assert_eq!(per_chunk.len(), 5);
    assert!(per_chunk.iter().any(|b| *b != dir.advanced_bounds().unwrap()), "the chunks differ");
}
