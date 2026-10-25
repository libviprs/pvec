//! Tier-aware viewport plans (issue P5.5 follow-up, pvec#129).
//!
//! The fixtures are written through the container writer with a Tier 0 block per leaf (kind 8, #149):
//! `rooms-1m` of the P5.5 tests for the range counts, a 6,400 entity drawing for the rest, and one with
//! straddlers for the rule that they get no Tier 0 and are not planned. Every test is named for the
//! behaviour it holds `ViewportQuery::plan_tiered` to.
#![allow(missing_docs)]

use pvec_container::directory::{BlockKind, STRADDLER_CELL_FIRST};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, EntityBox, LongEntity, PayloadRef, Preview, PreviewForm, Priority, ReadPlan, SpatialDirectory,
    Tier, Tier0Builder, Tier0Leaves, Tier0Params, ViewportConfig, ViewportQuery, build_directory, classify,
    payload_cells, tier0_blocks,
};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// Range requests a full-extent view of `rooms-1m` may issue for complete Tier 0 at the default reader
/// configuration. The exact blocks of the same view are 3 requests (STREAMING.md 5.4).
const FULL_EXTENT_TIER0_MAX_RANGES: u32 = 4;
const EXTENT: f64 = 100_000.0;

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

fn bx(a: f64, b: f64, c: f64, d: f64) -> Aabb {
    Aabb::new([a, b], [c, d]).unwrap()
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

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
                let r = |v: f64| (v * 1000.0).round() / 1000.0;
                out.push(EntityBox { id: eid(out.len() as u32), bounds: bx(r(x), r(y), r(x + w), r(y + h)) });
            }
        }
    }
    out
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/129".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

/// A Tier 0 plan over `total` entities: a preview for one in ten, the rest exact.
fn tier0_plan(total: u32) -> pvec_stream::Tier0Plan {
    let mut b = Tier0Builder::new(total, Tier0Params { max_error: 5.0 }).unwrap();
    for i in 0..total {
        if i % 10 == 3 {
            let v = vec![[0.0, 0.0], [1.5, 2.5], [4.0, 4.0]];
            let p = Preview::new(eid(i), PreviewForm::ChordPolyline, 0.5, [f64::from(i), 7.0], v).unwrap();
            b.add_preview(p).unwrap();
        } else {
            b.add_exact(eid(i)).unwrap();
        }
    }
    b.finish().unwrap()
}

/// Entities in their cells, and a Tier 0 block per leaf when `tier0` is set.
fn artifact(entities: &[EntityBox], tier0: bool) -> Vec<u8> {
    let built = build_directory(entities).expect("directory");
    let mut w = ContainerWriter::new(config());
    for (e, cell) in entities.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    if tier0 {
        let plan = tier0_plan(entities.len() as u32);
        for b in tier0_blocks(&plan, entities, &built.cells).unwrap() {
            w.push_tier0_leaf(b.cell.get(), b.count, to_bounds(b.bounds), b.bytes).unwrap();
        }
    }
    w.finish().unwrap()
}

/// Long lines that cross the whole drawing, so `classify` makes straddlers of them. Their payload blocks
/// are hot blocks in the payload cells, and the Tier 0 plan covers every entity but is cut by the leaves,
/// so the straddlers are in no Tier 0 block.
fn with_straddlers() -> Vec<u8> {
    let mut all = rooms(12, 12, 250, 500.0);
    let mut rng = Rng(77);
    for k in 0..200 {
        let t = (rng.unit() * 6000.0 * 1000.0).round() / 1000.0;
        let bounds = if k % 2 == 0 { bx(0.0, t, 6000.0, t) } else { bx(t, 0.0, t, 6000.0) };
        all.push(EntityBox { id: eid(all.len() as u32), bounds });
    }
    let c = classify(&all, 4.0).expect("classified");
    assert!(!c.long.is_empty(), "the fixture has straddlers");
    let built = build_directory(&c.local).unwrap();
    let placeholder = |e: &EntityBox| LongEntity {
        id: e.id,
        bounds: e.bounds,
        payload: PayloadRef { block_id: BlockId(0), slot: 0 },
    };
    let cells = payload_cells(&c.scale, &c.long.iter().map(placeholder).collect::<Vec<_>>()).unwrap();
    let mut w = ContainerWriter::new(config());
    for (e, cell) in c.local.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    for e in &c.long {
        let cell = cells.iter().find(|(id, _)| *id == e.id).unwrap().1;
        let meta = EntityMeta { id: e.id, style: 0, cell, bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    let plan = tier0_plan(all.len() as u32);
    for b in tier0_blocks(&plan, &c.local, &built.cells).unwrap() {
        w.push_tier0_leaf(b.cell.get(), b.count, to_bounds(b.bounds), b.bytes).unwrap();
    }
    w.finish().unwrap()
}

fn small(tier0: bool) -> Vec<u8> {
    artifact(&rooms(5, 8, 100, 1000.0), tier0)
}

fn million() -> &'static Vec<u8> {
    static M: OnceLock<Vec<u8>> = OnceLock::new();
    M.get_or_init(|| artifact(&rooms(1, 50, 400, 2000.0), true))
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn open(bytes: &[u8]) -> (ContainerReader<Mem>, SpatialDirectory, Tier0Leaves) {
    let cfg = ReaderConfig { cache_bytes: 1 << 30, ..ReaderConfig::default() };
    let r = ContainerReader::open(Recording::new(MemorySource::new(bytes.to_vec())), cfg).unwrap();
    let dir = SpatialDirectory::open(&r).unwrap().expect("a directory");
    let leaves = Tier0Leaves::new(r.entries(), &dir).unwrap();
    (r, dir, leaves)
}

fn view(x: f64, y: f64, w: f64, h: f64) -> ViewportQuery {
    ViewportQuery::new(bx(x, y, x + w, y + h), 1000.0 / w)
}

fn no_margin() -> ViewportConfig {
    ViewportConfig { prefetch_margin_px: 0.0 }
}

fn kind_of(r: &ContainerReader<Mem>, id: BlockId) -> Option<BlockKind> {
    r.entries()[id.0 as usize].block_kind()
}

fn tier_blocks(r: &ContainerReader<Mem>, plan: &ReadPlan, tier: Tier, priority: Priority) -> Vec<BlockId> {
    plan.requests
        .iter()
        .chain(&plan.deferred)
        .filter(|p| p.tier == tier && p.priority == priority)
        .flat_map(|p| p.request.blocks.iter().copied())
        .inspect(|b| {
            let want = if tier == Tier::Tier0 { BlockKind::Tier0 } else { BlockKind::Hot };
            assert_eq!(kind_of(r, *b), Some(want), "a {tier:?} request holds a {:?} block", kind_of(r, *b));
        })
        .collect()
}

// ------------------------------------------------------------ the criteria

#[test]
fn a_full_extent_view_of_rooms_1m_plans_complete_tier_0_in_a_stated_number_of_ranges() {
    let (r, dir, leaves) = open(million());
    let q = ViewportQuery::new(bx(0.0, 0.0, EXTENT, EXTENT), 1000.0 / EXTENT);
    let plan = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    let t0 = plan.metrics.tier0;
    let exact = q.plan(&dir, &r, &no_margin()).unwrap().metrics.visible;
    eprintln!(
        "rooms-1m full extent, Tier 0: {} ranges, {} bytes, {} wasted, {} blocks (exact blocks: {} ranges, {} bytes)",
        t0.ranges, t0.bytes_requested, t0.bytes_wasted, t0.blocks, exact.ranges, exact.bytes_requested
    );
    assert!(t0.ranges >= 1 && t0.ranges <= FULL_EXTENT_TIER0_MAX_RANGES, "{} ranges", t0.ranges);
    // complete: every Tier 0 block of the file, one per leaf
    let all: BTreeSet<BlockId> = (0..r.entries().len() as u32)
        .map(BlockId)
        .filter(|b| kind_of(&r, *b) == Some(BlockKind::Tier0))
        .collect();
    assert_eq!(all.len(), leaves.len());
    let planned: BTreeSet<BlockId> =
        tier_blocks(&r, &plan, Tier::Tier0, Priority::Visible).into_iter().collect();
    assert_eq!(planned, all);
    // and far fewer bytes than the exact blocks it stands in for
    assert!(t0.bytes_requested * 5 < exact.bytes_requested);
}

#[test]
fn tier_0_requests_come_before_every_exact_request_and_the_exact_blocks_refine() {
    let (r, dir, leaves) = open(&small(true));
    let q = view(0.0, 0.0, 4000.0, 4000.0);
    let plan = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    let tiers: Vec<Tier> = plan.requests.iter().map(|p| p.tier).collect();
    assert!(tiers.contains(&Tier::Tier0) && tiers.contains(&Tier::Exact), "{tiers:?}");
    let last_t0 = tiers.iter().rposition(|t| *t == Tier::Tier0).unwrap();
    let first_exact = tiers.iter().position(|t| *t == Tier::Exact).unwrap();
    assert!(last_t0 < first_exact, "{tiers:?}");
    // the exact blocks are the ones the untiered plan asks for
    let untiered = q.plan(&dir, &r, &no_margin()).unwrap();
    let exact: BTreeSet<BlockId> =
        tier_blocks(&r, &plan, Tier::Exact, Priority::Visible).into_iter().collect();
    let want: BTreeSet<BlockId> = untiered.visible_block_ids().into_iter().collect();
    assert_eq!(exact, want);
    // a Tier 0 block per visible leaf, and none for a leaf outside the view
    for b in tier_blocks(&r, &plan, Tier::Tier0, Priority::Visible) {
        let e = r.entries()[b.0 as usize].bounds.unwrap();
        assert!(Aabb::new(e.min, e.max).unwrap().intersects(&q.bounds));
    }
}

#[test]
fn the_budget_spends_on_tier_0_first_and_defers_the_exact_refinement() {
    let (r, dir, leaves) = open(&small(true));
    let mut q = view(0.0, 0.0, 8000.0, 8000.0);
    let all = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    assert!(all.deferred.is_empty());
    let t0_bytes = all.metrics.tier0.bytes_requested;
    assert!(t0_bytes > 0);
    q.tier_budget = t0_bytes;
    let capped = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    assert!(capped.requests.iter().all(|p| p.tier == Tier::Tier0), "only Tier 0 fits");
    assert_eq!(capped.metrics.tier0.bytes_requested, t0_bytes, "Tier 0 is complete under its own cost");
    assert!(!capped.deferred.is_empty() && capped.deferred.iter().all(|p| p.tier == Tier::Exact));
    assert_eq!(capped.metrics.visible.ranges, 0);
    assert_eq!(capped.metrics.deferred_ranges as usize, capped.deferred.len());
    // a budget of zero still lets the first request through, and that one is Tier 0
    q.tier_budget = 0;
    let zero = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    assert_eq!(zero.requests.len(), 1);
    assert_eq!(zero.requests[0].tier, Tier::Tier0);
}

#[test]
fn once_tier_0_is_cached_the_plan_asks_only_for_the_refinement() {
    let (r, dir, leaves) = open(&small(true));
    let q = view(0.0, 0.0, 4000.0, 4000.0);
    let first = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    let t0: Vec<BlockId> = tier_blocks(&r, &first, Tier::Tier0, Priority::Visible);
    assert!(!t0.is_empty());
    r.read_blocks(&t0).unwrap();
    let second = q.plan_tiered(&dir, &r, &leaves, &no_margin()).unwrap();
    assert_eq!(second.metrics.tier0.ranges, 0);
    assert_eq!(second.metrics.tier0.blocks_cached as usize, t0.len());
    assert!(second.requests.iter().all(|p| p.tier == Tier::Exact));
    assert_eq!(second.metrics.visible.ranges, first.metrics.visible.ranges);
}

#[test]
fn a_margin_leaf_prefetches_its_tier_0_block_and_not_its_exact_blocks() {
    let (r, dir, leaves) = open(&small(true));
    let q = view(2000.0, 2000.0, 2000.0, 2000.0);
    let cfg = ViewportConfig { prefetch_margin_px: 1000.0 };
    let plan = q.plan_tiered(&dir, &r, &leaves, &cfg).unwrap();
    // Tier 0 blocks of leaves outside the view may ride in a visible request, so look at every request
    let outside = |b: &BlockId| {
        let e = r.entries()[b.0 as usize].bounds.unwrap();
        !Aabb::new(e.min, e.max).unwrap().intersects(&q.bounds)
    };
    let all_t0: Vec<BlockId> = plan
        .requests
        .iter()
        .filter(|p| p.tier == Tier::Tier0)
        .flat_map(|p| p.request.blocks.iter().copied())
        .collect();
    assert!(all_t0.iter().any(outside), "the margin has Tier 0 blocks");
    // an exact prefetch block belongs to a leaf the view itself touches, never to a margin-only leaf
    let seen: BTreeSet<u32> = dir.query(&q.bounds).map(|c| c.id.get()).collect();
    for b in tier_blocks(&r, &plan, Tier::Exact, Priority::Prefetch) {
        assert!(seen.contains(&r.entries()[b.0 as usize].cell));
    }
    // and the cost of the margin still adds up
    let m = plan.metrics;
    let spanned: u64 = plan.requests.iter().map(|p| u64::from(p.request.len)).sum();
    assert_eq!(m.visible.bytes_requested + m.tier0.bytes_requested + m.prefetch.bytes_requested, spanned);
}

#[test]
fn a_file_without_tier_0_plans_the_exact_blocks_as_before() {
    let (r, dir, leaves) = open(&small(false));
    assert!(leaves.is_empty());
    let q = view(1000.0, 1000.0, 3000.0, 3000.0);
    let cfg = ViewportConfig::default();
    let tiered = q.plan_tiered(&dir, &r, &leaves, &cfg).unwrap();
    assert_eq!(tiered, q.plan(&dir, &r, &cfg).unwrap());
    assert_eq!(tiered.metrics.tier0.ranges, 0);
}

#[test]
fn a_tiered_plan_is_deterministic_and_reads_nothing() {
    let bytes = small(true);
    let (r1, d1, l1) = open(&bytes);
    let (r2, d2, l2) = open(&bytes);
    let q = view(500.0, 500.0, 3000.0, 3000.0);
    let cfg = ViewportConfig::default();
    let before = r1.source().requests().len();
    let a = q.plan_tiered(&d1, &r1, &l1, &cfg).unwrap();
    assert_eq!(a, q.plan_tiered(&d2, &r2, &l2, &cfg).unwrap());
    assert_eq!(r1.source().requests().len(), before, "planning reads nothing");
}

#[test]
fn straddlers_get_no_tier_0_block_and_are_not_planned_in_either_tier() {
    let (r, dir, leaves) = open(&with_straddlers());
    let payload: BTreeSet<BlockId> = (0..r.entries().len() as u32)
        .map(BlockId)
        .filter(|b| r.entries()[b.0 as usize].cell >= STRADDLER_CELL_FIRST)
        .collect();
    assert!(!payload.is_empty(), "the file has straddler payload blocks");
    assert!(
        r.entries()
            .iter()
            .filter(|e| e.block_kind() == Some(BlockKind::Tier0))
            .all(|e| e.cell < STRADDLER_CELL_FIRST),
        "no Tier 0 block in a payload cell"
    );
    let q = ViewportQuery::new(bx(0.0, 0.0, 6000.0, 6000.0), 1000.0 / 6000.0);
    let plan = q.plan_tiered(&dir, &r, &leaves, &ViewportConfig::default()).unwrap();
    let planned: BTreeSet<BlockId> =
        plan.requests.iter().chain(&plan.deferred).flat_map(|p| p.request.blocks.iter().copied()).collect();
    assert!(planned.is_disjoint(&payload), "the plan holds no straddler payload block");
    // the leaves are complete without them: one Tier 0 block per leaf, every leaf in view
    assert_eq!(tier_blocks(&r, &plan, Tier::Tier0, Priority::Visible).len(), leaves.len());
}
