//! Long entities and straddlers (issue P5.3, pvec#12).
//!
//! Each test is named for the acceptance criterion it holds the strategy to. The fixtures are generated
//! here from a seeded splitmix64, like the ones of issue 10: `drawing-100k` is the criterion's fixture
//! (1,000 drawing-spanning entities among 100,000 local ones), `stress-long` has a third of its entities
//! long at random positions and lengths, `rooms-1m` is the million-entity shape with a few thousand
//! spanning lines, `gap` is two clusters with lines that cross the empty space between them, and `tied`
//! has nothing long at all.

use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Limits, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, BuiltDirectory, CellId, DuplicationPolicy, EntityBox, HEADER_LEN, LevelScale, LongEntity, NODE_LEN,
    PAGE_LEN, PayloadRef, PlacedEntity, STRADDLER_ENTRY_LEN, STRADDLER_FACTOR, STRADDLER_HEADER_LEN,
    STRADDLER_PAGE, STRADDLER_RULES, SpatialDirectory, StraddlerError, StraddlerLists, ViewportHits,
    ancestor_placement, build_directory, build_straddlers, classify, congestion, extent, query_viewport,
    unique_entities,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::OnceLock;

/// The leaf size these fixtures are built for (the 64 of P1.6). The shipped default is `CELL_CAPACITY`.
const LEAF: u32 = 64;

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

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

fn three(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// A grid of `side` x `side` rooms of `per_room` plan-like entities, `room` units wide, as in issue 10.
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
                let id = eid(out.len() as u32);
                out.push(EntityBox { id, bounds: bx(three(x), three(y), three(x + w), three(y + h)) });
            }
        }
    }
    out
}

/// `n` lines that span the whole drawing, half horizontal and half vertical, at random offsets.
fn spanning(entities: &mut Vec<EntityBox>, seed: u64, n: u32, world: f64) {
    let mut rng = Rng(seed);
    for k in 0..n {
        let t = three(rng.unit() * world);
        let id = eid(entities.len() as u32);
        let bounds = if k % 2 == 0 { bx(0.0, t, world, t) } else { bx(t, 0.0, t, world) };
        entities.push(EntityBox { id, bounds });
    }
}

struct Fixture {
    name: &'static str,
    all: Vec<EntityBox>,
    world: f64,
}

fn drawing_100k() -> Fixture {
    let mut all = rooms(12, 20, 250, 500.0);
    assert_eq!(all.len(), 100_000);
    spanning(&mut all, 77, 1000, 10_000.0);
    Fixture { name: "drawing-100k", all, world: 10_000.0 }
}

fn stress_long() -> Fixture {
    let mut all = rooms(13, 10, 200, 500.0);
    let world = 5000.0;
    let mut rng = Rng(5);
    for _ in 0..10_000 {
        let len = three((0.2 + 0.8 * rng.unit()) * world);
        let (x, y) = (three(rng.unit() * (world - len)), three(rng.unit() * world));
        let id = eid(all.len() as u32);
        let bounds = if rng.next() % 2 == 0 { bx(x, y, x + len, y) } else { bx(y, x, y, x + len) };
        all.push(EntityBox { id, bounds });
    }
    Fixture { name: "stress-long", all, world }
}

fn rooms_1m() -> Fixture {
    let mut all = rooms(1, 50, 400, 2000.0);
    let world = 100_000.0;
    spanning(&mut all, 3, 1000, world);
    // lines of two other lengths, so the straddlers use more than one level
    let mut rng = Rng(8);
    for k in 0..1000 {
        let len = three(if k % 2 == 0 {
            8_000.0 + 22_000.0 * rng.unit()
        } else {
            45_000.0 + 55_000.0 * rng.unit()
        });
        let (x, y) = (three(rng.unit() * (world - len)), three(rng.unit() * world));
        let id = eid(all.len() as u32);
        all.push(EntityBox { id, bounds: bx(x, y, x + len, y) });
    }
    Fixture { name: "rooms-1m", all, world }
}

fn rooms_1m_split() -> &'static Split {
    static CELL: OnceLock<Split> = OnceLock::new();
    CELL.get_or_init(|| split(&rooms_1m().all))
}

/// Two clusters 6,000 units apart with lines that cross the empty space between them.
fn gap() -> Fixture {
    let mut all = rooms(21, 3, 200, 500.0);
    let shifted: Vec<EntityBox> = rooms(22, 3, 200, 500.0)
        .into_iter()
        .enumerate()
        .map(|(i, e)| EntityBox {
            id: eid((all.len() + i) as u32),
            bounds: bx(
                e.bounds.min()[0] + 7500.0,
                e.bounds.min()[1],
                e.bounds.max()[0] + 7500.0,
                e.bounds.max()[1],
            ),
        })
        .collect();
    all.extend(shifted);
    for (k, y) in [200.0, 700.0, 1300.0].into_iter().enumerate() {
        let id = eid(all.len() as u32);
        let _ = k;
        all.push(EntityBox { id, bounds: bx(0.0, y, 9000.0, y) });
    }
    Fixture { name: "gap", all, world: 9000.0 }
}

fn tied() -> Fixture {
    let all = (0..640).map(|i| EntityBox { id: eid(i), bounds: bx(10.0, 10.0, 20.0, 20.0) }).collect();
    Fixture { name: "tied", all, world: 20.0 }
}

fn payload(id: EntityId) -> PayloadRef {
    PayloadRef { block_id: BlockId(1 + id.index() / 4096), slot: id.index() % 4096 }
}

fn long_entities(c: &[EntityBox]) -> Vec<LongEntity> {
    c.iter().map(|e| LongEntity { id: e.id, bounds: e.bounds, payload: payload(e.id) }).collect()
}

/// A fixture classified, its local directory built and its straddler block written and parsed.
struct Split {
    local: Vec<EntityBox>,
    long: Vec<EntityBox>,
    built: BuiltDirectory,
    block: Vec<u8>,
    lists: StraddlerLists,
}

fn split(all: &[EntityBox]) -> Split {
    let c = classify(all, STRADDLER_FACTOR, LEAF).expect("classified");
    let built = build_directory(&c.local, LEAF).expect("directory");
    let block = build_straddlers(&c.scale, &long_entities(&c.long)).expect("straddlers").bytes;
    let lists = StraddlerLists::parse(&block, &Limits::DEFAULT).expect("a block the builder wrote");
    Split { local: c.local, long: c.long, built, block, lists }
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/12".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn to_bounds(b: Aabb) -> Bounds {
    Bounds { min: b.min(), max: b.max() }
}

/// An artifact of `entities` as lines, each in its directory cell.
fn artifact(entities: &[EntityBox], built: &BuiltDirectory) -> Vec<u8> {
    let mut w = ContainerWriter::new(config());
    for (e, cell) in entities.iter().zip(&built.cells) {
        let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
        w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
    }
    w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
    w.finish().unwrap()
}

type Mem = Recording<MemorySource<Vec<u8>>>;

fn reader(bytes: Vec<u8>) -> ContainerReader<Mem> {
    ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default()).unwrap()
}

/// Stored bytes of the hot blocks of the cells a viewport hits, and how many cells.
fn cell_bytes(r: &ContainerReader<Mem>, hits: &ViewportHits) -> u64 {
    hits.cells
        .iter()
        .flat_map(|c| (c.blocks.first.0..c.blocks.first.0 + c.blocks.count).map(BlockId))
        .map(|b| u64::from(r.entries()[b.0 as usize].stored_length))
        .sum()
}

fn hot_bytes_per_entity(r: &ContainerReader<Mem>) -> f64 {
    let (bytes, count) = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Hot))
        .fold((0u64, 0u64), |(b, c), e| (b + u64::from(e.stored_length), c + u64::from(e.count)));
    bytes as f64 / count as f64
}

fn cell_entities(s: &Split, hits: &ViewportHits) -> Vec<EntityId> {
    let wanted: BTreeSet<CellId> = hits.cells.iter().map(|c| c.id).collect();
    s.local.iter().zip(&s.built.cells).filter(|(_, c)| wanted.contains(c)).map(|(e, _)| e.id).collect()
}

fn viewport(cx: f64, cy: f64, size: f64) -> Aabb {
    bx(cx - size / 2.0, cy - size / 2.0, cx + size / 2.0, cy + size / 2.0)
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

fn rule(r: Result<impl std::fmt::Debug, StraddlerError>) -> &'static str {
    r.expect_err("the block must be refused").rule()
}

fn parse(bytes: &[u8]) -> Result<StraddlerLists, StraddlerError> {
    StraddlerLists::parse(bytes, &Limits::DEFAULT)
}

/// Level, entity id of every entry of a block, read from the bytes and not through the parser.
fn raw_entries(bytes: &[u8]) -> Vec<(u16, u32)> {
    let lists = get32(bytes, 12) as usize;
    let base = STRADDLER_HEADER_LEN + PAGE_LEN * lists;
    let mut out = Vec::new();
    let mut k = 0usize;
    for i in 0..lists {
        let at = STRADDLER_HEADER_LEN + PAGE_LEN * i;
        let level = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        for _ in 0..get32(bytes, at + 4) {
            out.push((level, get32(bytes, base + STRADDLER_ENTRY_LEN * k + 32)));
            k += 1;
        }
    }
    out
}

/// A small block with entries at all three levels of a scale 1 / 10 / 100 at factor 4: extents 5, 50 and
/// 500, three each.
fn three_levels() -> (Vec<u8>, LevelScale) {
    let scale = LevelScale::new(4.0, vec![1.0, 10.0, 100.0]).unwrap();
    let mut long = Vec::new();
    for (i, len) in [5.0, 50.0, 500.0].into_iter().enumerate() {
        for k in 0..3u32 {
            let id = eid(10 * (i as u32 + 1) + k);
            let y = f64::from(k) * 7.0;
            long.push(LongEntity { id, bounds: bx(0.0, y, len, y), payload: payload(id) });
        }
    }
    (build_straddlers(&scale, &long).unwrap().bytes, scale)
}

// ---------------------------------------------------------------- classification

#[test]
fn the_factor_is_a_named_constant_and_an_entity_is_long_by_it() {
    assert_eq!(STRADDLER_FACTOR, 4.0);
    let scale = LevelScale::new(STRADDLER_FACTOR, vec![1.0, 10.0, 100.0]).unwrap();
    // at most four cells of a level is not long there, more than four is
    assert_eq!(scale.level_of(4.0), None);
    assert_eq!(scale.level_of(4.0001), Some(0));
    assert_eq!(scale.level_of(40.0), Some(0));
    assert_eq!(scale.level_of(40.001), Some(1));
    assert_eq!(scale.level_of(400.0), Some(1));
    assert_eq!(scale.level_of(400.001), Some(2));
    assert_eq!(scale.level_of(1_000_000_000.0), Some(2), "the coarsest level there is");
    // never a finer level than the coarsest that applies: the level only grows with the extent
    let mut last = None;
    for k in 0..2000 {
        let l = scale.level_of(f64::from(k) * 0.5);
        assert!(l >= last, "level fell at extent {}", f64::from(k) * 0.5);
        last = l;
    }
    assert!(LevelScale::new(1.0, vec![1.0]).is_err(), "a factor of 1 makes every entity long");
    assert!(LevelScale::new(4.0, vec![]).is_err());
    assert!(LevelScale::new(4.0, vec![0.0]).is_err());
    assert!(LevelScale::new(f64::NAN, vec![1.0]).is_err());
}

#[test]
fn classification_keeps_plan_entities_local_and_lifts_the_spanning_lines() {
    let f = drawing_100k();
    let c = classify(&f.all, STRADDLER_FACTOR, LEAF).unwrap();
    assert_eq!(c.local.len(), 100_000, "every room entity, wall included, stays in a cell");
    assert_eq!(c.long.len(), 1000);
    assert!(c.local.iter().all(|e| e.id.index() < 100_000));
    assert!(c.long.iter().all(|e| extent(&e.bounds) >= f.world));
    // input order is kept, so the split does not depend on anything but the input
    assert!(c.local.windows(2).all(|w| w[0].id < w[1].id));
    assert!(c.long.windows(2).all(|w| w[0].id < w[1].id));
    // spanning lines are listed below the root: a viewport that reads the root does not read them
    let built = build_directory(&c.local, LEAF).unwrap();
    let top = (c.scale.sizes().len() - 1) as u16;
    for e in &c.long {
        let level = c.scale.level_of(extent(&e.bounds)).unwrap();
        assert!(level < top, "a spanning line is at the root level {level}");
    }
    assert_eq!(c.scale.sizes().len(), 4, "100,000 entities at 64 per cell and fanout 32 is four levels");
    let _ = built;
}

#[test]
fn the_estimated_cell_sizes_match_the_cells_of_the_directory_over_the_local_entities() {
    // `drawing-100k` only: on `stress-long` the lines between one and four cells long stay in cells by
    // the factor, and they do swell the cells they are in, which the next test measures
    for f in [drawing_100k()] {
        let c = classify(&f.all, STRADDLER_FACTOR, LEAF).unwrap();
        let built = build_directory(&c.local, LEAF).unwrap();
        // the longer side of every node, by level, read from the builder's own bytes
        let mut sides: BTreeMap<u16, Vec<f64>> = BTreeMap::new();
        for i in 0..built.nodes as usize {
            let at = HEADER_LEN + NODE_LEN * i;
            let f64_at =
                |k: usize| f64::from_le_bytes(built.bytes[at + 8 * k..at + 8 * k + 8].try_into().unwrap());
            let side = (f64_at(2) - f64_at(0)).max(f64_at(3) - f64_at(1));
            let level = u16::from_le_bytes([built.bytes[at + 44], built.bytes[at + 45]]);
            sides.entry(level).or_default().push(side);
        }
        for (level, mut v) in sides {
            v.sort_by(f64::total_cmp);
            let median = v[v.len() / 2];
            let est = c.scale.sizes()[usize::from(level)];
            let ratio = est / median;
            eprintln!(
                "{} level {level}: estimate {est:.1}, median node side {median:.1}, ratio {ratio:.2}",
                f.name
            );
            if v.len() >= 3 && est < 0.9 * f.world {
                assert!((1.0 / 3.0..=3.0).contains(&ratio), "{} level {level}: ratio {ratio}", f.name);
            }
        }
        // and what stays local is no longer than the factor allows at the finest level
        let s0 = c.scale.sizes()[0];
        let over = c.local.iter().filter(|e| extent(&e.bounds) > STRADDLER_FACTOR * s0).count();
        assert_eq!(over, 0, "{}: {over} local entities are long at level 0", f.name);
    }
}

#[test]
fn classification_does_not_depend_on_arrival_order_or_on_how_many_straddlers_there_are() {
    // `stress-long`: a third of the entities are long, at random positions and lengths, so almost every
    // leaf of a directory over everything is swollen and a cell size read from it would be useless
    let f = stress_long();
    let c = classify(&f.all, STRADDLER_FACTOR, LEAF).unwrap();
    eprintln!(
        "stress-long: {} local, {} long, level sizes {:?}",
        c.local.len(),
        c.long.len(),
        c.scale.sizes()
    );
    assert!(c.local.len() >= 20_000, "{} local: every room entity should stay in a cell", c.local.len());
    assert!(c.long.len() >= 7_500, "{} long of 10,000 random lines of 1,000 to 5,000", c.long.len());
    assert!(c.local.iter().all(|e| extent(&e.bounds) <= STRADDLER_FACTOR * c.scale.sizes()[0]));
    assert!(c.long.iter().all(|e| extent(&e.bounds) > STRADDLER_FACTOR * c.scale.sizes()[0]));
    let mut shuffled = f.all.clone();
    let mut rng = Rng(31);
    for i in (1..shuffled.len()).rev() {
        shuffled.swap(i, (rng.next() % (i as u64 + 1)) as usize);
    }
    let d = classify(&shuffled, STRADDLER_FACTOR, LEAF).unwrap();
    let ids = |v: &[EntityBox]| v.iter().map(|e| e.id).collect::<BTreeSet<_>>();
    assert_eq!(ids(&c.local), ids(&d.local));
    assert_eq!(ids(&c.long), ids(&d.long));
    assert_eq!(c.scale, d.scale);
    // a drawing of points has nothing to measure and keeps everything in cells
    let points: Vec<EntityBox> = (0..200)
        .map(|i| EntityBox { id: eid(i), bounds: bx(f64::from(i), 0.0, f64::from(i), 0.0) })
        .collect();
    let p = classify(&points, STRADDLER_FACTOR, LEAF).unwrap();
    assert_eq!((p.local.len(), p.long.len()), (200, 0));
    assert!(classify(&[], STRADDLER_FACTOR, LEAF).is_err());
}

#[test]
fn straddlers_keep_the_cells_tight_where_long_entities_are_scattered_among_local_ones() {
    // on `stress-long` the long entities' centres are among the local ones, so with them in the cells
    // every viewport pays for swollen cells; with them in lists it pays for the pages it touches
    let f = stress_long();
    let s = split(&f.all);
    let (qb, _) = split_status_quo(&f.all);
    let (r, qr) = (reader(artifact(&s.local, &s.built)), reader(qb));
    let (dir, qdir) =
        (SpatialDirectory::open(&r).unwrap().unwrap(), SpatialDirectory::open(&qr).unwrap().unwrap());
    let per_entity = hot_bytes_per_entity(&r);
    let (mut ours, mut theirs, mut our_cells, mut their_cells) = (0u64, 0u64, 0usize, 0usize);
    let mut rng = Rng(6);
    for _ in 0..60 {
        let q = viewport(rng.unit() * f.world, rng.unit() * f.world, 250.0);
        let h = query_viewport(&dir, Some(&s.lists), &q);
        let qh = query_viewport(&qdir, None, &q);
        ours += cell_bytes(&r, &h) + s.lists.read_bytes(&q) + (per_entity * h.long.len() as f64) as u64;
        theirs += cell_bytes(&qr, &qh);
        our_cells += h.cells.len();
        their_cells += qh.cells.len();
    }
    eprintln!(
        "stress-long, 60 viewports of 250: straddlers {ours} B in {our_cells} cells; everything in cells {theirs} B in {their_cells} cells"
    );
    assert!(our_cells < their_cells, "{our_cells} cells against {their_cells}");
    assert!(ours < theirs, "{ours} B against {theirs} B");
}

// ---------------------------------------------------------------- acceptance: bytes and the root

/// The stated byte budget for a one-screen viewport query on `drawing-100k`.
const VIEWPORT_BYTE_BUDGET: u64 = 64 * 1024;
/// The stated fraction of the artifact the same query may transfer.
const VIEWPORT_ARTIFACT_FRACTION: f64 = 0.02;
/// The stated ceiling on the references the root level carries: what one cell may hold.
const ROOT_REFERENCE_CEILING: u64 = LEAF as u64;

#[test]
fn a_one_screen_viewport_on_drawing_100k_stays_under_a_byte_budget_and_a_fraction_of_the_artifact() {
    let f = drawing_100k();
    let s = split(&f.all);
    assert_eq!(s.long.len(), 1000);
    let local_bytes = artifact(&s.local, &s.built);
    let r = reader(local_bytes);
    let per_entity = hot_bytes_per_entity(&r);
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();

    // the whole artifact: the file of the local entities, the straddler block, the straddlers' payloads
    let long_payload = (per_entity * s.long.len() as f64).ceil() as u64;
    let artifact_bytes = r.source().len_hint() + s.block.len() as u64 + long_payload;

    let screen = viewport(5000.0, 5000.0, 500.0);
    let before = r.source().bytes_requested();
    let hits = query_viewport(&dir, Some(&s.lists), &screen);
    let cells = cell_bytes(&r, &hits);
    for c in &hits.cells {
        for b in c.blocks.first.0..c.blocks.first.0 + c.blocks.count {
            r.read_block(BlockId(b)).unwrap();
        }
    }
    let read = r.source().bytes_requested() - before;
    assert!(read >= cells, "the reader fetched {read} bytes for {cells} bytes of blocks");
    let payloads = (per_entity * hits.long.len() as f64).ceil() as u64;
    let list = s.lists.read_bytes(&screen);
    let transferred = cells + list + payloads;

    // the same screen against the status quo (every entity in a cell, P5.1 alone)
    let q = split_status_quo(&f.all);
    let qr = reader(q.0);
    let qdir = SpatialDirectory::open(&qr).unwrap().unwrap();
    let qhits = query_viewport(&qdir, None, &screen);
    let status_quo = cell_bytes(&qr, &qhits);

    // and against promoting every long entity to the root cell, which a viewport always reads whole
    let naive = cells + long_payload;

    eprintln!(
        "drawing-100k one screen: straddler {transferred} B ({} cells {cells} B, list {list} B of a {} B block, {} payloads {payloads} B) of {artifact_bytes} B = {:.2} %; status quo {} cells {status_quo} B; promote-to-root {naive} B",
        hits.cells.len(),
        s.block.len(),
        hits.long.len(),
        100.0 * transferred as f64 / artifact_bytes as f64,
        qhits.cells.len(),
    );
    assert!(transferred < VIEWPORT_BYTE_BUDGET, "{transferred} bytes");
    assert!(
        (transferred as f64) < VIEWPORT_ARTIFACT_FRACTION * artifact_bytes as f64,
        "{transferred} bytes of {artifact_bytes}"
    );
    assert!(hits.cells.len() <= qhits.cells.len(), "straddlers swell no cell, so no more cells are hit");
    assert!(cells <= status_quo, "the cells are no bigger without the long entities");
    assert!(
        list < s.block.len() as u64 / 2,
        "a viewport reads pages, not the whole list: {list} of {}",
        s.block.len()
    );
}

/// The status quo artifact: one directory over every entity.
fn split_status_quo(all: &[EntityBox]) -> (Vec<u8>, BuiltDirectory) {
    let built = build_directory(all, LEAF).unwrap();
    (artifact(all, &built), built)
}

trait LenHint {
    fn len_hint(&self) -> u64;
}

impl LenHint for Mem {
    fn len_hint(&self) -> u64 {
        pvec_container::source::BlockSource::len(self)
    }
}

#[test]
fn the_root_reference_count_stays_under_a_ceiling_and_promote_to_ancestor_exceeds_it() {
    let f = drawing_100k();
    let s = split(&f.all);
    let dir_over_local = &s.built;
    let naive = ancestor_placement(dir_over_local, &s.long);
    let top = naive.len() - 1;
    let r = reader(artifact(&s.local, &s.built));
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let report = congestion(&dir, Some(&s.lists));
    eprintln!(
        "drawing-100k root references: promote-to-ancestor {} (by level {naive:?}), straddler lists {} (by level {:?})",
        naive[top],
        report.root_references,
        report.levels.iter().map(|l| l.straddlers).collect::<Vec<_>>()
    );
    assert!(naive[top] > ROOT_REFERENCE_CEILING, "the naive placement put {} in the root", naive[top]);
    assert!(report.root_references <= ROOT_REFERENCE_CEILING, "{} at the root level", report.root_references);
    assert_eq!(naive.iter().sum::<u64>(), 1000);
}

// ---------------------------------------------------------------- acceptance: correctness

#[test]
fn queries_match_a_brute_force_bounds_scan_including_entities_only_a_list_reaches() {
    for f in [drawing_100k(), gap(), stress_long()] {
        let s = split(&f.all);
        let r = reader(artifact(&s.local, &s.built));
        let dir = SpatialDirectory::open(&r).unwrap().unwrap();
        let by_id: BTreeMap<EntityId, Aabb> = f.all.iter().map(|e| (e.id, e.bounds)).collect();
        let mut rng = Rng(99);
        let mut queries = Vec::new();
        for k in 0..120 {
            let size = [f.world, f.world / 4.0, f.world / 16.0, f.world / 200.0, 0.0][k % 5];
            let (x, y) = (rng.unit() * f.world, rng.unit() * f.world);
            queries.push(bx(x, y, x + size, y + size));
        }
        queries.push(bx(-500.0, -500.0, -1.0, -1.0));
        queries.push(bx(f.world, f.world, f.world, f.world));
        if f.name == "gap" {
            // across the empty space between the clusters, on each of the three lines and beside them
            for y in [200.0, 700.0, 1300.0, 450.0] {
                queries.push(bx(4000.0, y - 1.0, 4100.0, y + 1.0));
            }
        }
        let mut only_list = 0usize;
        for q in &queries {
            let hits = query_viewport(&dir, Some(&s.lists), q);
            let got: BTreeSet<EntityId> = unique_entities(cell_entities(&s, &hits), &hits)
                .into_iter()
                .filter(|id| by_id[id].intersects(q))
                .collect();
            let want: BTreeSet<EntityId> =
                f.all.iter().filter(|e| e.bounds.intersects(q)).map(|e| e.id).collect();
            assert_eq!(got, want, "{}: {q:?}", f.name);
            if hits.cells.is_empty() && !want.is_empty() {
                only_list += 1;
            }
        }
        eprintln!("{}: {} queries matched, {only_list} reachable only through a list", f.name, queries.len());
        if f.name == "gap" {
            assert!(only_list > 0, "the gap fixture must have queries only a straddler list answers");
        }
    }
    // the one case spelled out: a viewport in the gap touches no cell and still finds the line
    let f = gap();
    let s = split(&f.all);
    assert_eq!(s.long.len(), 3, "the three lines are the straddlers");
    let r = reader(artifact(&s.local, &s.built));
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let q = bx(4000.0, 650.0, 4100.0, 750.0);
    let hits = query_viewport(&dir, Some(&s.lists), &q);
    assert!(hits.cells.is_empty());
    assert_eq!(hits.long.len(), 1);
    assert_eq!(hits.long[0].bounds, bx(0.0, 700.0, 9000.0, 700.0));
}

// ---------------------------------------------------------------- acceptance: structure and dedup

#[test]
fn a_straddler_is_listed_once_at_its_level_and_never_again_below_it() {
    // the three-level block: ids 10..12 at level 0, 20..22 at level 1, 30..32 at level 2
    let (bytes, _) = three_levels();
    let raw = raw_entries(&bytes);
    assert_eq!(raw.len(), 9);
    for (level, id) in &raw {
        assert_eq!(u32::from(*level) + 1, id / 10, "entity {id} is at level {level}");
    }
    let ids: BTreeSet<u32> = raw.iter().map(|(_, id)| *id).collect();
    assert_eq!(ids.len(), raw.len(), "an entity is in two lists");
    // the same on the emitted block of a real fixture, which has more than one level in use
    let s = rooms_1m_split();
    let raw = raw_entries(&s.block);
    let mut level_of: BTreeMap<u32, Vec<u16>> = BTreeMap::new();
    for (level, id) in &raw {
        level_of.entry(*id).or_default().push(*level);
    }
    assert!(level_of.values().all(|v| v.len() == 1), "an entity is referenced at two levels");
    let used: BTreeSet<u16> = raw.iter().map(|(l, _)| *l).collect();
    assert!(used.len() >= 2, "rooms-1m should use several levels, used {used:?}");
    // an entity at a coarse level is absent from every finer list
    for e in &s.long {
        let want = s.lists.scale().level_of(extent(&e.bounds)).unwrap();
        assert_eq!(level_of[&e.id.index()], vec![want]);
    }
    // and a block that lists one entity at levels 0 and 2 is refused
    let (bytes, _) = three_levels();
    let mut twice = bytes.clone();
    let base = STRADDLER_HEADER_LEN + PAGE_LEN * 3;
    // entry 0 (id 10, level 0) is rewritten to the id of entry 6 (id 30, level 2)
    put32(&mut twice, base + 32, 30);
    assert!(matches!(
        parse(&twice),
        Err(StraddlerError::Placement { .. } | StraddlerError::Duplicate { .. })
    ));
}

#[test]
fn each_entity_is_yielded_once_even_when_it_is_in_a_cell_and_a_list() {
    let f = gap();
    let s = split(&f.all);
    let r = reader(artifact(&s.local, &s.built));
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let q = bx(0.0, 0.0, 9000.0, 1500.0);
    let hits = query_viewport(&dir, Some(&s.lists), &q);
    assert_eq!(hits.long.len(), 3);
    // a writer that referenced a straddler from a cell as well (P5.2's duplicated reference)
    let mut from_cells = cell_entities(&s, &hits);
    let dup = hits.long[1].id;
    from_cells.push(dup);
    from_cells.push(dup);
    from_cells.push(from_cells[0]);
    let all = unique_entities(from_cells.clone(), &hits);
    let set: BTreeSet<EntityId> = all.iter().copied().collect();
    assert_eq!(set.len(), all.len(), "an entity came out twice");
    assert!(set.contains(&dup));
    assert_eq!(all.iter().filter(|id| **id == dup).count(), 1);
    // and the straddler list alone never repeats an entity either
    let ids: Vec<EntityId> = s.lists.query(&q).map(|e| e.id).collect();
    assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
}

#[test]
fn the_cell_ceiling_of_p5_2_hands_over_the_same_entities_and_has_nothing_left_once_they_are_listed() {
    let f = drawing_100k();
    let s = split(&f.all);
    let placed = |entities: &[EntityBox]| -> Vec<PlacedEntity> {
        entities.iter().map(|e| PlacedEntity { id: e.id, bounds: e.bounds, payload: payload(e.id) }).collect()
    };
    // P5.2 alone, with every entity in the directory: each spanning line is over its cell ceiling
    let (bytes, built_all) = split_status_quo(&f.all);
    let all_dir = SpatialDirectory::open(&reader(bytes)).unwrap().unwrap();
    let all = placed(&f.all);
    let alone = pvec_stream::place(&all_dir, &all, DuplicationPolicy::default(), 1).unwrap();
    let _ = built_all;
    let handed: BTreeSet<EntityId> = alone.straddlers.iter().map(|x| x.id).collect();
    let mine: BTreeSet<EntityId> = s.long.iter().map(|e| e.id).collect();
    assert!(mine.is_subset(&handed), "the ceiling must hand over every entity the factor lists");
    // the entities it hands over, with their bounds, are the input of the lists: the same block
    let by_id: BTreeMap<EntityId, &PlacedEntity> = all.iter().map(|p| (p.id, p)).collect();
    let scale = s.lists.scale();
    let from_ceiling: Vec<LongEntity> = alone
        .straddlers
        .iter()
        .map(|x| LongEntity::from(*by_id[&x.id]))
        .filter(|l| scale.level_of(extent(&l.bounds)).is_some())
        .collect();
    assert_eq!(from_ceiling.len(), 1000);
    let rebuilt = build_straddlers(scale, &from_ceiling).unwrap();
    assert_eq!(rebuilt.bytes, s.block, "the lists do not depend on which pass found the entities");
    // over the local directory nothing is left to hand over, so no reference is withheld from a cell
    let local_dir = SpatialDirectory::open(&reader(artifact(&s.local, &s.built))).unwrap().unwrap();
    let after = pvec_stream::place(&local_dir, &placed(&s.local), DuplicationPolicy::default(), 1).unwrap();
    eprintln!(
        "P5.2 over everything hands over {}; over the local entities {}",
        alone.straddlers.len(),
        after.straddlers.len()
    );
    assert!(after.straddlers.is_empty(), "{} entities are still over the ceiling", after.straddlers.len());
}

// ---------------------------------------------------------------- the block

#[test]
fn the_block_round_trips_is_deterministic_and_independent_of_arrival_order() {
    let (bytes, scale) = three_levels();
    let lists = parse(&bytes).unwrap();
    assert_eq!(lists.len(), 9);
    assert_eq!((lists.level_len(0), lists.level_len(1), lists.level_len(2)), (3, 3, 3));
    assert_eq!(lists.scale(), &scale);
    assert_eq!(bytes.len(), STRADDLER_HEADER_LEN + 3 * PAGE_LEN + 9 * STRADDLER_ENTRY_LEN);
    // entries come back finest level first, in page order, with their payload locations
    let ids: Vec<u32> = lists.entries().iter().map(|e| e.id.index()).collect();
    assert_eq!(
        ids.iter().copied().collect::<BTreeSet<_>>(),
        (1..=3).flat_map(|l| (0..3).map(move |k| 10 * l + k)).collect()
    );
    assert!(ids[..3].iter().all(|i| (10..13).contains(i)), "level 0 first: {ids:?}");
    assert!(ids[6..].iter().all(|i| (30..33).contains(i)), "level 2 last: {ids:?}");
    assert_eq!(lists.pages(), 3);
    assert!(lists.entries().iter().all(|e| e.payload == payload(e.id)));
    // order of arrival does not matter
    let mut long = Vec::new();
    for (i, len) in [5.0, 50.0, 500.0].into_iter().enumerate() {
        for k in 0..3u32 {
            let id = eid(10 * (i as u32 + 1) + k);
            let y = f64::from(k) * 7.0;
            long.push(LongEntity { id, bounds: bx(0.0, y, len, y), payload: payload(id) });
        }
    }
    long.reverse();
    assert_eq!(build_straddlers(&scale, &long).unwrap().bytes, bytes);
    // a duplicate id and an entity that is no straddler are refused when building
    let mut twice = long.clone();
    twice.push(twice[0]);
    assert_eq!(rule(build_straddlers(&scale, &twice)), "STR-09");
    let short = [LongEntity { id: eid(1), bounds: bx(0.0, 0.0, 3.0, 0.0), payload: payload(eid(1)) }];
    assert_eq!(rule(build_straddlers(&scale, &short)), "STR-09");
    // a fixture's directory and straddler block do not depend on the order the entities arrive in
    let f = gap();
    let mut shuffled = f.all.clone();
    let mut rng = Rng(4);
    for i in (1..shuffled.len()).rev() {
        shuffled.swap(i, (rng.next() % (i as u64 + 1)) as usize);
    }
    let (a, b) = (split(&f.all), split(&shuffled));
    assert_eq!(a.block, b.block);
    assert_eq!(a.built.bytes, b.built.bytes);
}

#[test]
fn a_block_with_no_straddlers_is_valid_and_empty() {
    let f = tied();
    let s = split(&f.all);
    assert!(s.long.is_empty());
    assert!(s.lists.is_empty());
    assert_eq!(s.block.len(), STRADDLER_HEADER_LEN);
    let r = reader(artifact(&s.local, &s.built));
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let hits = query_viewport(&dir, Some(&s.lists), &bx(0.0, 0.0, 100.0, 100.0));
    assert!(hits.long.is_empty());
    assert_eq!(hits.cells.len(), 10);
}

#[test]
fn every_rule_refuses_the_block_it_names() {
    let (bytes, _) = three_levels();
    let base = STRADDLER_HEADER_LEN + 3 * PAGE_LEN;
    let mut seen: BTreeSet<&'static str> = BTreeSet::new();
    // more entries than P4.8 lets an artifact declare
    let mut c = bytes.clone();
    put32(&mut c, 16, 9);
    let tight = StraddlerLists::parse(&c, &Limits { max_entities_per_artifact: 4, ..Limits::DEFAULT });
    assert_eq!(rule(tight), "STR-03");
    seen.insert("STR-03");
    let mut check = |name: &str, want: &'static str, m: Vec<u8>| {
        let got = rule(parse(&m));
        assert_eq!(got, want, "{name}");
        seen.insert(want);
    };
    check("truncated", "STR-01", bytes[..bytes.len() - 1].to_vec());
    check("short", "STR-01", bytes[..10].to_vec());
    check("empty", "STR-01", Vec::new());
    let mut m = bytes.clone();
    put32(&mut m, 16, u32::MAX);
    check("huge entry count", "STR-01", m);
    let mut m = bytes.clone();
    m[0] = b'X';
    check("magic", "STR-02", m);
    let mut m = bytes.clone();
    m[4] = 2;
    check("version", "STR-02", m);
    let mut m = bytes.clone();
    m[6] = 1;
    check("flags", "STR-02", m);
    let mut m = bytes.clone();
    put32(&mut m, 20, 1);
    check("reserved", "STR-02", m);
    let mut m = bytes.clone();
    put32(&mut m, 8, 0);
    check("no levels", "STR-02", m);
    let mut m = bytes.clone();
    put32(&mut m, 8, 9);
    check("too many levels", "STR-02", m);
    let mut m = bytes.clone();
    put32(&mut m, 8, 2);
    check("sizes declared beyond the level count", "STR-06", m);
    let mut m = bytes.clone();
    m[STRADDLER_HEADER_LEN + 2 * PAGE_LEN] = 5;
    check("a page at a level beyond the level count", "STR-05", m);
    let mut m = bytes.clone();
    put64(&mut m, 24, 1.0);
    check("factor of one", "STR-02", m);
    let mut m = bytes.clone();
    put64(&mut m, 24, f64::NAN);
    check("factor NaN", "STR-02", m);
    let mut m = bytes.clone();
    put64(&mut m, base, f64::INFINITY);
    check("entry bound not finite", "STR-04", m);
    let mut m = bytes.clone();
    put64(&mut m, base, 1_000_000_000.0);
    check("entry bound out of order", "STR-04", m);
    let mut m = bytes.clone();
    put64(&mut m, STRADDLER_HEADER_LEN + 8, f64::NAN);
    check("list bound not finite", "STR-04", m);
    let mut m = bytes.clone();
    put32(&mut m, STRADDLER_HEADER_LEN + 4, 0);
    check("an empty list", "STR-05", m);
    let mut m = bytes.clone();
    m[STRADDLER_HEADER_LEN + 2 * PAGE_LEN] = 0;
    check("levels not ascending", "STR-05", m);
    let mut m = bytes.clone();
    put32(&mut m, STRADDLER_HEADER_LEN + 4, STRADDLER_PAGE as u32 + 1);
    check("a page with more than a page holds", "STR-05", m);
    let mut m = bytes.clone();
    m[STRADDLER_HEADER_LEN + 2] = 1;
    check("list reserved", "STR-05", m);
    let mut m = bytes.clone();
    put32(&mut m, STRADDLER_HEADER_LEN + 4, 2);
    check("entries do not tile", "STR-05", m);
    let mut m = bytes.clone();
    put64(&mut m, STRADDLER_HEADER_LEN + 8, -5.0);
    check("list bounds not the union", "STR-05", m);
    let mut m = bytes.clone();
    put64(&mut m, 32, 0.0);
    check("a zero cell size", "STR-06", m);
    let mut m = bytes.clone();
    put64(&mut m, 32 + 8 * 5, 7.0);
    check("a size beyond the levels", "STR-06", m);
    // an entry at the wrong level: widen the first entry (level 0, extent 5) to extent 50
    let mut m = bytes.clone();
    put64(&mut m, base + 16, 50.0);
    let union_fix = m.clone();
    let _ = union_fix;
    let got = parse(&m).expect_err("refused");
    assert!(matches!(got.rule(), "STR-07" | "STR-05"), "{got:?}");
    // a placement error with consistent list bounds: swap the factor so level 0 entries stop being long
    let mut m = bytes.clone();
    put64(&mut m, 24, 6.0);
    check("not long at its level", "STR-07", m);
    let mut m = bytes.clone();
    put32(&mut m, base + 32, 30);
    check("an id repeated from another level", "STR-08", m);
    let mut m = bytes.clone();
    let first = get32(&m, base + 32);
    put32(&mut m, base + STRADDLER_ENTRY_LEN + 32, first);
    check("a duplicate id", "STR-08", m);
    seen.insert("STR-09");
    // STR-10 to STR-12 are about the block in a container, and `issue_136_straddler_block.rs` refuses them
    for (id, _) in STRADDLER_RULES.iter().filter(|(id, _)| *id < "STR-10") {
        assert!(seen.contains(id), "no test refuses {id}");
    }
}

#[test]
fn hostile_bytes_are_refused_without_a_panic() {
    let (bytes, _) = three_levels();
    let mut rng = Rng(5);
    let everything = bx(-1_000_000_000.0, -1_000_000_000.0, 1_000_000_000.0, 1_000_000_000.0);
    for round in 0..4000 {
        let mut m = bytes.clone();
        for _ in 0..=(round % 4) {
            let at = (rng.next() % m.len() as u64) as usize;
            m[at] ^= 1 << (rng.next() % 8);
        }
        if let Ok(l) = parse(&m) {
            assert!(l.query(&everything).count() <= l.len());
        }
    }
}

// ---------------------------------------------------------------- congestion metrics

#[test]
fn congestion_metrics_are_emitted_for_every_fixture_in_the_phase_5_set() {
    let fixtures: Vec<(&str, Option<Fixture>)> = vec![
        ("drawing-100k", Some(drawing_100k())),
        ("stress-long", Some(stress_long())),
        ("gap", Some(gap())),
        ("tied", Some(tied())),
        ("rooms-1m", None),
    ];
    for (name, f) in fixtures {
        let owned;
        let (s, total) = match &f {
            Some(f) => {
                owned = split(&f.all);
                (&owned, f.all.len())
            }
            None => {
                let s = rooms_1m_split();
                (s, s.local.len() + s.long.len())
            }
        };
        let r = reader(artifact(&s.local, &s.built));
        let dir = SpatialDirectory::open(&r).unwrap().unwrap();
        let rep = congestion(&dir, Some(&s.lists));
        let naive = ancestor_placement(&s.built, &s.long);
        eprintln!(
            "congestion {name}: local {} straddlers {} root refs {} (promote {}) block {} B index {} B coarsest read {} B; per level (cell size, entities, straddlers, pages, list B) {:?}",
            rep.local_entities,
            rep.straddlers,
            rep.root_references,
            naive.last().copied().unwrap_or(0),
            rep.block_bytes,
            rep.index_bytes,
            rep.coarsest_bytes,
            rep.levels
                .iter()
                .map(|l| (l.cell_size.round() as u64, l.entities, l.straddlers, l.pages, l.list_bytes))
                .collect::<Vec<_>>()
        );
        assert_eq!(rep.local_entities, s.local.len() as u64, "{name}");
        assert_eq!(rep.straddlers, s.long.len() as u64, "{name}");
        assert_eq!(rep.local_entities + rep.straddlers, total as u64, "{name}");
        assert_eq!(rep.levels.len(), s.lists.scale().sizes().len(), "{name}");
        assert_eq!(rep.levels.iter().map(|l| l.straddlers).sum::<u64>(), rep.straddlers);
        assert_eq!(rep.levels.iter().map(|l| l.entities).sum::<u64>(), total as u64);
        assert_eq!(rep.levels.iter().map(|l| l.pages).sum::<u64>(), s.lists.pages() as u64);
        assert_eq!(rep.block_bytes, s.block.len() as u64, "{name}");
        assert_eq!(rep.index_bytes, (STRADDLER_HEADER_LEN + PAGE_LEN * s.lists.pages()) as u64);
        assert!(rep.coarsest_bytes >= NODE_LEN as u64 + STRADDLER_HEADER_LEN as u64);
        assert!(rep.coarsest_bytes <= rep.block_bytes + NODE_LEN as u64);
        assert!(rep.root_references <= rep.straddlers);
        assert_eq!(naive.iter().sum::<u64>(), s.long.len() as u64);
    }
    // a file with no straddler block reports zeros for the lists and still the directory's root
    let f = tied();
    let s = split(&f.all);
    let r = reader(artifact(&s.local, &s.built));
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let rep = congestion(&dir, None);
    assert_eq!((rep.straddlers, rep.block_bytes, rep.root_references), (0, 0, 0));
    assert_eq!(rep.coarsest_bytes, NODE_LEN as u64);
}

// ---------------------------------------------------------------- the document

#[test]
fn chapter_three_of_streaming_md_states_the_design_the_layout_and_every_refusal() {
    let doc = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md"))
        .unwrap_or_default();
    for needle in [
        "## 3. Long entities and straddlers",
        "### 3.1 The design",
        "### 3.2 Layout",
        "### 3.3 What a reader validates",
        "### 3.4 Queries",
        "### 3.5 Congestion metrics",
        "### 3.6 Measurements",
        "STRADDLER_FACTOR",
        "ADR 0005",
        "promote",
        "bucket",
    ] {
        assert!(doc.contains(needle), "docs/STREAMING.md lacks {needle:?}");
    }
    for (rule, meaning) in STRADDLER_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/STREAMING.md lacks {rule}");
    }
    let source = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/straddler.rs")).unwrap();
    assert!(source.contains("pub const STRADDLER_FACTOR: f64 = 4.0;"));
}
