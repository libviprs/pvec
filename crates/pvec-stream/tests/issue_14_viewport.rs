//! Viewport queries and priority range reads (issue P5.5, pvec#14).
//!
//! Each test is named for the acceptance criterion it holds the planner to. The fixture is the
//! `rooms-1m` one of the P5.1 tests (50 x 50 rooms of 400 plan-like entities over 100,000 x 100,000,
//! seed 1), written through the container writer with the spatial directory.
//!
//! The numbers the criteria say "a stated number" and "a stated fraction" for are the constants at
//! the top, and the tests print what they measured beside them.

use pvec_container::reader::RangeRequest;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, EntityBox, PlanSource, PlannedRequest, Priority, ReadPlan, SpatialDirectory, VIEWPORT_RULES,
    ViewportConfig, ViewportError, ViewportQuery, build_directory,
};
use std::collections::BTreeSet;
use std::sync::OnceLock;

/// The leaf size these fixtures are built for (the 64 of P1.6). The shipped default is `CELL_CAPACITY`.
const LEAF: u32 = 64;

/// Range requests a full-extent view of `rooms-1m` may issue for complete Tier 0 at the default
/// reader configuration (64 KiB gap, 16 MiB request ceiling).
const FULL_EXTENT_MAX_RANGES: u32 = 4;
/// After a one-screen pan with a one-screen prefetch margin already fetched, the bytes of visible
/// requests may be at most this fraction of the initial view's.
const PAN_MAX_DELTA_FRACTION: f64 = 0.1;
/// A half-screen pan with no prefetch: bytes requested after it as a fraction of the initial view's.
const HALF_PAN_MAX_DELTA_FRACTION: f64 = 0.65;
/// Bytes wasted through coalescing, as a fraction of bytes requested, at the default gap threshold, for
/// a view of a sixteenth of the drawing or more.
const MAX_WASTE_FRACTION: f64 = 0.25;
/// The same for a view of a few dozen blocks, where the 64 KiB gap is many times a block (about 2.6 KB
/// here) and the fixed threshold reads a lot of padding. Measured 0.49; the tuning is P8.6's.
const MAX_WASTE_FRACTION_SMALL_VIEW: f64 = 0.55;

const EXTENT: f64 = 100_000.0;

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
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn bx(a: f64, b: f64, c: f64, d: f64) -> Aabb {
    Aabb::new([a, b], [c, d]).unwrap()
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
                let id = EntityId::new(out.len() as u32).unwrap();
                out.push(EntityBox { id, bounds: bx(r(x), r(y), r(x + w), r(y + h)) });
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
            producer_version: "viewport/1".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn artifact(entities: &[EntityBox]) -> Vec<u8> {
    let built = build_directory(entities, LEAF).expect("directory");
    let mut w = ContainerWriter::new(config());
    for (e, cell) in entities.iter().zip(&built.cells) {
        let (lo, hi) = (e.bounds.min(), e.bounds.max());
        let bounds = Bounds { min: lo, max: hi };
        w.push_line(EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds }, lo, hi).unwrap();
    }
    let root = Bounds { min: built.bounds.min(), max: built.bounds.max() };
    w.set_spatial_directory(built.nodes, root, built.bytes.clone()).unwrap();
    w.finish().unwrap()
}

fn million() -> &'static Vec<u8> {
    static M: OnceLock<Vec<u8>> = OnceLock::new();
    M.get_or_init(|| {
        let entities = rooms(1, 50, 400, 2000.0);
        assert_eq!(entities.len(), 1_000_000);
        artifact(&entities)
    })
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn open(bytes: &[u8]) -> (ContainerReader<Mem>, SpatialDirectory) {
    let cfg = ReaderConfig { cache_bytes: 1 << 30, ..ReaderConfig::default() };
    let r = ContainerReader::open(Recording::new(MemorySource::new(bytes.to_vec())), cfg).unwrap();
    let dir = SpatialDirectory::open(&r).unwrap().expect("a directory");
    (r, dir)
}

/// Makes the requests of a plan, which is what the reader (P4.5) does with it.
fn execute(r: &ContainerReader<Mem>, plan: &ReadPlan) {
    r.read_blocks(&plan.block_ids()).unwrap();
}

/// A viewport of `w` x `h` source units whose lower left is `(x, y)`, shown on a 1,000 px wide screen.
fn view(x: f64, y: f64, w: f64, h: f64) -> ViewportQuery {
    ViewportQuery::new(bx(x, y, x + w, y + h), 1000.0 / w)
}

fn no_margin() -> ViewportConfig {
    ViewportConfig { prefetch_margin_px: 0.0, ..ViewportConfig::default() }
}

fn bounds_of(r: &ContainerReader<Mem>, id: BlockId) -> Aabb {
    let b = r.entries()[id.0 as usize].bounds.expect("a hot block has bounds");
    Aabb::new(b.min, b.max).unwrap()
}

fn grow(b: Aabb, by: f64) -> Aabb {
    let (lo, hi) = (b.min(), b.max());
    bx(lo[0] - by, lo[1] - by, hi[0] + by, hi[1] + by)
}

// ---------------------------------------------------------------- the criteria

#[test]
fn a_full_extent_view_issues_at_most_a_stated_number_of_range_requests_for_complete_tier_0() {
    let (r, dir) = open(million());
    let q = ViewportQuery::new(bx(0.0, 0.0, EXTENT, EXTENT), 1000.0 / EXTENT);
    let plan = q.plan(&dir, &r, &no_margin()).unwrap();
    let m = plan.metrics.visible;
    eprintln!(
        "full extent of rooms-1m: {} ranges, {} bytes requested, {} wasted, {} blocks",
        m.ranges, m.bytes_requested, m.bytes_wasted, m.blocks
    );
    assert!(m.ranges <= FULL_EXTENT_MAX_RANGES, "{} range requests", m.ranges);
    assert_eq!(plan.metrics.prefetch.ranges, 0, "nothing is outside the drawing to prefetch");
    // complete: every hot block of the file is in the plan
    let hot = r.entries().iter().filter(|e| e.cell != 0).count();
    assert_eq!(plan.block_ids().len(), hot);
    assert_eq!(plan.requests.len() as u32, m.ranges);
}

#[test]
fn a_pan_reuses_cached_blocks_and_requests_only_the_delta() {
    let (r, dir) = open(million());
    let first = view(30_000.0, 30_000.0, 25_000.0, 25_000.0).plan(&dir, &r, &no_margin()).unwrap();
    let initial = first.metrics.visible.bytes_requested;
    assert!(initial > 0);
    execute(&r, &first);
    // half a screen to the right
    let panned = view(42_500.0, 30_000.0, 25_000.0, 25_000.0).plan(&dir, &r, &no_margin()).unwrap();
    let delta = panned.metrics.visible.bytes_requested;
    eprintln!(
        "half-screen pan: initial {initial} bytes, delta {delta} ({:.3}), {} blocks from cache",
        delta as f64 / initial as f64,
        panned.metrics.visible.blocks_cached
    );
    assert!(delta as f64 <= HALF_PAN_MAX_DELTA_FRACTION * initial as f64);
    assert!(panned.metrics.visible.blocks_cached > 0, "the overlap comes from the cache");
    // and nothing the first view fetched is asked for again
    let again: BTreeSet<BlockId> = first.block_ids().into_iter().collect();
    assert!(panned.block_ids().iter().all(|b| !again.contains(b)));
}

#[test]
fn a_one_screen_pan_after_prefetch_is_below_a_stated_fraction_of_the_initial_view() {
    let (r, dir) = open(million());
    let cfg = ViewportConfig { prefetch_margin_px: 1000.0, ..ViewportConfig::default() }; // one screen
    let first = view(30_000.0, 30_000.0, 25_000.0, 25_000.0).plan(&dir, &r, &cfg).unwrap();
    assert!(first.metrics.prefetch.bytes_requested > 0, "the margin costs something, and it is counted");
    execute(&r, &first);
    let panned = view(55_000.0, 30_000.0, 25_000.0, 25_000.0).plan(&dir, &r, &cfg).unwrap();
    let (initial, delta) = (first.metrics.visible.bytes_requested, panned.metrics.visible.bytes_requested);
    eprintln!(
        "one-screen pan: initial visible {initial} bytes, visible delta {delta}, prefetch {}",
        first.metrics.prefetch.bytes_requested
    );
    assert!(delta as f64 <= PAN_MAX_DELTA_FRACTION * initial as f64, "{delta} of {initial}");
}

#[test]
fn bytes_wasted_through_coalescing_stay_under_a_stated_fraction_at_the_default_gap_threshold() {
    let (r, dir) = open(million());
    for (x, y, w, max) in [
        (30_000.0, 30_000.0, 25_000.0, MAX_WASTE_FRACTION),
        (10_000.0, 60_000.0, 6_000.0, MAX_WASTE_FRACTION_SMALL_VIEW),
        (0.0, 0.0, 100_000.0, MAX_WASTE_FRACTION),
    ] {
        let q = view(x, y, w, w);
        let plan = q.plan(&dir, &r, &ViewportConfig::default()).unwrap();
        for (name, m) in [("visible", plan.metrics.visible), ("prefetch", plan.metrics.prefetch)] {
            if m.bytes_requested == 0 {
                continue;
            }
            let f = m.bytes_wasted as f64 / m.bytes_requested as f64;
            eprintln!("view {w}: {name} wasted {} of {} bytes ({f:.4})", m.bytes_wasted, m.bytes_requested);
            assert!(f <= max, "{name}: {f}");
        }
        // the waste is the requests' length less the blocks' stored length, whatever the metric says
        let blocks: u64 = plan
            .requests
            .iter()
            .flat_map(|p| p.request.blocks.iter())
            .filter(|b| bounds_of(&r, **b).intersects(&q.bounds))
            .map(|b| u64::from(r.entries()[b.0 as usize].stored_length))
            .sum();
        assert_eq!(plan.metrics.visible.bytes_requested - plan.metrics.visible.bytes_wasted, blocks);
        // the two classes add up to everything the requests span
        let spanned: u64 = plan.requests.iter().map(|p| u64::from(p.request.len)).sum();
        let (v, p) = (plan.metrics.visible, plan.metrics.prefetch);
        assert_eq!(v.bytes_requested + p.bytes_requested, spanned);
    }
}

#[test]
fn plan_generation_is_deterministic_for_a_viewport_artifact_and_cache_state() {
    let (r1, d1) = open(million());
    let (r2, d2) = open(million());
    let q = view(12_345.0, 54_321.0, 18_000.0, 9_000.0);
    let cfg = ViewportConfig::default();
    assert_eq!(q.plan(&d1, &r1, &cfg).unwrap(), q.plan(&d2, &r2, &cfg).unwrap());
    // the same cache state, reached the same way, gives the same plan again
    let warm = view(10_000.0, 50_000.0, 12_000.0, 12_000.0).plan(&d1, &r1, &no_margin()).unwrap();
    execute(&r1, &warm);
    execute(&r2, &warm);
    assert_eq!(q.plan(&d1, &r1, &cfg).unwrap(), q.plan(&d2, &r2, &cfg).unwrap());
    // and asking twice changes nothing: planning does not touch the cache or its counters
    let stats = r1.stats();
    let a = q.plan(&d1, &r1, &cfg).unwrap();
    assert_eq!(a, q.plan(&d1, &r1, &cfg).unwrap());
    assert_eq!(r1.stats(), stats);
}

#[test]
fn the_plan_never_contains_a_block_outside_the_viewport_plus_the_margin() {
    let (r, dir) = open(million());
    for (x, y, w, margin_px) in [
        (30_000.0, 30_000.0, 25_000.0, 128.0),
        (0.0, 0.0, 8_000.0, 1000.0),
        (95_000.0, 95_000.0, 5_000.0, 500.0),
        (40_000.0, 40_000.0, 100.0, 0.0),
    ] {
        let q = view(x, y, w, w);
        let plan = q
            .plan(&dir, &r, &ViewportConfig { prefetch_margin_px: margin_px, ..ViewportConfig::default() })
            .unwrap();
        let margin = margin_px / q.scale;
        let outer = grow(q.bounds, margin);
        let planned: BTreeSet<BlockId> = plan.block_ids().into_iter().collect();
        for p in &plan.requests {
            for id in &p.request.blocks {
                let b = bounds_of(&r, *id);
                assert!(b.intersects(&outer), "block {id:?} is outside the viewport and its margin");
                // a prefetch block may ride in a visible request, but a prefetch request is all margin
                if p.priority == Priority::Prefetch {
                    assert!(!b.intersects(&q.bounds), "{id:?} is visible");
                }
            }
        }
        // and nothing inside it is missing (nothing is cached yet)
        for (i, e) in r.entries().iter().enumerate() {
            if e.cell == 0 {
                continue;
            }
            let b = bounds_of(&r, BlockId(i as u32));
            assert_eq!(b.intersects(&outer), planned.contains(&BlockId(i as u32)), "block {i}");
        }
    }
}

#[test]
fn zooming_into_a_dense_region_does_not_re_request_blocks_decoded_for_the_outer_view() {
    let (r, dir) = open(million());
    let outer = view(20_000.0, 20_000.0, 40_000.0, 40_000.0).plan(&dir, &r, &no_margin()).unwrap();
    execute(&r, &outer);
    let before = r.stats();
    let inner =
        view(35_000.0, 35_000.0, 4_000.0, 4_000.0).plan(&dir, &r, &ViewportConfig::default()).unwrap();
    assert!(inner.metrics.visible.blocks_cached > 0, "the inner view is wanted and held");
    assert_eq!(inner.metrics.visible.ranges, 0);
    assert_eq!(inner.metrics.visible.bytes_requested, 0);
    assert!(inner.block_ids().iter().all(|b| !outer.block_ids().contains(b)));
    assert_eq!(r.stats().requests, before.requests, "planning makes no request");
}

// ---------------------------------------------------------------- the scope

#[test]
fn visible_requests_come_first_and_the_best_gain_per_byte_comes_first_among_them() {
    let (r, dir) = open(million());
    let plan = view(30_000.0, 30_000.0, 8_000.0, 8_000.0)
        .plan(&dir, &r, &ViewportConfig { prefetch_margin_px: 300.0, ..ViewportConfig::default() })
        .unwrap();
    // prefetch-only requests, if there are any, come after every visible one
    let first_prefetch =
        plan.requests.iter().position(|p| p.priority == Priority::Prefetch).unwrap_or(plan.requests.len());
    assert!(plan.requests[first_prefetch..].iter().all(|p| p.priority == Priority::Prefetch));
    assert!(first_prefetch > 1, "need several visible requests to order them");
    let rate = |p: &PlannedRequest| p.gain / f64::from(p.request.len);
    let visible = &plan.requests[..first_prefetch];
    for w in visible.windows(2) {
        assert!(rate(&w[0]) >= rate(&w[1]), "{} then {}", rate(&w[0]), rate(&w[1]));
    }
    assert!(visible.iter().all(|p| p.gain > 0.0));
    // the margin is in the metrics on its own, whether it has requests of its own or rides in a
    // visible one across a gap
    assert!(plan.metrics.prefetch.blocks > 0 && plan.metrics.prefetch.bytes_requested > 0);
    assert_eq!(plan.metrics.visible.ranges as usize, first_prefetch);
}

#[test]
fn a_viewport_that_clips_a_cell_ranks_the_requests_by_the_share_they_bring_in() {
    // two clusters of equal size far apart in the file, one fully in the viewport and one mostly
    // outside it: the full one is worth more per byte and is requested first
    let mut entities = Vec::new();
    let mut id = 0u32;
    for (cx, cy) in [(100.0, 100.0), (5000.0, 5000.0)] {
        for k in 0..640u32 {
            let (x, y) = (cx + f64::from(k % 40), cy + f64::from(k / 40));
            entities.push(EntityBox { id: EntityId::new(id).unwrap(), bounds: bx(x, y, x + 0.5, y + 0.5) });
            id += 1;
        }
    }
    let bytes = artifact(&entities);
    let (r, dir) = open(&bytes);
    // covers the first cluster completely and the lower fifth of the second
    let q = ViewportQuery::new(bx(0.0, 0.0, 5100.0, 5003.0), 0.2);
    let plan = q.plan(&dir, &r, &no_margin()).unwrap();
    let blocks = plan.visible_block_ids();
    assert!(blocks.len() >= 2);
    let first = bounds_of(&r, blocks[0]);
    assert!(first.max()[0] < 1000.0, "the fully visible cluster is requested first: {first:?}");
}

#[test]
fn a_tier_budget_defers_the_requests_it_cannot_afford_and_always_lets_the_first_through() {
    let (r, dir) = open(million());
    let mut q = view(20_000.0, 20_000.0, 40_000.0, 40_000.0);
    let all = q.plan(&dir, &r, &ViewportConfig::default()).unwrap();
    assert!(all.requests.len() > 2 && all.deferred.is_empty());
    q.tier_budget = all.requests[0].request.len as u64 + 1;
    let capped = q.plan(&dir, &r, &ViewportConfig::default()).unwrap();
    assert_eq!(capped.requests[0], all.requests[0]);
    assert!(capped.metrics.visible.bytes_requested <= q.tier_budget);
    assert_eq!(capped.requests.len() + capped.deferred.len(), all.requests.len());
    assert_eq!(capped.metrics.prefetch.ranges, 0, "no prefetch while visible work is deferred");
    assert_eq!(capped.metrics.deferred_ranges as usize, capped.deferred.len());
    // a budget of zero still makes progress
    q.tier_budget = 0;
    let none = q.plan(&dir, &r, &ViewportConfig::default()).unwrap();
    assert_eq!(none.requests.len(), 1);
}

#[test]
fn planning_a_plan_executes_to_every_visible_entity_on_a_small_drawing() {
    let entities = rooms(5, 6, 200, 1000.0);
    let bytes = artifact(&entities);
    let (r, dir) = open(&bytes);
    let q = ViewportQuery::new(bx(1500.0, 1500.0, 3600.0, 2900.0), 0.5);
    let plan = q.plan(&dir, &r, &no_margin()).unwrap();
    execute(&r, &plan);
    let mut held = BTreeSet::new();
    for id in plan.block_ids() {
        held.insert(id);
    }
    let want: usize = entities.iter().filter(|e| e.bounds.intersects(&q.bounds)).count();
    let got: u64 = held.iter().map(|b| u64::from(r.entries()[b.0 as usize].count)).sum();
    assert!(got as usize >= want, "{got} entities in the blocks, {want} visible");
    // a second plan for the same view has nothing left to fetch
    let again = q.plan(&dir, &r, &no_margin()).unwrap();
    assert!(again.requests.is_empty());
    assert_eq!(again.metrics.visible.blocks_cached as usize, held.len());
}

#[test]
fn a_viewport_off_the_drawing_plans_nothing_and_a_bad_query_is_refused_by_rule() {
    let entities = rooms(5, 3, 100, 1000.0);
    let bytes = artifact(&entities);
    let (r, dir) = open(&bytes);
    let off = ViewportQuery::new(bx(50_000.0, 50_000.0, 51_000.0, 51_000.0), 1.0);
    let plan = off.plan(&dir, &r, &no_margin()).unwrap();
    assert!(plan.requests.is_empty() && plan.block_ids().is_empty());
    let on = bx(0.0, 0.0, 100.0, 100.0);
    for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let e = ViewportQuery::new(on, scale).plan(&dir, &r, &no_margin()).unwrap_err();
        assert_eq!(e.rule(), "VP-01", "{scale}");
    }
    for margin in [-1.0, f64::NAN, f64::INFINITY, f64::MAX] {
        let e = ViewportQuery::new(on, 1.0)
            .plan(&dir, &r, &ViewportConfig { prefetch_margin_px: margin, ..ViewportConfig::default() })
            .unwrap_err();
        assert!(matches!(e, ViewportError::BadMargin), "{margin}");
        assert_eq!(e.rule(), "VP-02");
    }
    for (id, _) in VIEWPORT_RULES {
        assert!(id.starts_with("VP-"));
    }
}

#[test]
fn both_readers_can_be_planned_against() {
    fn is_source<T: PlanSource>() {}
    is_source::<ContainerReader<Mem>>();
    is_source::<pvec_container::AsyncContainerReader<MemorySource<Vec<u8>>>>();
    let _: Option<RangeRequest> = None;
}
