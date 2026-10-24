//! The reference arrays as a container block, and the advanced set split by reference (issue pvec#131).
//!
//! Every artifact is written through the real container writer and read back through the real reader.
//! The block ids of the hot blocks are fixed before the reference block exists (hot blocks sort first),
//! so a writer places in two passes: write the payloads, scan them, place, write again with the block.

mod support;

use pvec_container::directory::{BlockEntry, BlockKind};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, CellRefs, DuplicationPolicy, EntityBox, OWNERSHIP_RULES, OwnershipError, PayloadRef, PlacedEntity,
    SpatialDirectory, advanced_rows, build_directory, gather, payload_bytes, place, read_viewport,
    scan_advanced, scan_payloads,
};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use support::fixtures::*;

type Mem = Recording<MemorySource<Vec<u8>>>;

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/131".into(),
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

fn push_line(w: &mut ContainerWriter, e: &Fx, cell: u32) {
    let meta = EntityMeta { id: e.id, style: 0, cell, bounds: to_bounds(e.bounds()) };
    w.push_line(meta, e.points[0], e.points[1]).unwrap();
}

/// Lines in a directory of their own, and optionally a reference block and a few advanced curves.
struct Build {
    lines: Vec<Fx>,
    cells: Vec<u32>,
    dir_nodes: u32,
    dir_bounds: Aabb,
    dir_bytes: Vec<u8>,
}

fn build(lines: Vec<Fx>) -> Build {
    let boxes: Vec<EntityBox> = lines.iter().map(|e| EntityBox { id: e.id, bounds: e.bounds() }).collect();
    let built = build_directory(&boxes).unwrap();
    Build {
        cells: built.cells.iter().map(|c| c.get()).collect(),
        dir_nodes: built.nodes,
        dir_bounds: built.bounds,
        dir_bytes: built.bytes,
        lines,
    }
}

impl Build {
    fn writer(&self) -> ContainerWriter {
        let mut w = ContainerWriter::new(config());
        for (e, cell) in self.lines.iter().zip(&self.cells) {
            push_line(&mut w, e, *cell);
        }
        w.set_spatial_directory(self.dir_nodes, to_bounds(self.dir_bounds), self.dir_bytes.clone()).unwrap();
        w
    }

    fn placed(&self, r: &ContainerReader<Mem>) -> Vec<PlacedEntity> {
        let at: HashMap<EntityId, PayloadRef> =
            scan_payloads(r).unwrap().iter().map(|s| (s.id, s.payload)).collect();
        self.lines.iter().map(|e| PlacedEntity { id: e.id, bounds: e.bounds(), payload: at[&e.id] }).collect()
    }
}

/// The fixture: 400 lines of two rooms side by side, enough for several cells that overlap.
fn lines() -> Vec<Fx> {
    rooms(3, 2, 200, 2000.0)
}

/// Two passes: the file without the block, its placement, the file with it.
struct Pair {
    plain: Vec<u8>,
    with: Vec<u8>,
    refs: CellRefs,
    placed: Vec<PlacedEntity>,
}

fn pair(b: &Build) -> Pair {
    let plain = b.writer().finish().unwrap();
    let r = open(plain.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let placed = b.placed(&r);
    let p = place(&dir, &placed, DuplicationPolicy::default(), payload_bytes(r.entries())).unwrap();
    let mut w = b.writer();
    w.set_cell_refs(p.refs.cells() as u32, p.refs.encode()).unwrap();
    Pair { plain, with: w.finish().unwrap(), refs: p.refs, placed }
}

fn refs_entries(entries: &[BlockEntry]) -> Vec<(usize, BlockEntry)> {
    entries.iter().copied().enumerate().filter(|(_, e)| e.block_kind() == Some(BlockKind::CellRefs)).collect()
}

fn rule(r: Result<impl std::fmt::Debug, OwnershipError>) -> &'static str {
    r.unwrap_err().rule()
}

// ---------------------------------------------------------------- the block

#[test]
fn the_reference_arrays_are_one_block_of_the_new_kind_that_stores_the_encoded_bytes() {
    assert_eq!(BlockKind::CellRefs.id(), 6);
    assert_eq!(BlockKind::CellRefs.name(), "refs");
    assert_eq!(BlockKind::from_id(6), Some(BlockKind::CellRefs));
    let b = build(lines());
    let p = pair(&b);
    let r = open(p.with.clone());
    let found = refs_entries(r.entries());
    assert_eq!(found.len(), 1);
    let (id, e) = found[0];
    assert_eq!(e.cell, 0, "the arrays are not in a cell");
    assert_eq!(e.count as usize, p.refs.cells());
    assert_eq!(e.flags & 1, 0, "a reader that does not know the kind skips it");
    assert_eq!(e.codec, 0, "stored as it is");
    assert_eq!(r.read_block(BlockId(id as u32)).unwrap().as_bytes(), p.refs.encode().as_slice());
    assert_eq!(id, r.entries().len() - 1, "after every block of a lower kind");
}

#[test]
fn adding_the_block_moves_no_hot_block() {
    let b = build(lines());
    let p = pair(&b);
    let (plain, with) = (open(p.plain.clone()), open(p.with.clone()));
    let hot = |r: &ContainerReader<Mem>| -> Vec<(usize, u32, u32)> {
        r.entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot))
            .map(|(i, e)| (i, e.cell, e.count))
            .collect()
    };
    assert_eq!(hot(&plain), hot(&with), "the references name block ids, so those ids must not change");
    assert_eq!(scan_payloads(&plain).unwrap(), scan_payloads(&with).unwrap());
}

#[test]
fn opening_the_file_and_the_directory_reads_none_of_the_reference_block() {
    let b = build(lines());
    let p = pair(&b);
    let r = open(p.with.clone());
    let (_, e) = refs_entries(r.entries())[0];
    SpatialDirectory::open(&r).unwrap().unwrap();
    assert!(
        !r.source().touched(e.offset, u64::from(e.stored_length)),
        "the reference block was read before anything asked for it"
    );
    let opened = CellRefs::open(&r).unwrap().expect("the file has the block");
    assert_eq!(opened, p.refs);
    assert!(r.source().touched(e.offset, u64::from(e.stored_length)), "and now it was read");
}

#[test]
fn a_file_without_the_block_has_no_arrays() {
    let b = build(lines());
    let p = pair(&b);
    let r = open(p.plain);
    assert!(CellRefs::open(&r).unwrap().is_none());
    assert!(CellRefs::find(r.entries()).unwrap().is_none());
}

#[test]
fn arrays_read_back_from_the_file_give_the_same_viewport_as_the_ones_placed() {
    let b = build(lines());
    let p = pair(&b);
    let r = open(p.with.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let refs = CellRefs::open(&r).unwrap().unwrap();
    for (cx, cy, size) in [(1000.0, 1000.0, 600.0), (3000.0, 1000.0, 2000.0), (2000.0, 2000.0, 4000.0)] {
        let q = bx(cx - size / 2.0, cy - size / 2.0, cx + size / 2.0, cy + size / 2.0);
        let (from_file, in_memory) = (gather(&dir, &refs, &q), gather(&dir, &p.refs, &q));
        assert_eq!(from_file, in_memory);
        let mut seen = BTreeSet::new();
        let n = read_viewport(&r, &from_file, |id, _, _| assert!(seen.insert(id), "{id:?} twice")).unwrap();
        assert_eq!(n, seen.len());
        for e in &p.placed {
            if e.bounds.intersects(&q) {
                assert!(seen.contains(&e.id), "{:?} is in the viewport and was not yielded", e.id);
            }
        }
    }
}

#[test]
fn a_block_whose_count_is_not_its_cell_count_is_refused() {
    let b = build(lines());
    let p = pair(&b);
    let mut w = b.writer();
    w.set_cell_refs(p.refs.cells() as u32 + 1, p.refs.encode()).unwrap();
    let r = open(w.finish().unwrap());
    assert_eq!(rule(CellRefs::open(&r)), "OWN-11");
}

#[test]
fn a_block_with_a_bad_layout_is_refused_by_the_layout_rules() {
    let b = build(lines());
    let p = pair(&b);
    let mut bytes = p.refs.encode();
    bytes[0] = b'X';
    let mut w = b.writer();
    w.set_cell_refs(p.refs.cells() as u32, bytes).unwrap();
    assert_eq!(rule(CellRefs::open(&open(w.finish().unwrap()))), "OWN-02");
    let mut short = p.refs.encode();
    short.truncate(short.len() - 3);
    let mut w = b.writer();
    w.set_cell_refs(p.refs.cells() as u32, short).unwrap();
    assert_eq!(rule(CellRefs::open(&open(w.finish().unwrap()))), "OWN-01");
}

#[test]
fn two_reference_blocks_are_refused() {
    let b = build(lines());
    let p = pair(&b);
    let r = open(p.with);
    let mut entries = r.entries().to_vec();
    let again = entries[refs_entries(&entries)[0].0];
    entries.push(again);
    assert_eq!(rule(CellRefs::find(&entries)), "OWN-10");
}

#[test]
fn the_new_rules_and_the_block_are_documented() {
    assert_eq!(OWNERSHIP_RULES.len(), 11);
    let doc = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md")).unwrap();
    for (rule, meaning) in OWNERSHIP_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/STREAMING.md lacks {rule}");
    }
    assert!(!doc.contains("A reference array is not a container block yet"), "section 2.3 is out of date");
    let container = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONTAINER.md")).unwrap();
    assert!(container.contains("| 6 | refs |"), "docs/CONTAINER.md lacks the block kind");
}

// ---------------------------------------------------------------- the advanced set

/// A two point spline between `a` and `b`, as a curve the writer accepts.
fn spline(w: &mut ContainerWriter, id: u32, a: [f64; 2], b: [f64; 2]) {
    let bounds = Aabb::new([a[0].min(b[0]), a[1].min(b[1])], [a[0].max(b[0]), a[1].max(b[1])]).unwrap();
    let meta = EntityMeta { id: eid(id), style: 0, cell: 0, bounds: to_bounds(bounds) };
    let spec = pvec_container::blocks::advanced::SplineSpec {
        degree: 1,
        knots: &[0.0, 0.0, 1.0, 1.0],
        x: &[a[0], b[0]],
        y: &[a[1], b[1]],
        weights: None,
        closed: false,
        periodic: false,
    };
    w.push_spline(meta, spec).unwrap();
}

/// Lines in four cells' worth of rooms and three advanced curves: 9000 on the left, 9001 on the right and
/// 9002, a composite of both, across the middle. They are written out of order on purpose.
fn with_advanced(b: &Build) -> (Vec<u8>, Vec<(u32, Aabb)>) {
    let mut w = b.writer();
    let left = ([100.0, 100.0], [900.0, 500.0]);
    let right = ([3100.0, 100.0], [3900.0, 500.0]);
    spline(&mut w, 9001, right.0, right.1);
    spline(&mut w, 9000, left.0, left.1);
    let both = Aabb::new([100.0, 100.0], [3900.0, 500.0]).unwrap();
    let meta = EntityMeta { id: eid(9002), style: 0, cell: 0, bounds: to_bounds(both) };
    w.push_composite(meta, &[eid(9000), eid(9001)], None, false).unwrap();
    let boxes = vec![
        (9000, Aabb::new(left.0, left.1).unwrap()),
        (9001, Aabb::new(right.0, right.1).unwrap()),
        (9002, both),
    ];
    (w.finish().unwrap(), boxes)
}

#[test]
fn the_advanced_set_stays_one_set_and_its_records_are_found_by_row() {
    let b = build(lines());
    let (bytes, _) = with_advanced(&b);
    let r = open(bytes);
    let slots = scan_advanced(&r).unwrap();
    assert_eq!(slots.iter().map(|s| s.id.index()).collect::<Vec<_>>(), vec![9000, 9001, 9002], "id order");
    let records = r.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Records)).unwrap();
    for (row, s) in slots.iter().enumerate() {
        assert_eq!(s.payload, PayloadRef { block_id: BlockId(records as u32), slot: row as u32 });
        assert_eq!(s.cell, 0);
    }
}

#[test]
fn a_cell_references_the_advanced_curves_that_reach_it_and_a_viewport_names_the_rows_it_needs() {
    let b = build(lines());
    let (bytes, boxes) = with_advanced(&b);
    let r = open(bytes);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let mut placed = b.placed(&r);
    let at: HashMap<EntityId, PayloadRef> =
        scan_advanced(&r).unwrap().iter().map(|s| (s.id, s.payload)).collect();
    for (id, bounds) in &boxes {
        placed.push(PlacedEntity { id: eid(*id), bounds: *bounds, payload: at[&eid(*id)] });
    }
    let p = place(&dir, &placed, DuplicationPolicy::default(), payload_bytes(r.entries())).unwrap();

    let left = bx(0.0, 0.0, 1000.0, 600.0);
    let rows = advanced_rows(&r, &gather(&dir, &p.refs, &left)).unwrap();
    assert_eq!(rows, vec![0, 2], "the left curve and the composite, not the right curve");
    let right = bx(3000.0, 0.0, 4000.0, 600.0);
    assert_eq!(advanced_rows(&r, &gather(&dir, &p.refs, &right)).unwrap(), vec![1, 2]);
    let far = bx(1500.0, 3500.0, 2000.0, 3900.0);
    let g = gather(&dir, &p.refs, &far);
    assert!(advanced_rows(&r, &g).unwrap().is_empty(), "nothing of the advanced set is needed over there");

    // reading the hot payloads of a viewport that also lists advanced rows visits the hot ones only
    let g = gather(&dir, &p.refs, &left);
    let mut visited = 0;
    let n = read_viewport(&r, &g, |_, _, _| visited += 1).unwrap();
    assert_eq!(n, visited);
    assert_eq!(n + 2, g.refs.len());
}

#[test]
fn a_reference_to_a_record_that_does_not_exist_is_refused() {
    let b = build(lines());
    let (bytes, _) = with_advanced(&b);
    let r = open(bytes);
    let records = r.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Records)).unwrap();
    let g = pvec_stream::ViewportRefs {
        raw: 1,
        refs: vec![PayloadRef { block_id: BlockId(records as u32), slot: 3 }],
    };
    assert_eq!(rule(advanced_rows(&r, &g)), "OWN-08");
    assert_eq!(rule(read_viewport(&r, &g, |_, _, _| {})), "OWN-08");
}
