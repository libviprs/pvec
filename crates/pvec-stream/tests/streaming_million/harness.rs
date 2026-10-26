//! The harness behind `streaming_million.rs`: the three fixtures, the viewport driver and the
//! load-shed scenario.
//!
//! The fixtures are rooms of plan-like entities (the shape of `rooms-1m` in the P5.5 tests) on a grid
//! whose room size and per-room density do not change with N, so a viewport of a given size holds the
//! same number of entities in the 100,000 and in the 1,000,000 artifact. That is what makes "resident
//! bytes do not grow with N" a statement about the runtime and not about the viewport.
//!
//! - mixed: every entity a line, ids in room order.
//! - spline-heavy: one entity in four a cubic B-spline (the advanced set, cell 0), the rest lines.
//! - straddler-heavy: lines plus drawing-spanning lines, `LONG_PER_BAND` per row and per column of
//!   rooms, so a viewport meets a number of them that depends on its size and not on N.

use pvec_container::BlockId;
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::{BlockKind, STRADDLER_CELL_FIRST};
use pvec_container::source::{BlockSource, Bytes, MemorySource, Recording, SourceError};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, EntityBox, GenerationClock, JobOutcome, LongEntity, PayloadRef, STRADDLER_FACTOR, SpatialDirectory,
    StraddlerIndex, ViewportConfig, ViewportQuery, build_directory, build_straddlers, classify,
    payload_cells, run_job, scan_payloads,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub const SMALL: u32 = 100_000;
pub const MILLION: u32 = 1_000_000;

const ROOM: f64 = 2000.0;
const PER_ROOM: u32 = 400;
const LONG_PER_BAND: u32 = 60;
/// Window sides in rooms, wide to tight.
const SIDES: [u32; 3] = [10, 4, 1];
const SEED: u64 = 17;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Mixed,
    SplineHeavy,
    StraddlerHeavy,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Mixed, Kind::SplineHeavy, Kind::StraddlerHeavy];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mixed => "mixed",
            Kind::SplineHeavy => "spline-heavy",
            Kind::StraddlerHeavy => "straddler-heavy",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Wide,
    Mid,
    Tight,
}

impl Scale {
    pub fn name(self) -> &'static str {
        match self {
            Scale::Wide => "wide",
            Scale::Mid => "mid",
            Scale::Tight => "tight",
        }
    }
    fn side(self) -> u32 {
        SIDES[self as usize]
    }
}

// ---------------------------------------------------------------- the generator

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

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).expect("an entity id")
}

fn bx(a: f64, b: f64, c: f64, d: f64) -> Aabb {
    Aabb::new([a, b], [c, d]).expect("a box")
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

/// The grid for `entities`: columns and rows of rooms, both at least the largest window plus its offset.
fn grid(entities: u32) -> (u32, u32) {
    let rooms = entities.div_ceil(PER_ROOM);
    let nx = (f64::from(rooms).sqrt().ceil() as u32).max(12);
    (nx, rooms.div_ceil(nx).max(12))
}

/// `count` plan-like boxes, room by room, in the order a drawing is made.
fn rooms(seed: u64, count: u32, nx: u32) -> Vec<Aabb> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(count as usize);
    'rooms: for room in 0.. {
        let (ox, oy) = (f64::from(room % nx) * ROOM, f64::from(room / nx) * ROOM);
        for _ in 0..PER_ROOM {
            if out.len() as u32 == count {
                break 'rooms;
            }
            let wall = rng.next() % 20 == 0;
            let (w, h) = if wall {
                if rng.next() % 2 == 0 { (ROOM * 0.9, 0.0) } else { (0.0, ROOM * 0.9) }
            } else {
                (rng.unit() * ROOM * 0.04, rng.unit() * ROOM * 0.04)
            };
            let x = ox + rng.unit() * (ROOM - w);
            let y = oy + rng.unit() * (ROOM - h);
            out.push(bx(r3(x), r3(y), r3(x + w), r3(y + h)));
        }
    }
    out
}

/// Drawing-spanning lines: `LONG_PER_BAND` across every row of rooms and down every column.
fn spanning(seed: u64, nx: u32, ny: u32) -> Vec<Aabb> {
    let mut rng = Rng(seed);
    let (wx, wy) = (f64::from(nx) * ROOM, f64::from(ny) * ROOM);
    let mut out = Vec::new();
    for row in 0..ny {
        for _ in 0..LONG_PER_BAND {
            let y = r3((f64::from(row) + rng.unit()) * ROOM);
            out.push(bx(0.0, y, wx, y));
        }
    }
    for col in 0..nx {
        for _ in 0..LONG_PER_BAND {
            let x = r3((f64::from(col) + rng.unit()) * ROOM);
            out.push(bx(x, 0.0, x, wy));
        }
    }
    out
}

fn config(seed: u64) -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: format!("million/{seed}"),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn push_hot(w: &mut ContainerWriter, id: EntityId, cell: u32, b: Aabb) {
    let meta = EntityMeta { id, style: 0, cell, bounds: to_bounds(b) };
    w.push_line(meta, b.min(), b.max()).expect("a line");
}

fn push_spline(w: &mut ContainerWriter, id: EntityId, b: Aabb) {
    const KNOTS: [f64; 9] = [0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0];
    const LIFT: [f64; 5] = [0.0, 1.0, 0.0, 1.0, 0.0];
    let (lo, hi) = (b.min(), b.max());
    let x: Vec<f64> = (0..5).map(|k| r3(lo[0] + (hi[0] - lo[0]) * f64::from(k) / 4.0)).collect();
    let y: Vec<f64> = LIFT.iter().map(|t| r3(lo[1] + (hi[1] - lo[1]) * t)).collect();
    let meta = EntityMeta { id, style: 0, cell: 0, bounds: to_bounds(b) };
    let spec =
        SplineSpec { degree: 3, knots: &KNOTS, x: &x, y: &y, weights: None, closed: false, periodic: false };
    w.push_spline(meta, spec).expect("a spline");
}

/// The artifact of `kind` with `entities` entities, from `seed`.
pub fn generate(kind: Kind, entities: u32, seed: u64) -> Vec<u8> {
    let (nx, ny) = grid(entities);
    match kind {
        Kind::Mixed | Kind::SplineHeavy => {
            let boxes = rooms(seed, entities, nx);
            // The directory covers the hot entities; the splines are in the advanced set, cell 0.
            let is_spline = |i: usize| kind == Kind::SplineHeavy && i % 4 == 0;
            let hot: Vec<EntityBox> = boxes
                .iter()
                .enumerate()
                .filter(|(i, _)| !is_spline(*i))
                .map(|(i, b)| EntityBox { id: eid(i as u32), bounds: *b })
                .collect();
            let built = build_directory(&hot).expect("directory");
            let mut w = ContainerWriter::new(config(seed));
            let mut cells = built.cells.iter();
            for (i, b) in boxes.iter().enumerate() {
                if is_spline(i) {
                    push_spline(&mut w, eid(i as u32), *b);
                } else {
                    push_hot(&mut w, eid(i as u32), cells.next().expect("a cell").get(), *b);
                }
            }
            w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone())
                .expect("directory");
            w.finish().expect("artifact")
        }
        Kind::StraddlerHeavy => {
            let long_boxes = spanning(seed ^ 0x5eed, nx, ny);
            let local_n = entities - long_boxes.len() as u32;
            let mut all: Vec<EntityBox> = rooms(seed, local_n, nx)
                .into_iter()
                .enumerate()
                .map(|(i, bounds)| EntityBox { id: eid(i as u32), bounds })
                .collect();
            all.extend(
                long_boxes
                    .into_iter()
                    .enumerate()
                    .map(|(k, bounds)| EntityBox { id: eid(local_n + k as u32), bounds }),
            );
            straddler_artifact(seed, &all)
        }
    }
}

/// The writer order of P5.3b: classify, build the directory over the local entities, write the local
/// entities and the straddlers' payloads in the cells `payload_cells` names, learn the block ids from
/// that layout, build the lists over them and write the file again with the lists.
fn straddler_artifact(seed: u64, all: &[EntityBox]) -> Vec<u8> {
    let c = classify(all, STRADDLER_FACTOR).expect("classified");
    let built = build_directory(&c.local).expect("directory");
    let placeholder = |e: &EntityBox| LongEntity {
        id: e.id,
        bounds: e.bounds,
        payload: PayloadRef { block_id: BlockId(0), slot: 0 },
    };
    let cells = payload_cells(&c.scale, &c.long.iter().map(placeholder).collect::<Vec<_>>()).expect("cells");
    let by_id: HashMap<u32, u32> = cells.iter().map(|(id, cell)| (id.index(), *cell)).collect();
    let lay_out = |lists: Option<(u32, Vec<u8>)>| {
        let mut w = ContainerWriter::new(config(seed));
        for (e, cell) in c.local.iter().zip(&built.cells) {
            push_hot(&mut w, e.id, cell.get(), e.bounds);
        }
        for e in &c.long {
            let cell = by_id[&e.id.index()];
            assert!(cell >= STRADDLER_CELL_FIRST);
            push_hot(&mut w, e.id, cell, e.bounds);
        }
        w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone())
            .expect("directory");
        if let Some((entries, bytes)) = lists {
            let world = c.long.iter().skip(1).fold(c.long[0].bounds, |u, e| {
                bx(
                    u.min()[0].min(e.bounds.min()[0]),
                    u.min()[1].min(e.bounds.min()[1]),
                    u.max()[0].max(e.bounds.max()[0]),
                    u.max()[1].max(e.bounds.max()[1]),
                )
            });
            w.set_straddler_lists(entries, to_bounds(world), bytes).expect("lists");
        }
        w.finish().expect("artifact")
    };
    let first = lay_out(None);
    let r = ContainerReader::open(MemorySource::new(first), ReaderConfig::default()).expect("a first pass");
    let slots: HashMap<u32, PayloadRef> =
        scan_payloads(&r).expect("slots").into_iter().map(|s| (s.id.index(), s.payload)).collect();
    let long: Vec<LongEntity> = c
        .long
        .iter()
        .map(|e| LongEntity { id: e.id, bounds: e.bounds, payload: slots[&e.id.index()] })
        .collect();
    let lists = build_straddlers(&c.scale, &long).expect("lists");
    lay_out(Some((lists.entries, lists.bytes)))
}

/// The artifact, built once per process. The lock is held while one is built, so two big artifacts are
/// never in construction at once.
pub fn artifact(kind: Kind, entities: u32) -> Arc<[u8]> {
    static CACHE: Mutex<Vec<(u64, Arc<[u8]>)>> = Mutex::new(Vec::new());
    let key = ((kind as u64) << 32) | u64::from(entities);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, found)) = cache.iter().find(|(k, _)| *k == key) {
        return Arc::clone(found);
    }
    let built: Arc<[u8]> = generate(kind, entities, SEED).into();
    cache.push((key, Arc::clone(&built)));
    built
}

pub struct Counts {
    pub entities: u64,
    pub advanced: u64,
    pub long: u64,
}

pub fn counts(bytes: &[u8]) -> Counts {
    let r = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).expect("opens");
    let (mut hot, mut advanced, mut long) = (0u64, 0u64, 0u64);
    for e in r.entries() {
        match e.block_kind() {
            Some(BlockKind::Hot) => {
                hot += u64::from(e.count);
                if e.cell >= STRADDLER_CELL_FIRST {
                    long += u64::from(e.count);
                }
            }
            Some(BlockKind::Records) => advanced += u64::from(e.count),
            _ => {}
        }
    }
    Counts { entities: hot + advanced, advanced, long }
}

// ---------------------------------------------------------------- the viewport driver

fn query(window: Aabb) -> ViewportQuery {
    ViewportQuery::new(window, 1000.0 / (window.max()[0] - window.min()[0]))
}

fn no_margin() -> ViewportConfig {
    ViewportConfig { prefetch_margin_px: 0.0 }
}

pub struct Measured {
    pub directory_ready: Duration,
    pub first_drawing: Duration,
    pub ranges: u64,
    /// Of those, the ranges that read the advanced chunks (one per chunk, or fewer when chunks are adjacent).
    pub advanced_ranges: u64,
    /// Bytes asked of the source for the viewport (the advanced set included, when it is read).
    pub bytes: u64,
    pub blocks: u64,
    /// Decode work for the viewport: bytes stored plus bytes decoded.
    pub decode_work: u64,
    /// The reader's peak of live decoded bytes, plus the parsed directory and the straddler index.
    pub peak_resident: u64,
    /// What the resident indexes cost: the cached directory block, its parsed copy and the straddler
    /// index. It grows with N (about one cell per 64 entities) and is budgeted per entity.
    pub index_bytes: u64,
    /// What reading the advanced set added to the peak: the six blocks and the coalesced request they
    /// arrive in. Zero when the viewport does not touch the advanced curves.
    pub advanced_set_bytes: u64,
    /// The six advanced blocks as stored and as decoded.
    pub advanced_stored: u64,
    pub advanced_decoded: u64,
    /// Bytes the viewport pins once its blocks are read, besides the indexes and before the advanced set
    /// is read: the part of the resident bytes that is the viewport's own.
    pub held: u64,
}

impl Measured {
    /// The panel's quantity (Q11): resident bytes with the advanced set taken out.
    pub fn resident_without_advanced(&self) -> u64 {
        self.peak_resident - self.advanced_set_bytes
    }
    /// Range requests besides the advanced chunks, the number the old single-set read was added to.
    pub fn ranges_without_advanced(&self) -> u64 {
        self.ranges - self.advanced_ranges
    }
    /// Bytes the viewport asked for besides the advanced set.
    pub fn bytes_without_advanced(&self) -> u64 {
        self.bytes.saturating_sub(self.advanced_stored)
    }
    /// Decode work besides the advanced set.
    pub fn work_without_advanced(&self) -> u64 {
        self.decode_work.saturating_sub(self.advanced_stored + self.advanced_decoded)
    }
    pub fn line(&self) -> String {
        format!(
            "directory {:?}, first drawing {:?}, {} ranges, {} bytes, {} blocks, work {}, resident {} (index {}, advanced {}, without advanced {}, held {})",
            self.directory_ready,
            self.first_drawing,
            self.ranges,
            self.bytes,
            self.blocks,
            self.decode_work,
            self.peak_resident,
            self.index_bytes,
            self.advanced_set_bytes,
            self.resident_without_advanced(),
            self.held
        )
    }
}

/// One viewport the way the runtime serves it, on a cold reader: directory ready, then plan, read the
/// planned blocks, read the straddler pages and payloads the viewport touches, and read the advanced
/// set when it touches the viewport (the whole set: STREAMING 2.8).
pub fn measure(kind: Kind, entities: u32, scale: Scale) -> Measured {
    let bytes = artifact(kind, entities);
    let t0 = Instant::now();
    let r = ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default())
        .expect("opens");
    let dir = SpatialDirectory::open(&r).expect("directory").expect("a directory");
    let long = StraddlerIndex::open(&r).expect("straddler index");
    let directory_ready = t0.elapsed();

    let acct = r.accountant();
    let (requests0, work0, live_open) = (r.source().requests().len(), acct.work(), acct.live());
    let (o, side) = (2.0 * ROOM, f64::from(scale.side()) * ROOM);
    let win = bx(o, o, o + side, o + side);
    let t1 = Instant::now();
    let plan = query(win).plan(&dir, &r, &no_margin()).expect("plan");
    let mut ids = plan.block_ids();
    if let Some(ix) = &long {
        let mut payload: Vec<BlockId> =
            ix.query(&r, &win).expect("straddlers").iter().map(|h| h.payload.block_id).collect();
        payload.sort();
        payload.dedup();
        ids.extend(payload);
    }
    let blocks = r.read_blocks(&ids).expect("blocks");
    let drawn: usize = blocks.iter().map(|b| b.hot().map_or(0, |h| h.len())).sum();
    let (peak_before, live_before) = (acct.peak(), acct.live());

    // The whole set is read when it touches the viewport (STREAMING 2.8), chunk by chunk.
    let requests_before_advanced = r.source().requests().len();
    let chunks = if plan.advanced_in_view { r.advanced_chunk_count().expect("chunk count") } else { 0 };
    let sets: Vec<_> =
        (0..chunks).map(|c| r.advanced(c).expect("advanced chunk").expect("chunk exists")).collect();
    for set in &sets {
        let view = set.view().expect("view");
        assert_eq!((0..view.len()).filter(|i| view.record(*i).is_some()).count(), view.len());
    }
    let first_drawing = t1.elapsed();
    assert!(drawn > 0 || !sets.is_empty(), "the viewport drew nothing");

    let advanced: Vec<_> = r
        .entries()
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Records | BlockKind::Arena)))
        .collect();
    let read_advanced = !sets.is_empty();
    let (advanced_stored, advanced_decoded) = if read_advanced {
        (
            advanced.iter().map(|e| u64::from(e.stored_length)).sum(),
            advanced.iter().map(|e| u64::from(e.uncompressed_length)).sum::<u64>(),
        )
    } else {
        (0, 0)
    };
    let parsed = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::SpatialDirectory))
        .map(|e| u64::from(e.uncompressed_length))
        .sum::<u64>();
    let straddlers = long.as_ref().map_or(0, StraddlerIndex::index_bytes);
    let log = r.source().requests();
    Measured {
        directory_ready,
        first_drawing,
        ranges: (log.len() - requests0) as u64,
        advanced_ranges: (log.len() - requests_before_advanced) as u64,
        bytes: log[requests0..].iter().map(|q| u64::from(q.len)).sum(),
        blocks: blocks.len() as u64,
        decode_work: acct.work() - work0,
        peak_resident: acct.peak() + parsed + straddlers,
        index_bytes: live_open + parsed + straddlers,
        advanced_set_bytes: acct.peak().saturating_sub(peak_before),
        advanced_stored,
        advanced_decoded,
        held: live_before.saturating_sub(live_open),
    }
}

// ---------------------------------------------------------------- the load-shed scenario

/// What the throttled source charges per range: a latency and a bandwidth.
const SHED_LATENCY: Duration = Duration::from_millis(8);
const SHED_BYTES_PER_SEC: u64 = 100 << 20;

/// A source that answers like a slow link: a fixed latency and a bandwidth, per range.
struct Throttled<S> {
    inner: S,
    latency: Duration,
    bytes_per_sec: u64,
}

impl<S: BlockSource> BlockSource for Throttled<S> {
    fn len(&self) -> u64 {
        self.inner.len()
    }
    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        std::thread::sleep(
            self.latency + Duration::from_secs_f64(f64::from(len) / self.bytes_per_sec as f64),
        );
        self.inner.read_range(offset, len)
    }
}

pub struct Shed {
    pub changes: u32,
    pub jobs_cancelled: u64,
    pub units_planned: u64,
    pub units_started: u64,
    pub units_superseded: u64,
    pub units_discarded: u64,
    pub stale_applied: u64,
    pub final_matches_last_viewport: bool,
}

impl Shed {
    pub fn shed_rate(&self) -> f64 {
        self.units_discarded as f64 / self.units_superseded as f64
    }
    pub fn useful_fraction(&self) -> f64 {
        1.0 - self.units_discarded as f64 / self.units_started as f64
    }
    pub fn units_per_viewport(&self) -> f64 {
        self.units_planned as f64 / f64::from(self.changes)
    }
    pub fn line(&self) -> String {
        format!(
            "{} changes, {} jobs cancelled, units: {:.1} per viewport, {} planned, {} started, {} superseded in flight, {} discarded; shed_rate {:.3}, useful_fraction {:.3}",
            self.changes,
            self.jobs_cancelled,
            self.units_per_viewport(),
            self.units_planned,
            self.units_started,
            self.units_superseded,
            self.units_discarded,
            self.shed_rate(),
            self.useful_fraction()
        )
    }
}

/// What the worker should do next: the newest job only. A job still waiting here when the next change
/// arrives is replaced, not queued behind it.
struct Slot {
    job: Option<(pvec_stream::CancelToken, Vec<BlockId>)>,
    closed: bool,
}

/// The scene: the blocks of the newest generation that finished. `apply` is the barrier: a result of
/// an older generation is refused.
struct Scene {
    generation: u64,
    blocks: Vec<BlockId>,
}

/// `changes` viewport changes spread over `seconds`, on a throttled source: each change advances the
/// generation (cancelling the one before), plans the new viewport and hands the job to the one worker.
/// A unit is one block read. Viewports are 3 x 3 rooms, one room further along a diagonal each time.
pub fn load_shed(changes: u32, seconds: u64) -> Shed {
    let bytes = artifact(Kind::Mixed, MILLION);
    let source = Throttled {
        inner: MemorySource::new(bytes),
        latency: SHED_LATENCY,
        bytes_per_sec: SHED_BYTES_PER_SEC,
    };
    load_shed_over(source, changes, seconds)
}

/// The scenario over `source`, which has to charge for every range itself.
fn load_shed_over<S: BlockSource + Send + Sync + 'static>(source: S, changes: u32, seconds: u64) -> Shed {
    // No cache: every unit is a range read, so a unit is what the throttle charges for.
    let cfg = ReaderConfig { cache_bytes: 0, ..ReaderConfig::default() };
    let reader = Arc::new(ContainerReader::open(source, cfg).expect("opens"));
    let dir = SpatialDirectory::open(&reader).expect("directory").expect("a directory");

    let viewport = |k: u32| {
        let o = ROOM * (2.0 + f64::from(k % 40));
        bx(o, o, o + 3.0 * ROOM, o + 3.0 * ROOM)
    };
    let units_of = |k: u32| query(viewport(k)).plan(&dir, &*reader, &no_margin()).expect("plan").block_ids();

    let clock = Arc::new(GenerationClock::new());
    let slot = Arc::new((Mutex::new(Slot { job: None, closed: false }), Condvar::new()));
    let scene = Arc::new(Mutex::new(Scene { generation: 0, blocks: Vec::new() }));
    let started = Arc::new(AtomicU64::new(0));
    let superseded = Arc::new(AtomicU64::new(0));
    let stale_applied = Arc::new(AtomicU64::new(0));

    let worker = {
        let (reader, clock, slot, scene) =
            (Arc::clone(&reader), Arc::clone(&clock), Arc::clone(&slot), Arc::clone(&scene));
        let (started, superseded, stale_applied) =
            (Arc::clone(&started), Arc::clone(&superseded), Arc::clone(&stale_applied));
        std::thread::spawn(move || {
            loop {
                let (token, units) = {
                    let (lock, cv) = &*slot;
                    let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
                    loop {
                        if let Some(job) = s.job.take() {
                            break job;
                        }
                        if s.closed {
                            return;
                        }
                        s = cv.wait(s).unwrap_or_else(|e| e.into_inner());
                    }
                };
                let outcome = run_job(&token, units.clone(), |id| {
                    started.fetch_add(1, Ordering::Relaxed);
                    let block = reader.read_block(id);
                    // The unit was in flight when its generation was superseded.
                    if token.is_cancelled() {
                        superseded.fetch_add(1, Ordering::Relaxed);
                    }
                    block.map(|b| b.id())
                })
                .expect("reads");
                if let JobOutcome::Complete(done) = outcome {
                    let mut s = scene.lock().unwrap_or_else(|e| e.into_inner());
                    if token.generation().value() >= s.generation {
                        s.generation = token.generation().value();
                        s.blocks = done;
                        if token.generation() != clock.current() {
                            stale_applied.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        })
    };

    let mut planned = 0u64;
    let begin = Instant::now();
    let tick = Duration::from_secs(seconds) / changes;
    for k in 0..changes {
        std::thread::sleep((begin + tick * k).saturating_duration_since(Instant::now()));
        let units = units_of(k);
        planned += units.len() as u64;
        let token = clock.advance();
        let (lock, cv) = &*slot;
        lock.lock().unwrap_or_else(|e| e.into_inner()).job = Some((token, units));
        cv.notify_one();
    }
    // The last viewport is left to finish; nothing supersedes it.
    let deadline = Instant::now() + Duration::from_secs(60);
    let want = units_of(changes - 1);
    let final_matches = loop {
        {
            let s = scene.lock().unwrap_or_else(|e| e.into_inner());
            if s.generation == clock.current().value() {
                break s.blocks == want;
            }
        }
        if Instant::now() > deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    {
        let (lock, cv) = &*slot;
        lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
        cv.notify_all();
    }
    worker.join().expect("worker");

    let m = clock.metrics();
    Shed {
        changes,
        jobs_cancelled: m.jobs_cancelled,
        units_planned: planned,
        units_started: started.load(Ordering::Relaxed),
        units_superseded: superseded.load(Ordering::Relaxed),
        units_discarded: m.work_discarded,
        stale_applied: stale_applied.load(Ordering::Relaxed),
        final_matches_last_viewport: final_matches,
    }
}

// ---------------------------------------------------------------- the hermetic range source

/// The image the range source runs in: the multi-platform index of `python:3.12-slim` (CPython 3.12.15),
/// pinned by digest as `pvec-interchange`'s STEP reader test pins it. The server is `range_server.py`,
/// mounted read-only beside the artifact.
const RANGE_SOURCE_IMAGE: &str =
    "python@sha256:a6e34c598f2467ed0e9a8d349809fcd8b5c603269512df273a0bb1784edc11b1";

/// The pipes of the one `docker exec -i` that talks to `range_server.py`, and the artifact's length.
/// Cloning is cheap; the helper is killed when the last clone is dropped.
#[derive(Clone)]
pub struct HermeticSource {
    inner: Arc<Pipes>,
}

struct Pipes {
    io: Mutex<(std::process::ChildStdin, std::io::BufReader<std::process::ChildStdout>)>,
    child: Mutex<std::process::Child>,
    len: u64,
}

impl Drop for Pipes {
    fn drop(&mut self) {
        let child = self.child.get_mut().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn io_err(e: impl std::fmt::Display) -> SourceError {
    SourceError::Io(e.to_string())
}

/// One request line and its answer: `OK <n>` then `n` bytes, or `ERR <reason>`.
fn ask(
    io: &mut (std::process::ChildStdin, std::io::BufReader<std::process::ChildStdout>),
    request: &str,
) -> Result<Vec<u8>, SourceError> {
    use std::io::{BufRead, Read, Write};
    writeln!(io.0, "{request}").and_then(|()| io.0.flush()).map_err(io_err)?;
    let mut head = String::new();
    if io.1.read_line(&mut head).map_err(io_err)? == 0 {
        return Err(SourceError::Io("the range source closed".into()));
    }
    let head = head.trim_end();
    let Some(n) = head.strip_prefix("OK ").and_then(|n| n.parse::<usize>().ok()) else {
        return Err(SourceError::RangeNotHonoured { detail: head.to_string() });
    };
    if request == "L" {
        return Ok(n.to_string().into_bytes());
    }
    let mut body = vec![0u8; n];
    io.1.read_exact(&mut body).map_err(io_err)?;
    Ok(body)
}

impl BlockSource for HermeticSource {
    fn len(&self) -> u64 {
        self.inner.len
    }
    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        if offset.saturating_add(u64::from(len)) > self.inner.len {
            return Err(SourceError::OutOfRange { offset, len: u64::from(len), source_len: self.inner.len });
        }
        let mut io = self.inner.io.lock().unwrap_or_else(|e| e.into_inner());
        let body = ask(&mut io, &format!("R {offset} {len}"))?;
        if body.len() != len as usize {
            return Err(SourceError::Short { wanted: u64::from(len), got: body.len() as u64 });
        }
        Ok(Bytes::new(body))
    }
}

/// The scratch directory the container mounts, removed on drop.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        // The test's own scratch, named in full.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs `f` with a range source whose throttle runs inside a hermetic container (`pvec-hermetic`: no
/// network, a read-only root, the artifact mounted read-only), and gives it the container's name. The
/// container is removed when `f` returns or panics. `None` when Docker is not there and this is a
/// developer run (`PVEC_DEV_RUN`); anywhere else a container that cannot start fails.
pub fn with_hermetic_source<R>(
    bytes: &[u8],
    latency: Duration,
    bytes_per_sec: u64,
    f: impl FnOnce(&HermeticSource, &str) -> R,
) -> Option<R> {
    use pvec_hermetic::{Config, Container, Decision, Policy};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pvec-range-source-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).expect("scratch");
    let scratch = Scratch(dir);
    std::fs::write(scratch.0.join("artifact"), bytes).expect("artifact");
    std::fs::write(scratch.0.join("range_server.py"), include_str!("range_server.py")).expect("script");
    let cfg = Config { image: RANGE_SOURCE_IMAGE.to_string(), mount: scratch.0.clone(), search_path: None };
    // `sleep` keeps the container alive; the server is one `exec -i` into it.
    let container = match Container::start_with_command(&cfg, &["sleep", "3600"]) {
        Ok(c) => c,
        Err(e) => match Policy::from_env().decide(e) {
            Decision::Skip(why) => {
                eprintln!("skipped: {why}");
                return None;
            }
            Decision::Fail(e) => panic!("the range source container could not start: {e}"),
        },
    };
    let millis = latency.as_millis().to_string();
    let rate = bytes_per_sec.to_string();
    let mut child = container
        .exec_stream(&["python3", "/corpus/range_server.py", &millis, &rate])
        .expect("the range server starts");
    let mut io =
        (child.stdin.take().expect("stdin"), std::io::BufReader::new(child.stdout.take().expect("stdout")));
    let len =
        String::from_utf8(ask(&mut io, "L").expect("the length")).expect("text").parse().expect("number");
    assert_eq!(len, bytes.len() as u64, "the container serves the artifact it was given");
    let source =
        HermeticSource { inner: Arc::new(Pipes { io: Mutex::new(io), child: Mutex::new(child), len }) };
    let r = f(&source, container.name());
    drop(source);
    container.stop().expect("the container is removed");
    Some(r)
}

/// As [`load_shed`], with the throttle in a hermetic container. `None` when the run is skipped.
pub fn load_shed_hermetic(changes: u32, seconds: u64) -> Option<Shed> {
    let bytes = artifact(Kind::Mixed, MILLION);
    with_hermetic_source(&bytes, SHED_LATENCY, SHED_BYTES_PER_SEC, |source, _| {
        load_shed_over(source.clone(), changes, seconds)
    })
}

// ---------------------------------------------------------------- the results file

fn host_label() -> &'static str {
    if cfg!(target_arch = "x86_64") {
        "native x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "other"
    }
}

/// The measurements as a Markdown section for the host the test runs on. The host is named by
/// `PVEC_RESULTS_HOST` (mars, nas, mercury); the architecture comes from the build.
pub fn results_markdown(changes: u32, seconds: u64) -> String {
    let host = std::env::var("PVEC_RESULTS_HOST").unwrap_or_else(|_| "unnamed host".into());
    let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
    let mut s = format!("## {} ({host}, {profile} build)\n\n", host_label());
    s.push_str("| fixture | entities | viewport | directory ready ms | first drawing ms | ranges | bytes | decode work | blocks | peak resident | index bytes | advanced_set_bytes | resident - advanced | held |\n");
    s.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    for kind in Kind::ALL {
        for n in [SMALL, MILLION] {
            for scale in [Scale::Wide, Scale::Mid, Scale::Tight] {
                let m = measure(kind, n, scale);
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {:.1} | {:.1} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                    kind.name(),
                    n,
                    scale.name(),
                    m.directory_ready.as_secs_f64() * 1000.0,
                    m.first_drawing.as_secs_f64() * 1000.0,
                    m.ranges,
                    m.bytes,
                    m.decode_work,
                    m.blocks,
                    m.peak_resident,
                    m.index_bytes,
                    m.advanced_set_bytes,
                    m.resident_without_advanced(),
                    m.held
                );
            }
        }
    }
    let r = load_shed(changes, seconds);
    let _ = write!(
        s,
        "\nLoad shed ({changes} viewport changes in {seconds} s, 8 ms + 100 MB/s per range, no block cache): units per viewport {:.1}, \
         units_planned {}, units_started {}, units_superseded {}, units_discarded {}, jobs cancelled {}; \
         shed_rate {:.3}, useful_fraction {:.3}; final scene is the last viewport: {}.\n\n",
        r.units_per_viewport(),
        r.units_planned,
        r.units_started,
        r.units_superseded,
        r.units_discarded,
        r.jobs_cancelled,
        r.shed_rate(),
        r.useful_fraction(),
        r.final_matches_last_viewport
    );
    s
}
