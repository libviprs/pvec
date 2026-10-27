//! Tier 0 per leaf as container block kind 8 (issue P5.4b, pvec#149).
//!
//! The fixture is 6,400 short lines in 100 leaves, a Tier 0 plan over them with a preview for one in
//! ten and one entity the plan declares infeasible. Every test writes a real file through the container
//! writer and reads it back through a recording source, so what is counted is range reads.
#![allow(missing_docs)]

use pvec_container::directory::{
    BlockEntry, BlockKind, FLAG_HAS_BOUNDS, FLAG_MUST_UNDERSTAND, STRADDLER_CELL_FIRST,
};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Ellipse, Placement2, Point2, Trimmed, Vector2};
use pvec_stream::{
    Aabb, EntityBox, Preview, PreviewForm, SpatialDirectory, Tier0Builder, Tier0Error, Tier0Leaves,
    Tier0Params, Tier0Plan, build_directory, tier0_blocks,
};
use std::collections::BTreeSet;

/// The leaf size these fixtures are built for (the 64 of P1.6). The shipped default is `CELL_CAPACITY`.
const LEAF: u32 = 64;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

fn lines(n: u32) -> Vec<EntityBox> {
    let mut rng = Rng(149);
    (0..n)
        .map(|i| {
            let (x, y) = ((rng.unit() * 10_000.0).round(), (rng.unit() * 10_000.0).round());
            let (w, h) = (1.0 + (rng.unit() * 20.0).round(), 1.0 + (rng.unit() * 20.0).round());
            EntityBox { id: eid(i), bounds: Aabb::new([x, y], [x + w, y + h]).unwrap() }
        })
        .collect()
}

const ENTITIES: u32 = 6400;
const BUDGET: f64 = 5.0;

/// An ellipse arc a billion units wide: no bounded preview fits in 4,096 vertices.
fn impossible() -> Curve {
    let p = Placement2::new(Point2::new(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap();
    let e = Ellipse::new(p, 1_000_000_000.0, 500_000_000.0).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), 0.0, 5.0, true).unwrap())
}

fn plan() -> Tier0Plan {
    let mut b = Tier0Builder::new(ENTITIES, Tier0Params { max_error: BUDGET }).unwrap();
    for i in 0..ENTITIES {
        if i == 777 {
            b.add_curve(eid(i), &impossible()).unwrap();
        } else if i % 10 == 3 {
            let v = vec![[0.0, 0.0], [1.5, 2.5], [4.0, 4.0]];
            let p = Preview::new(eid(i), PreviewForm::ChordPolyline, 0.5, [f64::from(i), 7.0], v).unwrap();
            b.add_preview(p).unwrap();
        } else {
            b.add_exact(eid(i)).unwrap();
        }
    }
    b.finish().unwrap()
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/149".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn reader(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

struct Written {
    bytes: Vec<u8>,
    plan: Tier0Plan,
    cells: Vec<u32>,
}

/// The writer order of the issue: directory over the entities, hot blocks in their cells, the plan cut
/// per leaf, one block each.
fn write() -> Written {
    let all = lines(ENTITIES);
    let built = build_directory(&all, LEAF).unwrap();
    let plan = plan();
    let mut w = ContainerWriter::new(config());
    for (e, cell) in all.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    for b in tier0_blocks(&plan, &all, &built.cells).unwrap() {
        w.push_tier0_leaf(b.cell.get(), b.count, to_bounds(b.bounds), b.bytes).unwrap();
    }
    Written { bytes: w.finish().unwrap(), plan, cells: built.cells.iter().map(|c| c.get()).collect() }
}

fn written() -> &'static Written {
    static W: std::sync::OnceLock<Written> = std::sync::OnceLock::new();
    W.get_or_init(write)
}

fn tier0_entries(r: &ContainerReader<Mem>) -> Vec<(usize, BlockEntry)> {
    r.entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.block_kind() == Some(BlockKind::Tier0))
        .map(|(i, e)| (i, *e))
        .collect()
}

#[test]
fn kind_eight_is_tier0_and_nine_is_still_unknown() {
    assert_eq!(BlockKind::Tier0.id(), 8);
    assert_eq!(BlockKind::Tier0.name(), "tier0");
    assert_eq!(BlockKind::from_id(8), Some(BlockKind::Tier0));
    assert_eq!(BlockKind::from_id(9), None);
    assert_eq!(BlockKind::ALL.len(), 8);
}

#[test]
fn a_file_with_tier0_validates_and_has_one_optional_block_per_leaf() {
    let w = written();
    let r = reader(w.bytes.clone());
    r.validate().expect("a file with Tier 0 blocks validates");
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let entries = tier0_entries(&r);
    assert_eq!(entries.len(), dir.cells(), "one block for each leaf");
    assert!(entries.len() >= 100);
    for (_, e) in &entries {
        assert_eq!(e.kind, 8);
        assert_eq!(e.flags & FLAG_MUST_UNDERSTAND, 0, "optional: an older reader skips it");
        assert_ne!(e.flags & FLAG_HAS_BOUNDS, 0);
        assert_ne!(e.cell, 0);
        assert!(e.cell < STRADDLER_CELL_FIRST);
    }
    // compressed with deflate whatever the policy (LZ4 until P8.6), and smaller for it
    assert!(entries.iter().all(|(_, e)| e.codec == 0 || e.codec == 2));
    let (stored, raw): (u64, u64) = entries.iter().fold((0, 0), |(s, r), (_, e)| {
        (s + u64::from(e.stored_length), r + u64::from(e.uncompressed_length))
    });
    assert!(entries.iter().filter(|(_, e)| e.codec == 2).count() > entries.len() / 2);
    assert!(stored < raw, "{stored} stored of {raw}");
    // the block directory orders blocks by kind and then cell
    assert!(entries.windows(2).all(|p| p[0].1.cell < p[1].1.cell));
    // each block is its leaf's: cell, entity count and the leaf's bounds
    for cell in dir.query(&dir.bounds()) {
        let (_, e) = entries.iter().find(|(_, e)| e.cell == cell.id.get()).expect("a block for the leaf");
        assert_eq!(e.count, cell.entities);
        assert_eq!(e.bounds.map(|b| (b.min, b.max)), Some((cell.bounds.min(), cell.bounds.max())));
    }
}

#[test]
fn each_leaf_plan_covers_exactly_its_leaf_and_together_they_are_the_global_plan() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    assert_eq!(leaves.len(), dir.cells());
    let (mut exact, mut previews, mut infeasible) = (0, 0, 0);
    let mut seen = BTreeSet::new();
    for cell in dir.query(&dir.bounds()) {
        let in_leaf: BTreeSet<u32> =
            w.cells.iter().enumerate().filter(|(_, c)| **c == cell.id.get()).map(|(i, _)| i as u32).collect();
        let got = leaves.tier0(&r, cell.id).unwrap().expect("a plan for the leaf");
        let ids: Vec<EntityId> = in_leaf.iter().map(|i| eid(*i)).collect();
        got.check_covers(&ids).expect("covers the leaf's entity set");
        assert_eq!(got.covered(), in_leaf.len());
        for id in &ids {
            assert!(seen.insert(id.index()), "an entity is in one leaf's Tier 0 only");
            let want = w.plan.entry(*id);
            assert_eq!(got.entry(*id), want, "the same entry as the global plan");
        }
        exact += got.exact_count();
        previews += got.previews().len();
        infeasible += got.infeasible().len();
    }
    assert_eq!(seen.len(), ENTITIES as usize);
    assert_eq!(
        (exact, previews, infeasible),
        (w.plan.exact_count(), w.plan.previews().len(), w.plan.infeasible().len())
    );
    assert_eq!(infeasible, 1, "the impossible arc is declared, not dropped");
}

#[test]
fn reading_one_leaf_is_one_range_of_that_leafs_block() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    let cell = dir.query(&dir.bounds()).nth(40).unwrap();
    let (i, e) = tier0_entries(&r).into_iter().find(|(_, e)| e.cell == cell.id.get()).unwrap();
    let before = r.source().requests().len();
    leaves.tier0(&r, cell.id).unwrap().unwrap();
    let after = r.source().requests();
    assert_eq!(after.len() - before, 1, "one request");
    assert_eq!(after[before].offset, e.offset);
    assert_eq!(after[before].len, e.stored_length);
    assert!(r.is_cached(BlockId(i as u32)));
}

#[test]
fn a_leaf_with_no_block_has_no_plan() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let mut entries = r.entries().to_vec();
    let at = entries.iter().position(|e| e.block_kind() == Some(BlockKind::Tier0)).unwrap();
    let gone = entries.remove(at);
    let leaves = Tier0Leaves::new(&entries, &dir).unwrap();
    assert_eq!(leaves.len(), dir.cells() - 1);
    let cell = dir.query(&dir.bounds()).find(|c| c.id.get() == gone.cell).unwrap();
    assert!(leaves.tier0(&r, cell.id).unwrap().is_none());
}

// ------------------------------------------------------------- refusals

#[test]
fn tier0_09_onward_a_block_for_a_leaf_that_does_not_exist_is_refused() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let mut entries = r.entries().to_vec();
    let at = entries.iter().position(|e| e.block_kind() == Some(BlockKind::Tier0)).unwrap();
    entries[at].cell = dir.cells() as u32 + 10;
    let e = Tier0Leaves::new(&entries, &dir).unwrap_err();
    assert_eq!(e.rule(), "TIER0-10", "{e}");
}

#[test]
fn a_block_in_cell_zero_or_in_a_straddler_payload_cell_is_refused() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    for cell in [0, STRADDLER_CELL_FIRST, u32::MAX] {
        let mut entries = r.entries().to_vec();
        let at = entries.iter().position(|e| e.block_kind() == Some(BlockKind::Tier0)).unwrap();
        entries[at].cell = cell;
        let e = Tier0Leaves::new(&entries, &dir).unwrap_err();
        assert_eq!(e.rule(), "TIER0-11", "cell {cell}: {e}");
    }
}

#[test]
fn two_blocks_for_one_leaf_are_refused() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let mut entries = r.entries().to_vec();
    let at = entries.iter().position(|e| e.block_kind() == Some(BlockKind::Tier0)).unwrap();
    let again = entries[at];
    entries.insert(at + 1, again);
    let e = Tier0Leaves::new(&entries, &dir).unwrap_err();
    assert_eq!(e.rule(), "TIER0-12", "{e}");
}

#[test]
fn an_entry_that_disagrees_with_its_leaf_or_its_bytes_is_refused() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let at = r.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Tier0)).unwrap();
    // a count that is not the leaf's entity count
    let mut entries = r.entries().to_vec();
    entries[at].count += 1;
    assert_eq!(Tier0Leaves::new(&entries, &dir).unwrap_err().rule(), "TIER0-13");
    // bounds that are not the leaf's
    let mut entries = r.entries().to_vec();
    entries[at].bounds = Some(Bounds { min: [0.0, 0.0], max: [1.0, 1.0] });
    assert_eq!(Tier0Leaves::new(&entries, &dir).unwrap_err().rule(), "TIER0-13");
    // no bounds at all
    let mut entries = r.entries().to_vec();
    entries[at].bounds = None;
    assert_eq!(Tier0Leaves::new(&entries, &dir).unwrap_err().rule(), "TIER0-13");
}

#[test]
fn a_leaf_whose_tier0_does_not_cover_the_leaf_is_refused() {
    let w = written();
    let all = lines(ENTITIES);
    let built = build_directory(&all, LEAF).unwrap();
    let blocks = tier0_blocks(&w.plan, &all, &built.cells).unwrap();
    let leaf_ids = |cell: u32| -> Vec<EntityId> {
        w.cells.iter().enumerate().filter(|(_, c)| **c == cell).map(|(i, _)| eid(i as u32)).collect()
    };
    let b = &blocks[5];
    let ids = leaf_ids(b.cell.get());
    let plan = Tier0Plan::decode_leaf(&b.bytes).unwrap();
    plan.check_covers(&ids).unwrap();
    // an entity the leaf holds is missing: Gap, TIER0-01
    let mut missing = ids.clone();
    missing.push(eid(w.cells.len() as u32 - 1));
    missing.sort();
    missing.dedup();
    if missing.len() > ids.len() {
        assert!(matches!(plan.check_covers(&missing), Err(Tier0Error::Gap(_))));
    }
    // an entity the leaf does not hold is in the plan: OutOfRange, TIER0-03
    let fewer = &ids[1..];
    assert!(matches!(plan.check_covers(fewer), Err(Tier0Error::OutOfRange(_))));
    // the dense stream and the leaf stream are not interchangeable
    assert!(Tier0Plan::decode(&b.bytes).is_err());
    assert!(Tier0Plan::decode_leaf(&w.plan.encode()).is_err());
}

#[test]
fn hostile_leaf_streams_are_refused_before_they_allocate() {
    let w = written();
    let all = lines(ENTITIES);
    let built = build_directory(&all, LEAF).unwrap();
    let good = tier0_blocks(&w.plan, &all, &built.cells).unwrap().remove(7).bytes;
    assert!(Tier0Plan::decode_leaf(&good).is_ok());
    // every truncation is an error, never a panic or a short plan
    for n in 0..good.len() {
        assert!(Tier0Plan::decode_leaf(&good[..n]).is_err(), "truncated to {n}");
    }
    // trailing bytes
    let mut long = good.clone();
    long.push(0);
    assert!(Tier0Plan::decode_leaf(&long).is_err());
    // counts that promise more than the bytes hold
    let mut huge = good.clone();
    huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(Tier0Plan::decode_leaf(&huge).is_err());
    // an unknown version
    let mut v = good.clone();
    v[4] = 9;
    assert!(Tier0Plan::decode_leaf(&v).is_err());
}

#[test]
fn the_writer_refuses_cell_zero_the_payload_cells_and_a_second_block_for_a_cell() {
    let mut w = ContainerWriter::new(config());
    let bounds = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
    for cell in [0, STRADDLER_CELL_FIRST, u32::MAX] {
        let e = w.push_tier0_leaf(cell, 1, bounds, vec![0; 24]).unwrap_err();
        assert_eq!(e.rule(), "WRT-14", "cell {cell}");
    }
    w.push_tier0_leaf(3, 1, bounds, vec![0; 24]).unwrap();
    assert_eq!(w.push_tier0_leaf(3, 1, bounds, vec![0; 24]).unwrap_err().rule(), "WRT-14");
    let bad = Bounds { min: [f64::NAN, 0.0], max: [1.0, 1.0] };
    assert_eq!(w.push_tier0_leaf(4, 1, bad, vec![0; 24]).unwrap_err().rule(), "WRT-06");
}

// ------------------------------------------------------------ measurement

/// The compression choice, measured on the artifact the issue names. A measurement, not a gate: run in
/// release mode with `-- --ignored --nocapture`.
#[test]
#[ignore = "measurement"]
fn measurement_million_entities_per_leaf_compressed_or_not() {
    use pvec_container::compress::{Codec, compress, decompress};
    use std::time::Instant;
    let n: u32 = 1_000_000;
    let mut b = Tier0Builder::new(n, Tier0Params { max_error: BUDGET }).unwrap();
    let mut rng = Rng(5);
    for i in 0..n {
        if rng.next() % 100 < 8 {
            let k = 3 + (rng.next() % 6) as usize;
            let v: Vec<[f32; 2]> = (0..k).map(|j| [j as f32 * 3.0, (rng.unit() * 20.0) as f32]).collect();
            let origin = [(rng.unit() * 10_000.0).round(), (rng.unit() * 10_000.0).round()];
            b.add_preview(Preview::new(eid(i), PreviewForm::ChordPolyline, 0.5, origin, v).unwrap()).unwrap();
        } else {
            b.add_exact(eid(i)).unwrap();
        }
    }
    let plan = b.finish().unwrap();
    let boxes: Vec<EntityBox> = (0..n)
        .map(|i| {
            let (x, y) = (f64::from(i % 1000) * 10.0, f64::from(i / 1000) * 10.0);
            EntityBox { id: eid(i), bounds: Aabb::new([x, y], [x + 5.0, y + 5.0]).unwrap() }
        })
        .collect();
    let built = build_directory(&boxes, LEAF).unwrap();
    let blocks = tier0_blocks(&plan, &boxes, &built.cells).unwrap();
    let raw: usize = blocks.iter().map(|b| b.bytes.len()).sum();
    println!("leaves {} raw bytes {} (global stream {})", blocks.len(), raw, plan.encode().len());
    for codec in Codec::ALL {
        let packed: Vec<Vec<u8>> = blocks.iter().map(|b| compress(codec, &b.bytes)).collect();
        let stored: usize = packed.iter().map(Vec::len).sum();
        let t = Instant::now();
        for (p, b) in packed.iter().zip(&blocks) {
            let out = decompress(codec, p, b.bytes.len()).unwrap();
            let _ = Tier0Plan::decode_leaf(&out).unwrap();
        }
        println!(
            "{:8} stored {:>10} ratio {:.3} fetch+decode {:?}",
            codec.name(),
            stored,
            stored as f64 / raw as f64,
            t.elapsed()
        );
    }
}

#[test]
fn two_leaves_with_swapped_blocks_pass_the_count_and_fail_the_full_check() {
    let all = lines(ENTITIES);
    let built = build_directory(&all, LEAF).unwrap();
    let mut blocks = tier0_blocks(&plan(), &all, &built.cells).unwrap();
    assert_eq!(blocks[2].count, blocks[3].count);
    let swapped = blocks[3].bytes.clone();
    blocks[3].bytes = std::mem::replace(&mut blocks[2].bytes, swapped);
    let mut w = ContainerWriter::new(config());
    for (e, cell) in all.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    for b in blocks.iter() {
        w.push_tier0_leaf(b.cell.get(), b.count, to_bounds(b.bounds), b.bytes.clone()).unwrap();
    }
    let r = reader(w.finish().unwrap());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    let cell = blocks[2].cell;
    assert!(leaves.tier0(&r, cell).is_ok(), "the cheap read only holds the count");
    let e = leaves.verify(&r, cell).unwrap_err();
    assert!(matches!(e, Tier0Error::Gap(_)), "{e}");
    assert_eq!(e.rule(), "TIER0-01");
    // an untouched leaf verifies
    assert!(leaves.verify(&r, blocks[10].cell).unwrap().is_some());
}

#[test]
fn tier0_14_a_block_the_container_refuses_to_read_is_a_read_error() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let (_, e) = tier0_entries(&r).into_iter().nth(3).unwrap();
    // a byte flipped inside the stored block: the directory still opens, the block checksum does not hold
    let mut bytes = w.bytes.clone();
    bytes[e.offset as usize + e.stored_length as usize / 2] ^= 0xFF;
    let r = reader(bytes);
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    let cell = dir.query(&dir.bounds()).find(|c| c.id.get() == e.cell).unwrap();
    let err = leaves.tier0(&r, cell.id).unwrap_err();
    assert!(matches!(err, Tier0Error::Read(_)), "{err}");
    assert_eq!(err.rule(), "TIER0-14");
    assert!(leaves.verify(&r, cell.id).is_err());
}
