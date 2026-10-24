//! Single payload ownership with duplicated spatial references (issue P5.2, pvec#11).
//!
//! Each test is named for the acceptance criterion it holds the placement pass and the read side to. The
//! fixtures are generated here from a seeded splitmix64, like the P5.1 ones, and every artifact is
//! written through the real container writer and read back through the real reader.

use pvec_container::directory::{BlockEntry, BlockKind};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, CellId, CellRefs, DEFAULT_MAX_CELLS, DuplicationPolicy, EntityBox, OWNERSHIP_RULES, OwnershipError,
    PAYLOAD_REF_LEN, PayloadRef, PlacedEntity, REF_BUDGET, SpatialDirectory, ViewportRefs, build_directory,
    gather, payload_bytes, place, read_viewport, scan_payloads,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
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

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// One entity of a fixture: its control points (two make a line, more make a polyline).
#[derive(Clone)]
struct Fx {
    id: EntityId,
    points: Vec<[f64; 2]>,
}

impl Fx {
    fn bounds(&self) -> Aabb {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &self.points {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Aabb::new(lo, hi).unwrap()
    }
}

/// A grid of `side` x `side` rooms of `per_room` plan-like lines each, one in twenty a wall as long as its
/// room: the shape of P1.6's `million` at a size a debug build can write.
fn rooms(seed: u64, side: u32, per_room: u32, room: f64) -> Vec<Fx> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
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
                out.push(Fx { id, points: vec![[r3(x), r3(y)], [r3(x + w), r3(y + h)]] });
            }
        }
    }
    out
}

/// `count` polylines of five random control points each, every one reaching across `span` of a drawing
/// `extent` wide: the long entities of P1.6's `stress-long`.
fn long_polylines(rng: &mut Rng, first_id: u32, count: u32, extent: f64, span: f64) -> Vec<Fx> {
    (0..count)
        .map(|i| {
            let x0 = rng.unit() * (extent - span);
            let y0 = rng.unit() * (extent - span);
            let points =
                (0..5).map(|k| [r3(x0 + span * f64::from(k) / 4.0), r3(y0 + rng.unit() * span)]).collect();
            Fx { id: eid(first_id + i), points }
        })
        .collect()
}

fn stress_long() -> Vec<Fx> {
    let mut v = rooms(11, 6, 300, 2000.0);
    let mut rng = Rng(12);
    let n = v.len() as u32;
    v.extend(long_polylines(&mut rng, n, 24, 12_000.0, 9_000.0));
    v
}

fn identical_boxes() -> Vec<Fx> {
    (0..640).map(|i| Fx { id: eid(i), points: vec![[10.0, 10.0], [500.0, 400.0]] }).collect()
}

/// 575 small lines in a 3 x 3 grid of clusters and one long polyline across all of them: 576 entities, so
/// exactly nine cells of 64, and the polyline's bounds are the drawing's.
fn nine_cells() -> Vec<Fx> {
    let mut rng = Rng(5);
    let mut v = Vec::new();
    for i in 0..575u32 {
        let (cx, cy) = (f64::from(i % 3) * 3000.0 + 500.0, f64::from((i / 3) % 3) * 3000.0 + 500.0);
        let (x, y) = (cx + rng.unit() * 900.0, cy + rng.unit() * 900.0);
        v.push(Fx { id: eid(i), points: vec![[r3(x), r3(y)], [r3(x + 20.0), r3(y + 5.0)]] });
    }
    let points = vec![[100.0, 100.0], [2500.0, 4100.0], [4500.0, 2600.0], [6200.0, 6900.0], [8900.0, 8900.0]];
    v.push(Fx { id: eid(575), points });
    v
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/11".into(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

type Mem = Recording<MemorySource<Vec<u8>>>;

/// A written artifact, opened.
struct Artifact {
    reader: ContainerReader<Mem>,
    dir: SpatialDirectory,
    entities: Vec<Fx>,
    entries: Vec<BlockEntry>,
    bytes: Vec<u8>,
}

fn write(entities: Vec<Fx>) -> Artifact {
    let boxes: Vec<EntityBox> = entities.iter().map(|e| EntityBox { id: e.id, bounds: e.bounds() }).collect();
    let built = build_directory(&boxes).expect("directory");
    let mut w = ContainerWriter::new(config());
    for (e, cell) in entities.iter().zip(&built.cells) {
        let b = e.bounds();
        let meta = EntityMeta {
            id: e.id,
            style: 0,
            cell: cell.get(),
            bounds: Bounds { min: b.min(), max: b.max() },
        };
        if e.points.len() == 2 {
            w.push_line(meta, e.points[0], e.points[1]).unwrap();
        } else {
            w.push_polyline(meta, &e.points).unwrap();
        }
    }
    let root = Bounds { min: built.bounds.min(), max: built.bounds.max() };
    w.set_spatial_directory(built.nodes, root, built.bytes.clone()).unwrap();
    let bytes = w.finish().unwrap();
    let reader =
        ContainerReader::open(Recording::new(MemorySource::new(bytes.clone())), ReaderConfig::default())
            .unwrap();
    let dir = SpatialDirectory::open(&reader).unwrap().expect("a directory");
    let entries = reader.entries().to_vec();
    Artifact { reader, dir, entities, entries, bytes }
}

impl Artifact {
    /// A reader with a cold cache and an empty request log, over the same bytes.
    fn cold_reader(&self) -> ContainerReader<Mem> {
        ContainerReader::open(Recording::new(MemorySource::new(self.bytes.clone())), ReaderConfig::default())
            .unwrap()
    }

    fn placed(&self) -> Vec<PlacedEntity> {
        let slots = scan_payloads(&self.reader).unwrap();
        let at: HashMap<EntityId, PayloadRef> = slots.iter().map(|s| (s.id, s.payload)).collect();
        self.entities
            .iter()
            .map(|e| PlacedEntity { id: e.id, bounds: e.bounds(), payload: at[&e.id] })
            .collect()
    }

    fn place(&self, policy: DuplicationPolicy) -> pvec_stream::Placement {
        place(&self.dir, &self.placed(), policy, payload_bytes(&self.entries)).unwrap()
    }
}

// ---------------------------------------------------------------- the criteria

#[test]
fn each_entitys_geometry_bytes_appear_exactly_once() {
    let a = write(stress_long());
    let placement = a.place(DuplicationPolicy::default());
    // references are duplicated (that is the point), the payloads are not
    assert!(
        placement.refs.total_refs() > a.entities.len(),
        "the fixture must duplicate references for this test to mean anything"
    );
    let slots = scan_payloads(&a.reader).unwrap();
    let hot_rows: u64 =
        a.entries.iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).map(|e| u64::from(e.count)).sum();
    assert_eq!(slots.len() as u64, hot_rows, "the scan sees every row of every hot block");
    assert_eq!(slots.len(), a.entities.len(), "the number of payload slots is the entity count");
    let ids: BTreeSet<_> = slots.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), a.entities.len(), "an entity is in two slots");
    let refs: BTreeSet<_> = slots.iter().map(|s| s.payload).collect();
    assert_eq!(refs.len(), slots.len(), "two slots share a reference");

    // byte level: the first four control points of every long polyline are a 32 byte run of x values;
    // across all hot blocks that run occurs once
    let mut all = Vec::new();
    for e in a.entries.iter().enumerate().filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot)) {
        all.extend_from_slice(a.reader.read_block(BlockId(e.0 as u32)).unwrap().as_bytes());
    }
    let mut checked = 0;
    for e in a.entities.iter().filter(|e| e.points.len() > 2) {
        let run: Vec<u8> = e.points[..4].iter().flat_map(|p| p[0].to_le_bytes()).collect();
        let found = all.windows(run.len()).filter(|w| *w == run.as_slice()).count();
        assert_eq!(found, 1, "the control-point run of entity {:?} is in the blocks {found} times", e.id);
        checked += 1;
    }
    assert!(checked >= 24);
}

#[test]
fn a_spatial_reference_is_at_most_8_bytes() {
    assert_eq!(REF_BUDGET, 8);
    assert!(
        std::mem::size_of::<PayloadRef>() <= 8,
        "PayloadRef is {} bytes",
        std::mem::size_of::<PayloadRef>()
    );
    assert_eq!(PAYLOAD_REF_LEN, 8);
    let r = PayloadRef { block_id: BlockId(0xDEAD_BEEF), slot: 0x0102_0304 };
    assert_eq!(PayloadRef::decode(r.encode()), r);
    assert_eq!(r.encode(), [0xEF, 0xBE, 0xAD, 0xDE, 4, 3, 2, 1], "little endian, block then slot");
    // the build fails if the type grows: the assertion is a const in the crate
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/ownership.rs")).unwrap();
    assert!(src.contains("assert!(core::mem::size_of::<PayloadRef>() <= REF_BUDGET)"));
}

/// The fraction of payload bytes that reference bytes may reach on the named fixtures when the directory is
/// built over every entity (the placement pass alone, before P5.3), measured and then stated in
/// `docs/STREAMING.md` section 2.4. The bound for the pipeline that classifies first is lower and is held
/// by `issue_131_reference_bound.rs`.
const MAX_FRACTION: f64 = 2.0;

#[test]
fn reference_bytes_stay_under_the_stated_fraction_of_payload_bytes_on_every_named_fixture() {
    let fixtures: Vec<(&str, Vec<Fx>)> = vec![
        ("rooms", rooms(1, 8, 400, 2000.0)),
        ("stress-long", stress_long()),
        ("identical", identical_boxes()),
        ("nine-cells", nine_cells()),
    ];
    for (name, entities) in fixtures {
        let a = write(entities);
        let p = a.place(DuplicationPolicy::default());
        let r = &p.report;
        eprintln!(
            "{name}: entities {} straddlers {} refs {} mean {:.3} p99 {} max {} ref bytes {} payload bytes {} fraction {:.4}",
            r.entities,
            r.straddlers,
            r.references,
            r.mean,
            r.p99,
            r.max,
            r.reference_bytes,
            r.payload_bytes,
            r.fraction()
        );
        assert!(r.fraction() < MAX_FRACTION, "{name}: {:.4} of the payload bytes", r.fraction());
        assert_eq!(r.reference_bytes, r.references * 8);
        // how the ceiling trades references for straddlers, for the table in section 2.4
        for c in [1u32, 2, 4, 8] {
            let q = a.place(DuplicationPolicy::new(c).unwrap()).report;
            eprintln!(
                "  {name} @ {c}: straddlers {} refs {} mean {:.3} fraction {:.4}",
                q.straddlers,
                q.references,
                q.mean,
                q.fraction()
            );
        }
    }
}

#[test]
fn a_viewport_over_nine_cells_that_all_reference_one_long_entity_decodes_it_once() {
    let a = write(nine_cells());
    assert_eq!(a.dir.cells(), 9);
    let p = a.place(DuplicationPolicy::default());
    assert!(p.straddlers.is_empty());
    let long = a.placed().into_iter().find(|e| e.id == eid(575)).unwrap();
    let viewport = a.dir.bounds();
    let cells: Vec<CellId> = a.dir.query(&viewport).map(|c| c.id).collect();
    assert_eq!(cells.len(), 9);
    for c in &cells {
        assert!(p.refs.refs_of(*c).contains(&long.payload), "cell {c:?} lacks the reference");
    }

    let g = gather(&a.dir, &p.refs, &viewport);
    let all: usize = cells.iter().map(|c| p.refs.refs_of(*c).len()).sum();
    assert_eq!(g.raw, all, "the raw count is what the nine cells list");
    assert_eq!(g.refs.iter().filter(|r| **r == long.payload).count(), 1);

    // placing scanned every block through `a.reader`, so the read goes through a cold one
    let cold = a.cold_reader();
    let before = cold.source().requests().len();
    let mut decoded: HashMap<EntityId, u32> = HashMap::new();
    let n = read_viewport(&cold, &g, |id, _view, _row| *decoded.entry(id).or_insert(0) += 1).unwrap();
    assert_eq!(n, 576);
    assert_eq!(decoded[&eid(575)], 1, "the long entity was decoded {} times", decoded[&eid(575)]);
    assert!(decoded.values().all(|v| *v == 1), "an entity was decoded twice");
    // the fixture really does list the long entity in every cell, so there was something to remove
    assert!(g.raw > g.refs.len(), "the cells overlap on the long entity, so deduplication removes some");
    // and the counting source agrees: its block was fetched once
    let entry = &a.entries[long.payload.block_id.0 as usize];
    let reads = cold.source().requests()[before..]
        .iter()
        .filter(|r| {
            r.offset < entry.offset + u64::from(entry.stored_length)
                && entry.offset < r.offset + u64::from(r.len)
        })
        .count();
    assert_eq!(reads, 1, "the long entity's block was fetched {reads} times");
}

#[test]
fn an_entity_over_the_ceiling_goes_to_the_straddler_path_instead_of_being_duplicated() {
    let a = write(stress_long());
    let placed = a.placed();
    for ceiling in [1u32, 2, 3, DEFAULT_MAX_CELLS, 1000] {
        let p = a.place(DuplicationPolicy::new(ceiling).unwrap());
        let straddlers: HashMap<_, _> = p.straddlers.iter().map(|s| (s.id, s.payload)).collect();
        let mut expected_refs = 0usize;
        let mut over = 0usize;
        for e in &placed {
            // the cells the entity touches, found independently of the pass
            let cells: Vec<CellId> = a.dir.query(&e.bounds).map(|c| c.id).collect();
            let listed: Vec<CellId> =
                cells.iter().copied().filter(|c| p.refs.refs_of(*c).contains(&e.payload)).collect();
            if cells.len() as u32 > ceiling {
                over += 1;
                assert_eq!(
                    straddlers.get(&e.id),
                    Some(&e.payload),
                    "{:?} over {ceiling} is not a straddler",
                    e.id
                );
                assert!(
                    listed.is_empty(),
                    "{:?} over {ceiling} got {} references anyway",
                    e.id,
                    listed.len()
                );
            } else {
                assert!(!straddlers.contains_key(&e.id));
                assert_eq!(listed, cells, "{:?} is not referenced from exactly the cells it touches", e.id);
                expected_refs += cells.len();
            }
        }
        assert_eq!(p.straddlers.len(), over);
        assert_eq!(p.refs.total_refs(), expected_refs);
        assert_eq!(p.report.straddlers as usize, over);
        assert!(p.report.max <= ceiling, "max {} over the ceiling {ceiling}", p.report.max);
    }
    // at the lowest ceiling the long polylines are certainly over it; at the highest nothing is
    assert!(!a.place(DuplicationPolicy::new(1).unwrap()).straddlers.is_empty());
    assert!(a.place(DuplicationPolicy::new(1_000_000).unwrap()).straddlers.is_empty());
    assert_eq!(DuplicationPolicy::new(0).unwrap_err().rule(), "OWN-07");
}

#[test]
fn the_report_counts_what_the_arrays_hold() {
    let a = write(stress_long());
    let p = a.place(DuplicationPolicy::default());
    let mut per: HashMap<PayloadRef, u32> = HashMap::new();
    for e in a.placed() {
        let _ = e;
    }
    for c in a.dir.query(&a.dir.bounds()) {
        for r in p.refs.refs_of(c.id) {
            *per.entry(*r).or_insert(0) += 1;
        }
    }
    let mut counts: Vec<u32> = per.values().copied().collect();
    counts.sort_unstable();
    let n = counts.len();
    assert_eq!(p.report.entities as usize, n);
    assert_eq!(n + p.straddlers.len(), a.entities.len());
    assert_eq!(p.report.max, *counts.last().unwrap());
    assert_eq!(p.report.p99, counts[(n * 99).div_ceil(100) - 1], "nearest rank");
    let total: u64 = counts.iter().map(|c| u64::from(*c)).sum();
    assert_eq!(p.report.references, total);
    assert_eq!(p.report.reference_bytes, total * 8);
    assert_eq!(p.report.mean, total as f64 / n as f64);
    let hot: u64 = a
        .entries
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Hot))
        .map(|e| u64::from(e.uncompressed_length))
        .sum();
    assert_eq!(p.report.payload_bytes, hot);
    assert_eq!(p.report.fraction(), (total * 8) as f64 / hot as f64);
}

#[test]
fn a_viewport_yields_every_visible_entity_once_whatever_cells_referenced_it() {
    let a = write(stress_long());
    let p = a.place(DuplicationPolicy::default());
    let straddlers: HashSet<EntityId> = p.straddlers.iter().map(|s| s.id).collect();
    let mut rng = Rng(77);
    let extent = 12_000.0;
    for k in 0..40 {
        let w = extent * [1.0, 0.3, 0.1, 0.02][k % 4];
        let (x, y) = (rng.unit() * (extent - w), rng.unit() * (extent - w));
        let q = bx(x, y, x + w, y + w);
        let g = gather(&a.dir, &p.refs, &q);
        let mut seen: HashSet<EntityId> = HashSet::new();
        let n = read_viewport(&a.reader, &g, |id, view, row| {
            assert_eq!(view.entity_ids()[row], id.index());
            assert!(seen.insert(id), "{id:?} was yielded twice for {q:?}");
        })
        .unwrap();
        assert_eq!(n, seen.len());
        for e in &a.entities {
            if e.bounds().intersects(&q) && !straddlers.contains(&e.id) {
                assert!(seen.contains(&e.id), "{:?} intersects {q:?} and was not yielded", e.id);
            }
        }
    }
}

// ---------------------------------------------------------------- the reference arrays and what they refuse

fn sample_refs() -> (CellRefs, Vec<u8>) {
    let a = write(nine_cells());
    let p = a.place(DuplicationPolicy::default());
    let bytes = p.refs.encode();
    (p.refs, bytes)
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn rule(r: Result<impl std::fmt::Debug, OwnershipError>) -> &'static str {
    r.expect_err("must be refused").rule()
}

#[test]
fn the_reference_arrays_round_trip_and_cost_header_plus_8_bytes_a_cell_plus_8_a_reference() {
    let (refs, bytes) = sample_refs();
    assert_eq!(bytes.len(), 16 + 8 * refs.cells() + 8 * refs.total_refs());
    assert_eq!(CellRefs::parse(&bytes).unwrap(), refs);
    assert_eq!(CellRefs::parse(&bytes).unwrap().encode(), bytes, "canonical");
    assert_eq!(CellRefs::default().encode().len(), 16);
    assert_eq!(CellRefs::parse(&CellRefs::default().encode()).unwrap(), CellRefs::default());
}

#[test]
fn hostile_reference_arrays_are_refused_by_rule() {
    let (refs, bytes) = sample_refs();
    let (n, r) = (refs.cells(), refs.total_refs());
    assert!(n >= 3 && r > n);
    // OWN-01: too short, too long, a count that does not match the length (and costs nothing to claim)
    assert_eq!(rule(CellRefs::parse(&bytes[..10])), "OWN-01");
    assert_eq!(rule(CellRefs::parse(&bytes[..bytes.len() - 1])), "OWN-01");
    let mut long = bytes.clone();
    long.push(0);
    assert_eq!(rule(CellRefs::parse(&long)), "OWN-01");
    let mut huge = bytes.clone();
    put32(&mut huge, 12, u32::MAX);
    assert_eq!(rule(CellRefs::parse(&huge)), "OWN-01");
    let mut huge = bytes.clone();
    put32(&mut huge, 8, u32::MAX);
    assert_eq!(rule(CellRefs::parse(&huge)), "OWN-01");
    // OWN-02: magic, version, flags
    for at in [0usize, 4, 6] {
        let mut b = bytes.clone();
        b[at] ^= 0x40;
        assert_eq!(rule(CellRefs::parse(&b)), "OWN-02", "header byte {at}");
    }
    // OWN-03: cell 0, cells that do not ascend, an empty cell, ends that do not reach the count
    let mut b = bytes.clone();
    put32(&mut b, 16, 0);
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-03");
    let mut b = bytes.clone();
    let first = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    put32(&mut b, 20, first);
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-03");
    let mut b = bytes.clone();
    let end0 = 16 + 4 * n;
    put32(&mut b, end0 + 4, u32::from_le_bytes(bytes[end0..end0 + 4].try_into().unwrap()));
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-03");
    let mut b = bytes.clone();
    put32(&mut b, end0 + 4 * (n - 1), r as u32 - 1);
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-03");
    // OWN-04: references inside a cell that do not ascend
    let mut b = bytes.clone();
    let refs0 = 16 + 8 * n;
    let len0 = u32::from_le_bytes(bytes[end0..end0 + 4].try_into().unwrap()) as usize;
    assert!(len0 >= 2);
    let (x, y) = (b[refs0..refs0 + 8].to_vec(), b[refs0 + 8..refs0 + 16].to_vec());
    b[refs0..refs0 + 8].copy_from_slice(&y);
    b[refs0 + 8..refs0 + 16].copy_from_slice(&x);
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-04");
    b[refs0 + 8..refs0 + 16].copy_from_slice(&y);
    assert_eq!(rule(CellRefs::parse(&b)), "OWN-04", "a repeated reference");
}

#[test]
fn placement_refuses_what_it_cannot_assign_to_exactly_one_payload() {
    let a = write(nine_cells());
    let placed = a.placed();
    let go = |v: &[PlacedEntity]| place(&a.dir, v, DuplicationPolicy::default(), 1);
    // OWN-05: one entity twice, one payload for two entities
    let mut dup_id = placed.clone();
    dup_id[1].id = dup_id[0].id;
    assert_eq!(rule(go(&dup_id)), "OWN-05");
    let mut dup_payload = placed.clone();
    dup_payload[1].payload = dup_payload[0].payload;
    assert_eq!(rule(go(&dup_payload)), "OWN-05");
    // OWN-06: bounds outside every cell
    let mut outside = placed.clone();
    outside[0].bounds = bx(-9000.0, -9000.0, -8000.0, -8000.0);
    assert_eq!(rule(go(&outside)), "OWN-06");
    assert!(go(&placed).is_ok());
}

#[test]
fn reading_refuses_a_reference_that_names_no_payload() {
    let a = write(nine_cells());
    let hot = a.entries.iter().position(|e| e.block_kind() == Some(BlockKind::Hot)).unwrap() as u32;
    let spatial =
        a.entries.iter().position(|e| e.block_kind() == Some(BlockKind::SpatialDirectory)).unwrap() as u32;
    let count = a.entries[hot as usize].count;
    let go = |r: PayloadRef| read_viewport(&a.reader, &ViewportRefs { raw: 1, refs: vec![r] }, |_, _, _| {});
    assert_eq!(rule(go(PayloadRef { block_id: BlockId(hot), slot: count })), "OWN-08", "a row past the end");
    assert_eq!(rule(go(PayloadRef { block_id: BlockId(spatial), slot: 0 })), "OWN-08", "not a hot block");
    assert_eq!(rule(go(PayloadRef { block_id: BlockId(u32::MAX), slot: 0 })), "OWN-08", "no such block");
    assert_eq!(go(PayloadRef { block_id: BlockId(hot), slot: 0 }).unwrap(), 1);
}

#[test]
fn the_rules_are_documented_and_the_policy_is_in_the_streaming_chapter() {
    let doc = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md")).unwrap();
    assert!(doc.contains("## 2. Payload ownership and duplicated references"));
    for needle in [
        "PayloadRef",
        "DuplicationPolicy",
        "DEFAULT_MAX_CELLS",
        "straddler",
        "read-side deduplication",
        "duplication report",
        "Scope note",
    ] {
        assert!(doc.contains(needle), "docs/STREAMING.md lacks {needle:?}");
    }
    let ids: BTreeMap<_, _> = OWNERSHIP_RULES.iter().copied().collect();
    assert_eq!(ids.len(), 11);
    for (rule, meaning) in OWNERSHIP_RULES {
        assert!(doc.contains(&format!("| {rule} | {meaning} |")), "docs/STREAMING.md lacks {rule}");
    }
}
