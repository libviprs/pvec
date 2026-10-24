//! The spatial directory over source-space bounds (issue P5.1, pvec#10).
//!
//! Each test is named for the acceptance criterion it holds the directory to. The fixtures are generated
//! here from a seeded splitmix64 (`rooms`), because the P1.6 fixtures live in a spike workspace that
//! this one cannot depend on; the shape is the same (a grid of rooms of 400 plan-like entities).

use pvec_container::directory::{BlockEntry, BlockKind};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, ContainerReader, LengthUnit, Limits, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, BuiltDirectory, CELL_CAPACITY, CellId, DIRECTORY_RULES, DirectoryError, EntityBox, FANOUT,
    HEADER_LEN, MAX_HEIGHT, NODE_LEN, SPLIT_POLICY, SpatialDirectory, SplitPolicy, build_directory,
};
use std::collections::BTreeSet;
use std::fs;

// ---------------------------------------------------------------- fixtures

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A value in `[0, 1)` from the top 53 bits.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn bx(a: f64, b: f64, c: f64, d: f64) -> Aabb {
    Aabb::new([a, b], [c, d]).unwrap()
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

/// `n` entities on a grid of `side` x `side` rooms, each room `room` units wide. Most are small, one in
/// twenty is a wall as long as the room, and every room gets the same share, which is the plan-like mix
/// of `million` in `docs/spikes/00-fixtures.md`.
fn rooms(seed: u64, side: u32, per_room: u32, room: f64) -> Vec<EntityBox> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity((side * side * per_room) as usize);
    for ry in 0..side {
        for rx in 0..side {
            for _ in 0..per_room {
                let (ox, oy) = (f64::from(rx) * room, f64::from(ry) * room);
                let wall = rng.next() % 20 == 0;
                let (w, h) = if wall {
                    if rng.next() % 2 == 0 { (room * 0.9, 0.0) } else { (0.0, room * 0.9) }
                } else {
                    (rng.unit() * room * 0.04, rng.unit() * room * 0.04)
                };
                let x = ox + rng.unit() * (room - w);
                let y = oy + rng.unit() * (room - h);
                // three decimals, like drawing units
                let r = |v: f64| (v * 1000.0).round() / 1000.0;
                let id = eid(out.len() as u32);
                out.push(EntityBox { id, bounds: bx(r(x), r(y), r(x + w), r(y + h)) });
            }
        }
    }
    out
}

fn shuffled<T: Clone>(items: &[T], seed: u64) -> Vec<T> {
    let mut v = items.to_vec();
    let mut rng = Rng(seed);
    for i in (1..v.len()).rev() {
        let j = (rng.next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

fn config(version: &str) -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: version.into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

/// An artifact of `entities` as lines, with the directory `tamper` leaves behind. `build` is the real
/// directory, and `tamper` may change its bytes before the writer stores them.
fn artifact_with(
    entities: &[EntityBox],
    version: &str,
    tamper: impl FnOnce(&mut Vec<u8>),
) -> (Vec<u8>, BuiltDirectory) {
    let built = build_directory(entities).expect("directory");
    let mut w = ContainerWriter::new(config(version));
    for (e, cell) in entities.iter().zip(&built.cells) {
        let (lo, hi) = (e.bounds.min(), e.bounds.max());
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, lo, hi).unwrap();
    }
    let mut bytes = built.bytes.clone();
    tamper(&mut bytes);
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), bytes).unwrap();
    (w.finish().unwrap(), built)
}

fn artifact(entities: &[EntityBox]) -> (Vec<u8>, BuiltDirectory) {
    artifact_with(entities, "t/1", |_| {})
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn reader(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

fn directory_block(r: &ContainerReader<Mem>) -> Vec<u8> {
    let id = SpatialDirectory::find(r.entries()).unwrap().expect("a spatial block");
    r.read_block(id).unwrap().as_bytes().to_vec()
}

fn rule(r: Result<impl std::fmt::Debug, DirectoryError>) -> &'static str {
    r.expect_err("the directory must be refused").rule()
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: f64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn node(i: usize) -> usize {
    HEADER_LEN + NODE_LEN * i
}

/// Node field offsets inside a node record.
const MAX_X: usize = 16;
const FIRST: usize = 32;
const COUNT: usize = 36;
const LEVEL: usize = 44;

fn parse(bytes: &[u8], entries: &[BlockEntry]) -> Result<SpatialDirectory, DirectoryError> {
    SpatialDirectory::parse(bytes, entries, &Limits::DEFAULT)
}

/// 2,500 entities: 40 cells, so a tree of three levels, and the entries of its artifact.
fn three_levels() -> (Vec<u8>, Vec<BlockEntry>, Vec<u8>) {
    let entities = rooms(7, 5, 100, 1000.0);
    let (bytes, built) = artifact(&entities);
    let r = reader(bytes);
    let entries = r.entries().to_vec();
    assert_eq!(built.nodes, 40 + 2 + 1, "40 leaves, 2 nodes above them, 1 root");
    (built.bytes.clone(), entries, directory_block(&r))
}

// ---------------------------------------------------------------- the criteria

#[test]
fn open_reads_a_million_entity_directory_in_at_most_four_range_reads_and_under_2_mib() {
    // `rooms-1m`: 50 x 50 rooms of 400 entities over 100,000 x 100,000, seed 1, the shape of `million`
    let entities = rooms(1, 50, 400, 2000.0);
    assert_eq!(entities.len(), 1_000_000);
    let (bytes, built) = artifact(&entities);
    let source = Recording::new(MemorySource::new(bytes));
    let r = ContainerReader::open(source, ReaderConfig::default()).unwrap();
    let dir = SpatialDirectory::open(&r).unwrap().expect("a directory");
    let requests = r.source().requests();
    let block: u64 = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::SpatialDirectory))
        .map(|e| u64::from(e.stored_length))
        .sum();
    eprintln!("rooms-1m: {} range reads {:?}, spatial block {block} bytes", requests.len(), requests);
    assert!(requests.len() <= 4, "{} range reads: {requests:?}", requests.len());
    assert!(block < 2 << 20, "the directory block is {block} bytes");
    assert_eq!(block, built.bytes.len() as u64, "stored as it is");
    assert_eq!(dir.entities(), 1_000_000);
    assert_eq!(dir.cells(), 1_000_000usize.div_ceil(CELL_CAPACITY as usize));
}

#[test]
fn queries_are_exact_against_a_brute_force_scan_on_100000_entities() {
    // the straddler shape: rooms plus four entities that each span the drawing
    let mut entities = rooms(2, 16, 390, 2000.0);
    let extent = 16.0 * 2000.0;
    for (i, (a, b, c, d)) in [
        (0.0, extent / 2.0, extent, extent / 2.0),
        (extent / 2.0, 0.0, extent / 2.0, extent),
        (0.0, 0.0, extent, extent),
        (1.0, 1.0, extent - 1.0, extent - 1.0),
    ]
    .into_iter()
    .enumerate()
    {
        let id = eid(entities.len() as u32);
        let _ = i;
        entities.push(EntityBox { id, bounds: bx(a, b, c, d) });
    }
    assert!(entities.len() >= 62_000);
    let (bytes, built) = artifact(&entities);
    let r = reader(bytes);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let mut rng = Rng(99);
    let mut queries: Vec<Aabb> = Vec::new();
    for k in 0..300 {
        let (w, h) = match k % 5 {
            0 => (extent, extent),
            1 => (extent / 4.0, extent / 4.0),
            2 => (extent / 16.0, extent / 16.0),
            3 => (extent / 200.0, extent / 200.0),
            _ => (0.0, 0.0),
        };
        let (x, y) = (rng.unit() * extent, rng.unit() * extent);
        queries.push(bx(x, y, x + w, y + h));
    }
    // outside the drawing, hugging an edge, poking out of it, on a corner
    queries.push(bx(-500.0, -500.0, -1.0, -1.0));
    queries.push(bx(extent + 1.0, 0.0, extent + 9.0, 5.0));
    queries.push(bx(-10.0, -10.0, 10.0, 10.0));
    queries.push(bx(extent, extent, extent, extent));
    for q in &queries {
        let got: Vec<_> = dir.query(q).collect();
        let cells: BTreeSet<CellId> = got.iter().map(|c| c.id).collect();
        assert_eq!(cells.len(), got.len(), "a cell was yielded twice for {q:?}");
        let mut missing = 0usize;
        for (e, cell) in entities.iter().zip(&built.cells) {
            if e.bounds.intersects(q) && !cells.contains(cell) {
                missing += 1;
            }
        }
        assert_eq!(missing, 0, "{missing} entities that intersect {q:?} are in no cell the query yielded");
        for c in &got {
            assert!(c.bounds.intersects(q), "cell {:?} does not touch the query {q:?}", c.id);
        }
    }
}

#[test]
fn the_directory_block_does_not_depend_on_the_display_scale_or_the_order_of_arrival() {
    let entities = rooms(3, 6, 100, 1000.0);
    let block = |version: &str, input: &[EntityBox]| {
        let (bytes, _) = artifact_with(input, version, |_| {});
        directory_block(&reader(bytes))
    };
    // the artifact declares its display scale only in what the caller writes into the metadata; the
    // directory is built from source-space boxes and has nowhere to read a scale from
    let a = block("display-scale=1:50 zoom=3", &entities);
    let b = block("display-scale=1:5000 zoom=18", &entities);
    assert_eq!(a, b, "two display scales gave two directories");
    let c = block("t/1", &shuffled(&entities, 5));
    assert_eq!(a, c, "the order the entities arrive in changed the directory");
}

#[test]
fn a_cell_whose_bounds_do_not_contain_its_children_is_refused_at_open() {
    let (bytes, entries, _) = three_levels();
    // an internal node that does not contain a child: shrink the root (the last node) on x
    let mut a = bytes.clone();
    let root = node(get32(&a, 8) as usize - 1);
    let min_x = f64::from_le_bytes(a[root..root + 8].try_into().unwrap());
    put64(&mut a, root + MAX_X, min_x);
    assert_eq!(rule(parse(&a, &entries)), "SPD-07");
    // a leaf that does not contain the blocks its cell names: collapse leaf 0 to a point
    let mut b = bytes.clone();
    for off in [16, 24] {
        let lo = f64::from_le_bytes(b[node(0) + off - 16..node(0) + off - 8].try_into().unwrap());
        put64(&mut b, node(0) + off, lo);
    }
    assert_eq!(rule(parse(&b, &entries)), "SPD-10");
    // and the same through `open`, on a file whose directory block was altered before it was written
    let entities = rooms(7, 5, 100, 1000.0);
    let (file, _) = artifact_with(&entities, "t/1", |d| {
        let root = node(get32(d, 8) as usize - 1);
        let min_x = f64::from_le_bytes(d[root..root + 8].try_into().unwrap());
        put64(d, root + MAX_X, min_x);
    });
    let r = reader(file);
    assert_eq!(rule(SpatialDirectory::open(&r)), "SPD-07");
}

#[test]
fn cyclic_or_out_of_range_child_pointers_are_refused_rather_than_traversed() {
    let (bytes, entries, _) = three_levels();
    let n = get32(&bytes, 8) as usize;
    let root = n - 1;
    let level1 = 40; // the first node above the leaves
    // past the end of the nodes
    let mut a = bytes.clone();
    put32(&mut a, node(root) + FIRST, n as u32);
    assert_eq!(rule(parse(&a, &entries)), "SPD-05");
    let mut b = bytes.clone();
    put32(&mut b, node(root) + FIRST, u32::MAX);
    assert_eq!(rule(parse(&b, &entries)), "SPD-05");
    // a child count that runs past the end
    let mut c = bytes.clone();
    put32(&mut c, node(level1) + COUNT, u32::MAX);
    assert_eq!(rule(parse(&c, &entries)), "SPD-05");
    // a node that is its own child, and a node whose child is the root above it: a cycle
    let mut d = bytes.clone();
    put32(&mut d, node(level1) + FIRST, level1 as u32);
    assert_eq!(rule(parse(&d, &entries)), "SPD-05");
    let mut e = bytes.clone();
    put32(&mut e, node(level1) + FIRST, root as u32);
    put32(&mut e, node(level1) + COUNT, 1);
    assert_eq!(rule(parse(&e, &entries)), "SPD-05");
    // a leaf that claims children, and a level that does not step down by one
    let mut f = bytes.clone();
    put32(&mut f, node(0) + COUNT, 1);
    assert_eq!(rule(parse(&f, &entries)), "SPD-06");
    let mut g = bytes.clone();
    g[node(level1) + LEVEL] = 3;
    assert_eq!(rule(parse(&g, &entries)), "SPD-06");
}

#[test]
fn counts_that_would_need_a_huge_allocation_are_refused_before_one_is_made() {
    let (bytes, entries, _) = three_levels();
    // a node count that the block's own length cannot hold
    let mut a = bytes.clone();
    put32(&mut a, 8, u32::MAX);
    assert_eq!(rule(parse(&a, &entries)), "SPD-01");
    // a height past the ceiling
    let mut b = bytes.clone();
    put32(&mut b, 16, MAX_HEIGHT + 1);
    assert_eq!(rule(parse(&b, &entries)), "SPD-02");
    // more entities than P4.8 lets an artifact declare
    let mut c = bytes.clone();
    put32(&mut c, 28, u32::MAX);
    let limit = SpatialDirectory::parse(&c, &entries, &Limits::DEFAULT).unwrap_err();
    assert!(matches!(limit, DirectoryError::Limit(_)), "{limit:?}");
    assert_eq!(limit.rule(), "SPD-03");
    // a block that is not a whole number of nodes, and one that is shorter than its header
    assert_eq!(rule(parse(&bytes[..bytes.len() - 1], &entries)), "SPD-01");
    assert_eq!(rule(parse(&bytes[..10], &entries)), "SPD-01");
    assert_eq!(rule(parse(&[], &entries)), "SPD-01");
}

#[test]
fn query_order_is_ascending_cell_id_and_is_the_same_for_ties() {
    // 640 entities with identical bounds tie on every key, so the order is decided by the entity id:
    // entity i is in cell i / 64 + 1 whatever order the entities arrive in
    let same = |id: u32| EntityBox { id: eid(id), bounds: bx(10.0, 10.0, 20.0, 20.0) };
    let tied: Vec<EntityBox> = (0..640).map(same).collect();
    let mixed = shuffled(&tied, 11);
    for input in [&tied, &mixed] {
        let built = build_directory(input).unwrap();
        for (e, cell) in input.iter().zip(&built.cells) {
            assert_eq!(cell.get(), e.id.index() / CELL_CAPACITY + 1, "entity {:?}", e.id);
        }
    }
    let (bytes, _) = artifact(&mixed);
    let r = reader(bytes);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let all: Vec<u32> = dir.query(&bx(0.0, 0.0, 100.0, 100.0)).map(|c| c.id.get()).collect();
    assert_eq!(all, (1..=10).collect::<Vec<u32>>());
    // touching at one corner counts, and every tied cell is yielded in the same order
    let corner: Vec<u32> = dir.query(&bx(20.0, 20.0, 30.0, 30.0)).map(|c| c.id.get()).collect();
    assert_eq!(corner, all);
    assert_eq!(dir.query(&bx(20.0001, 20.0, 30.0, 30.0)).count(), 0);
    // a spread-out fixture: ascending, and repeatable across a rebuild from shuffled input
    let spread = rooms(4, 8, 60, 500.0);
    let a = artifact(&spread).0;
    let b = artifact(&shuffled(&spread, 8)).0;
    let (da, db) = (
        SpatialDirectory::open(&reader(a)).unwrap().unwrap(),
        SpatialDirectory::open(&reader(b)).unwrap().unwrap(),
    );
    for q in
        [bx(0.0, 0.0, 4000.0, 4000.0), bx(900.0, 900.0, 2100.0, 1500.0), bx(1000.0, 1000.0, 1000.0, 1000.0)]
    {
        let (xs, ys): (Vec<_>, Vec<_>) = (da.query(&q).collect(), db.query(&q).collect());
        assert_eq!(xs, ys);
        assert!(xs.windows(2).all(|w| w[0].id < w[1].id), "cells are not ascending: {xs:?}");
    }
}

#[test]
fn cell_zero_is_not_a_cell() {
    assert!(CellId::new(0).is_none());
    assert_eq!(CellId::new(7).unwrap().get(), 7);
    let (bytes, entries, _) = three_levels();
    // a leaf with cell 0
    let mut a = bytes.clone();
    put32(&mut a, node(0) + FIRST, 0);
    assert_eq!(rule(parse(&a, &entries)), "SPD-09");
    // cells that are not strictly ascending
    let mut b = bytes.clone();
    put32(&mut b, node(1) + FIRST, 1);
    assert_eq!(rule(parse(&b, &entries)), "SPD-09");
    // a hot block in cell 0 beside a directory is entities the directory cannot find
    let entities = rooms(7, 2, 50, 1000.0);
    let built = build_directory(&entities).unwrap();
    let mut w = ContainerWriter::new(config("t/1"));
    for (i, (e, cell)) in entities.iter().zip(&built.cells).enumerate() {
        let cell = if i == 0 { 0 } else { cell.get() };
        let meta = EntityMeta { id: e.id, style: 0, cell, bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    let r = reader(w.finish().unwrap());
    assert_eq!(rule(SpatialDirectory::open(&r)), "SPD-10");
}

#[test]
fn a_directory_that_disagrees_with_the_blocks_is_refused() {
    let (bytes, entries, _) = three_levels();
    // a leaf entity count that is not the blocks' count
    let mut a = bytes.clone();
    let n = get32(&a, node(3) + 40) + 1;
    put32(&mut a, node(3) + 40, n);
    let found = rule(parse(&a, &entries));
    assert!(found == "SPD-08" || found == "SPD-10", "{found}");
    // a leaf whose cell no block has
    let mut b = bytes.clone();
    put32(&mut b, node(39) + FIRST, 9999);
    assert_eq!(rule(parse(&b, &entries)), "SPD-10");
    // fewer blocks than leaves
    assert_eq!(rule(parse(&bytes, &entries[..entries.len() / 2])), "SPD-10");
}

#[test]
fn hostile_bytes_are_refused_without_a_panic() {
    let (bytes, entries, _) = three_levels();
    let mut rng = Rng(5);
    for round in 0..4000 {
        let mut m = bytes.clone();
        for _ in 0..=(round % 4) {
            let at = (rng.next() % m.len() as u64) as usize;
            m[at] ^= 1 << (rng.next() % 8);
        }
        // Ok or a typed refusal, never a panic or a runaway walk
        if let Ok(d) = parse(&m, &entries) {
            let n =
                d.query(&bx(-1_000_000_000.0, -1_000_000_000.0, 1_000_000_000.0, 1_000_000_000.0)).count();
            assert!(n <= d.cells());
        }
    }
}

#[test]
fn bad_boxes_and_bad_inputs_are_refused() {
    assert!(Aabb::new([f64::NAN, 0.0], [1.0, 1.0]).is_err());
    assert!(Aabb::new([0.0, 0.0], [f64::INFINITY, 1.0]).is_err());
    assert!(Aabb::new([2.0, 0.0], [1.0, 1.0]).is_err());
    assert!(Aabb::new([0.0, 0.0], [10_000_000_000_000.0, 1.0]).is_err(), "beyond the numeric policy's range");
    assert!(build_directory(&[]).is_err());
    let twice = [EntityBox { id: eid(1), bounds: bx(0.0, 0.0, 1.0, 1.0) }; 2];
    assert_eq!(rule(build_directory(&twice)), "SPD-12");
}

#[test]
fn a_file_without_a_directory_opens_without_one_and_two_directories_are_refused() {
    let mut w = ContainerWriter::new(config("t/1"));
    let meta = EntityMeta { id: eid(0), style: 0, cell: 0, bounds: Bounds { min: [0.0; 2], max: [1.0; 2] } };
    w.push_line(meta, [0.0, 0.0], [1.0, 1.0]).unwrap();
    let r = reader(w.finish().unwrap());
    assert!(SpatialDirectory::open(&r).unwrap().is_none());
    assert!(SpatialDirectory::find(r.entries()).unwrap().is_none());
    // two entries of kind "spatial": the container allows them under different subtypes, the directory does not
    let (_, entries, _) = three_levels();
    let mut two = entries.clone();
    let mut extra = *two.iter().find(|e| e.block_kind() == Some(BlockKind::SpatialDirectory)).unwrap();
    extra.subtype = 1;
    two.push(extra);
    assert_eq!(rule(SpatialDirectory::find(&two)), "SPD-11");
}

#[test]
fn the_advanced_curve_set_is_reported_beside_the_cells() {
    use pvec_container::blocks::advanced::SplineSpec;
    let entities = rooms(6, 3, 40, 1000.0);
    let built = build_directory(&entities).unwrap();
    let mut w = ContainerWriter::new(config("t/1"));
    for (e, cell) in entities.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    // one advanced curve: the container keeps the set in cell 0, so the directory cannot place it in a leaf
    let spline_box = Bounds { min: [100.0, 200.0], max: [300.0, 400.0] };
    let meta = EntityMeta { id: eid(entities.len() as u32), style: 0, cell: 0, bounds: spline_box };
    w.push_spline(
        meta,
        SplineSpec {
            degree: 1,
            knots: &[0.0, 0.0, 1.0, 1.0],
            x: &[100.0, 300.0],
            y: &[200.0, 400.0],
            weights: None,
            closed: false,
            periodic: false,
        },
    )
    .unwrap();
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    let r = reader(w.finish().unwrap());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    assert_eq!(dir.advanced_bounds(), Some(bx(100.0, 200.0, 300.0, 400.0)));
    assert_eq!(dir.entities(), entities.len() as u64, "the directory indexes the hot entities only");
    assert_eq!(dir.height(), 2);
}

// ---------------------------------------------------------------- the constants and the chapter

#[test]
fn the_named_constants_and_the_p1_6_measurements_are_recorded() {
    assert_eq!(CELL_CAPACITY, 64);
    assert_eq!(FANOUT, 32);
    assert_eq!(SPLIT_POLICY, SplitPolicy::HilbertPack);
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/directory.rs")).unwrap();
    for measured in ["516,256", "63,616", "29.08", "31.12", "docs/spikes/01-6-spatial-index.md"] {
        assert!(
            source.contains(measured),
            "src/directory.rs does not record {measured} beside the constants"
        );
    }
}

#[test]
fn chapter_one_of_streaming_md_states_the_layout_the_invariants_and_every_refusal() {
    let doc = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md"))
        .unwrap_or_default();
    for needle in [
        "# Viewport streaming",
        "## 1. The spatial directory",
        "### 1.1 Layout",
        "### 1.2 Invariants",
        "### 1.3 What a reader validates before it traverses",
        "### 1.4 Query order",
        "### 1.5 Measurements",
        "cell 0",
        "source space",
        "CELL_CAPACITY",
        "FANOUT",
        "ADR 0005",
    ] {
        assert!(doc.contains(needle), "docs/STREAMING.md lacks {needle:?}");
    }
    assert!(DIRECTORY_RULES.len() >= 12, "{} rules", DIRECTORY_RULES.len());
    for (rule, meaning) in DIRECTORY_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/STREAMING.md lacks {rule}");
    }
}

#[test]
fn the_container_validates_a_file_that_carries_a_directory_and_skips_it_when_unknown() {
    let entities = rooms(9, 3, 50, 1000.0);
    let (bytes, _) = artifact(&entities);
    let r = reader(bytes);
    r.validate().expect("a canonical artifact");
    let e = r.entries().iter().find(|e| e.block_kind() == Some(BlockKind::SpatialDirectory)).unwrap();
    assert_eq!(e.flags & pvec_container::directory::FLAG_MUST_UNDERSTAND, 0, "optional, like the components");
    assert_eq!(e.cell, 0);
    assert_eq!(e.count, 8 + 1, "8 leaves and a root");
}
