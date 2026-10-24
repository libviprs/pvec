//! The straddler lists as a container block, and their payloads (issue P5.3b, pvec#136).
//!
//! The fixture is `drawing-100k` of issue 12: 100,000 local entities and 1,000 drawing-spanning lines.
//! Every test opens a real file written through the container writer and reads it back through a
//! recording source, so what is counted is range reads and not a model of them.

use pvec_container::directory::{BlockKind, FLAG_MUST_UNDERSTAND, STRADDLER_CELL_FIRST};
use pvec_container::source::{MemorySource, Recording};
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Limits, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, DuplicationPolicy, EntityBox, LongEntity, PayloadRef, PlacedEntity, STRADDLER_ENTRY_LEN,
    STRADDLER_FACTOR, STRADDLER_HEADER_LEN, STRADDLER_RULES, SpatialDirectory, StraddlerIndex,
    StraddlerLists, build_directory, build_straddlers, classify, payload_bytes, payload_cells, place,
    query_viewport, scan_payloads, unique_entities,
};
use std::collections::BTreeSet;
use std::fs;

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

fn rooms(seed: u64, side: u32, per_room: u32, room: f64) -> Vec<EntityBox> {
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
                out.push(EntityBox { id, bounds: bx(three(x), three(y), three(x + w), three(y + h)) });
            }
        }
    }
    out
}

fn spanning(entities: &mut Vec<EntityBox>, seed: u64, n: u32, world: f64) {
    let mut rng = Rng(seed);
    for k in 0..n {
        let t = three(rng.unit() * world);
        let id = eid(entities.len() as u32);
        let bounds = if k % 2 == 0 { bx(0.0, t, world, t) } else { bx(t, 0.0, t, world) };
        entities.push(EntityBox { id, bounds });
    }
}

fn drawing_100k() -> Vec<EntityBox> {
    let mut all = rooms(12, 20, 250, 500.0);
    spanning(&mut all, 77, 1000, 10_000.0);
    all
}

fn config() -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-stream-tests".into(),
            producer_version: "t/136".into(),
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

/// A written file and what went into it.
struct Written {
    bytes: Vec<u8>,
    all: Vec<EntityBox>,
    long: Vec<EntityBox>,
    local: Vec<EntityBox>,
    block: Vec<u8>,
}

/// The writer order of the issue: classify, build the directory over the local entities, write the
/// local entities in their cells and the straddlers' payloads in the cells `payload_cells` names, learn
/// the block ids from that layout, build the lists over them and write the file again with the lists.
fn write(all: Vec<EntityBox>) -> Written {
    let c = classify(&all, STRADDLER_FACTOR).expect("classified");
    let built = build_directory(&c.local).expect("directory");
    let placeholder = |e: &EntityBox| LongEntity {
        id: e.id,
        bounds: e.bounds,
        payload: PayloadRef { block_id: BlockId(0), slot: 0 },
    };
    let cells = payload_cells(&c.scale, &c.long.iter().map(placeholder).collect::<Vec<_>>()).unwrap();
    let lay_out = |lists: Option<(u32, Vec<u8>)>| {
        let mut w = ContainerWriter::new(config());
        for (e, cell) in c.local.iter().zip(&built.cells) {
            let meta = EntityMeta { id: e.id, style: 0, cell: cell.get(), bounds: to_bounds(e.bounds) };
            w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
        }
        for e in &c.long {
            let cell = cells.iter().find(|(id, _)| *id == e.id).expect("a payload cell").1;
            assert!(cell >= STRADDLER_CELL_FIRST);
            let meta = EntityMeta { id: e.id, style: 0, cell, bounds: to_bounds(e.bounds) };
            w.push_line(meta, e.bounds.min(), e.bounds.max()).unwrap();
        }
        w.set_spatial_directory(built.nodes, to_bounds(built.bounds), built.bytes.clone()).unwrap();
        if let Some((entries, bytes)) = lists {
            let world = c.long.iter().skip(1).fold(c.long[0].bounds, |u, e| {
                bx(
                    u.min()[0].min(e.bounds.min()[0]),
                    u.min()[1].min(e.bounds.min()[1]),
                    u.max()[0].max(e.bounds.max()[0]),
                    u.max()[1].max(e.bounds.max()[1]),
                )
            });
            w.set_straddler_lists(entries, to_bounds(world), bytes).unwrap();
        }
        w.finish().unwrap()
    };
    let first = reader(lay_out(None));
    let slots = scan_payloads(&first).unwrap();
    let long: Vec<LongEntity> = c
        .long
        .iter()
        .map(|e| LongEntity {
            id: e.id,
            bounds: e.bounds,
            payload: slots.iter().find(|s| s.id == e.id).expect("a slot").payload,
        })
        .collect();
    let built_lists = build_straddlers(&c.scale, &long).unwrap();
    let bytes = lay_out(Some((built_lists.entries, built_lists.bytes.clone())));
    Written { bytes, all, long: c.long, local: c.local, block: built_lists.bytes }
}

fn viewport(cx: f64, cy: f64, size: f64) -> Aabb {
    bx(cx - size / 2.0, cy - size / 2.0, cx + size / 2.0, cy + size / 2.0)
}

fn straddler_entry(r: &ContainerReader<Mem>) -> (usize, pvec_container::directory::BlockEntry) {
    r.entries()
        .iter()
        .enumerate()
        .find(|(_, e)| e.block_kind() == Some(BlockKind::Straddlers))
        .map(|(i, e)| (i, *e))
        .expect("a straddler block")
}

fn written() -> &'static Written {
    static W: std::sync::OnceLock<Written> = std::sync::OnceLock::new();
    W.get_or_init(|| write(drawing_100k()))
}

#[test]
fn a_file_with_straddlers_validates_and_the_block_is_optional_and_stored_as_it_is() {
    let w = written();
    assert_eq!(w.long.len(), 1000);
    let r = reader(w.bytes.clone());
    r.validate().expect("a file with straddlers validates");
    let (i, e) = straddler_entry(&r);
    assert_eq!(e.kind, 6);
    assert_eq!(e.flags & FLAG_MUST_UNDERSTAND, 0, "optional: an older reader skips it");
    assert_eq!(e.codec, 0, "stored as it is, so its index is one range");
    assert_eq!(e.cell, 0);
    assert_eq!(e.count as usize, w.long.len());
    assert_eq!(e.uncompressed_length as usize, w.block.len());
    assert_eq!(r.read_block(BlockId(i as u32)).unwrap().as_bytes(), &w.block[..]);
    assert_eq!(BlockKind::from_id(6), Some(BlockKind::Straddlers));
    // the whole block parses and is the block that went in
    StraddlerLists::parse(&w.block, &Limits::DEFAULT).unwrap();
}

#[test]
fn the_directory_does_not_traverse_the_straddlers_payload_blocks() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).expect("the directory accepts the payload cells").unwrap();
    assert_eq!(dir.entities(), w.local.len() as u64, "the cells hold the local entities only");
    let payload_blocks: Vec<_> = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Hot) && e.cell >= STRADDLER_CELL_FIRST)
        .collect();
    assert_eq!(payload_blocks.iter().map(|e| e.count as usize).sum::<usize>(), w.long.len());
    assert!(payload_blocks.len() >= 16, "one block per page of the list, not one block for all of them");
    assert!(payload_blocks.iter().all(|e| e.count <= 64));
}

#[test]
fn a_one_screen_viewport_reads_the_index_and_only_the_pages_and_payloads_it_touches() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let (_, entry) = straddler_entry(&r);
    let (start, end) = (entry.offset, entry.offset + u64::from(entry.stored_length));

    let before = r.source().requests().len();
    let index = StraddlerIndex::open(&r).unwrap().expect("an index");
    let screen = viewport(5000.0, 5000.0, 500.0);
    let hits = index.query(&r, &screen).unwrap();
    let after = r.source().requests();
    let in_block: Vec<_> = after[before..].iter().filter(|q| q.offset >= start && q.offset < end).collect();
    let read: u64 = in_block.iter().map(|q| u64::from(q.len)).sum();

    assert_eq!(read, index.read_bytes(&screen), "the reads are the index and the touched pages, counted");
    assert!(read < u64::from(entry.stored_length) / 2, "{read} of {} bytes", entry.stored_length);
    assert!(read >= index.index_bytes());
    assert_eq!(index.index_bytes() as usize, STRADDLER_HEADER_LEN + 40 * index.pages());
    assert!(in_block.iter().all(|q| u64::from(q.len) as usize <= STRADDLER_HEADER_LEN + 40 * index.pages()
        || (u64::from(q.len) as usize) % STRADDLER_ENTRY_LEN == 0));

    // the same straddlers as a brute-force scan of the bounds
    let want: BTreeSet<u32> =
        w.long.iter().filter(|e| e.bounds.intersects(&screen)).map(|e| e.id.index()).collect();
    assert!(!want.is_empty());
    assert_eq!(hits.iter().map(|h| h.id.index()).collect::<BTreeSet<_>>(), want);

    // each payload resolves to its entity, in a block that only the pages touched need
    let blocks: BTreeSet<u32> = hits.iter().map(|h| h.payload.block_id.0).collect();
    for h in &hits {
        let block = r.read_block(h.payload.block_id).unwrap();
        let view = block.hot().unwrap();
        assert_eq!(view.entity_ids()[h.payload.slot as usize], h.id.index());
    }
    let payload_total = r
        .entries()
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Hot) && e.cell >= STRADDLER_CELL_FIRST)
        .count();
    assert!(blocks.len() < payload_total, "{} of {payload_total} payload blocks", blocks.len());

    // and with the cells the viewport is the whole answer
    let lists = StraddlerLists::parse(&w.block, &Limits::DEFAULT).unwrap();
    let cells = query_viewport(&dir, Some(&lists), &screen);
    assert_eq!(cells.long.iter().map(|h| h.id).collect::<BTreeSet<_>>(), hits.iter().map(|h| h.id).collect());
    let mut local_ids = Vec::new();
    for c in &cells.cells {
        for b in c.blocks.first.0..c.blocks.first.0 + c.blocks.count {
            local_ids.extend(r.read_block(BlockId(b)).unwrap().hot().unwrap().entity_ids().iter().copied());
        }
    }
    let seen: BTreeSet<EntityId> =
        unique_entities(local_ids.iter().map(|i| eid(*i)), &cells).into_iter().collect();
    let brute: BTreeSet<EntityId> =
        w.all.iter().filter(|e| e.bounds.intersects(&screen)).map(|e| e.id).collect();
    assert!(brute.is_subset(&seen), "{} entities are missing", brute.difference(&seen).count());
}

#[test]
fn placement_over_the_local_directory_hands_nothing_over_once_the_lists_hold_the_long_entities() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let slots = scan_payloads(&r).unwrap();
    let placed: Vec<PlacedEntity> = w
        .local
        .iter()
        .map(|e| PlacedEntity {
            id: e.id,
            bounds: e.bounds,
            payload: slots.iter().find(|s| s.id == e.id).unwrap().payload,
        })
        .collect();
    let p = place(&dir, &placed, DuplicationPolicy::default(), payload_bytes(r.entries())).unwrap();
    assert!(p.straddlers.is_empty());
}

#[test]
fn a_block_that_lies_is_refused_by_rule_and_never_panics() {
    let w = written();
    let r = reader(w.bytes.clone());
    let (_, entry) = straddler_entry(&r);
    let mut seen = BTreeSet::new();
    // a payload that names a block that is no payload block, flipped in a copy of the file
    for at in [STRADDLER_HEADER_LEN + 40 * 16 + 36, STRADDLER_HEADER_LEN + 40 * 16 + 40, 4, 12] {
        let mut bytes = w.bytes.clone();
        let off = entry.offset as usize + at;
        bytes[off] ^= 0x7F;
        let Ok(r) = ContainerReader::open(Recording::new(MemorySource::new(bytes)), ReaderConfig::default())
        else {
            continue;
        };
        if let Ok(Some(index)) = StraddlerIndex::open(&r) {
            let q = bx(-1_000_000_000.0, -1_000_000_000.0, 1_000_000_000.0, 1_000_000_000.0);
            if let Err(e) = index.query(&r, &q) {
                seen.insert(e.rule());
            }
        }
    }
    let known: BTreeSet<&str> = STRADDLER_RULES.iter().map(|(r, _)| *r).collect();
    assert!(seen.iter().all(|r| known.contains(r)));
    assert!(STRADDLER_RULES.iter().any(|(r, _)| *r == "STR-10"));
    assert!(STRADDLER_RULES.iter().any(|(r, _)| *r == "STR-11"));
}

#[test]
fn the_documents_describe_the_block_kind_the_payload_cells_and_the_new_rules() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/");
    let container = fs::read_to_string(format!("{root}CONTAINER.md")).unwrap();
    let streaming = fs::read_to_string(format!("{root}STREAMING.md")).unwrap();
    assert!(container.contains("| 6 | straddlers |"), "CONTAINER.md has no row for kind 6");
    assert!(container.contains("STRADDLER_CELL_FIRST"), "CONTAINER.md does not name the payload cells");
    for (rule, meaning) in STRADDLER_RULES {
        assert!(streaming.contains(&format!("| {rule} | {meaning} |")), "STREAMING.md lacks {rule}");
    }
    assert!(!streaming.contains("It is not a block kind of the container yet"));
}

/// The stated budget of `docs/STREAMING.md` 3.6: a one-screen viewport reads under 64 KiB and under 2 per
/// cent of the artifact.
const VIEWPORT_BYTE_BUDGET: u64 = 64 * 1024;
const VIEWPORT_ARTIFACT_FRACTION: f64 = 0.02;

#[test]
fn the_numbers_of_section_3_6_are_measured_from_the_written_file_and_stay_in_budget() {
    let w = written();
    let r = reader(w.bytes.clone());
    let dir = SpatialDirectory::open(&r).unwrap().unwrap();
    let index = StraddlerIndex::open(&r).unwrap().unwrap();
    let screen = viewport(5000.0, 5000.0, 500.0);
    let hits = index.query(&r, &screen).unwrap();
    let cells = query_viewport(&dir, None, &screen).cells;
    let stored = |b: u32| u64::from(r.entries()[b as usize].stored_length);
    let cell_bytes: u64 = cells
        .iter()
        .flat_map(|c| (c.blocks.first.0..c.blocks.first.0 + c.blocks.count).map(|b| stored(b)))
        .sum();
    let payload_blocks: BTreeSet<u32> = hits.iter().map(|h| h.payload.block_id.0).collect();
    let payload_bytes: u64 = payload_blocks.iter().map(|b| stored(*b)).sum();
    let list_bytes = index.read_bytes(&screen);
    let total = cell_bytes + list_bytes + payload_bytes;
    let (_, entry) = straddler_entry(&r);
    eprintln!(
        "3.6 written file: {} B; block {} B (index {} B, {} pages); screen: {} cells {} B, list {} B, {} straddlers in {} payload blocks {} B; total {} B = {:.2} %",
        w.bytes.len(),
        entry.stored_length,
        index.index_bytes(),
        index.pages(),
        cells.len(),
        cell_bytes,
        list_bytes,
        hits.len(),
        payload_blocks.len(),
        payload_bytes,
        total,
        100.0 * total as f64 / w.bytes.len() as f64
    );
    assert!(total < VIEWPORT_BYTE_BUDGET, "{total} bytes");
    assert!((total as f64) < VIEWPORT_ARTIFACT_FRACTION * w.bytes.len() as f64);
}

fn everything() -> Aabb {
    bx(-1_000_000_000.0, -1_000_000_000.0, 1_000_000_000.0, 1_000_000_000.0)
}

#[test]
fn str_11_a_payload_that_is_not_in_the_payload_cell_of_its_page_is_refused() {
    let w = written();
    let r = reader(w.bytes.clone());
    let (_, entry) = straddler_entry(&r);
    let first_entry = entry.offset as usize + STRADDLER_HEADER_LEN + 40 * 16;
    // point the first entry's payload at block 0, a hot block of an ordinary cell
    let mut bytes = w.bytes.clone();
    bytes[first_entry + 36..first_entry + 40].copy_from_slice(&0u32.to_le_bytes());
    let r = reader(bytes);
    let index = StraddlerIndex::open(&r).unwrap().unwrap();
    assert_eq!(index.query(&r, &everything()).unwrap_err().rule(), "STR-11");
    // and one past the end of the block
    let mut bytes = w.bytes.clone();
    bytes[first_entry + 40..first_entry + 44].copy_from_slice(&64u32.to_le_bytes());
    let r = reader(bytes);
    let index = StraddlerIndex::open(&r).unwrap().unwrap();
    assert_eq!(index.query(&r, &everything()).unwrap_err().rule(), "STR-11");
}

#[test]
fn str_10_a_block_entry_that_disagrees_with_the_lists_is_refused() {
    let w = written();
    // a count that is not the entries listed
    let mut file = ContainerWriter::new(config());
    let meta = EntityMeta { id: eid(0), style: 0, cell: 1, bounds: to_bounds(bx(0.0, 0.0, 1.0, 1.0)) };
    file.push_line(meta, [0.0, 0.0], [1.0, 1.0]).unwrap();
    file.set_straddler_lists(w.long.len() as u32 + 1, to_bounds(bx(0.0, 0.0, 1.0, 1.0)), w.block.clone())
        .unwrap();
    let r = reader(file.finish().unwrap());
    assert_eq!(StraddlerIndex::open(&r).unwrap_err().rule(), "STR-10");
}
