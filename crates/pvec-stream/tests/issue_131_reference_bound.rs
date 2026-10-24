//! The reference fraction after P5.3 (issue pvec#131).
//!
//! P5.2 measured the fraction with a directory built over every entity, so long entities widened the
//! cells and the references overlapped heavily. P5.3 classifies first and builds the directory over the
//! local entities only. This file takes the same fixtures through that pipeline and holds the result to
//! the lower bound stated in `docs/STREAMING.md` section 2.4.

mod support;

use pvec_container::directory::{BlockEntry, BlockKind};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig};
use pvec_container::{Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    DuplicationPolicy, EntityBox, PayloadRef, PlacedEntity, STRADDLER_FACTOR, SpatialDirectory, build_directory,
    classify, payload_bytes, place, scan_payloads,
};
use std::collections::HashMap;
use support::fixtures::*;

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

fn push(w: &mut ContainerWriter, e: &Fx, cell: u32) {
    let b = e.bounds();
    let meta = EntityMeta { id: e.id, style: 0, cell, bounds: Bounds { min: b.min(), max: b.max() } };
    if e.points.len() == 2 {
        w.push_line(meta, e.points[0], e.points[1]).unwrap();
    } else {
        w.push_polyline(meta, &e.points).unwrap();
    }
}

/// What the post-P5.3 pipeline measured on one fixture.
struct Measured {
    straddlers: usize,
    entities: u64,
    references: u64,
    mean: f64,
    p99: u32,
    max: u32,
    reference_bytes: u64,
    payload_bytes: u64,
}

impl Measured {
    fn fraction(&self) -> f64 {
        self.reference_bytes as f64 / self.payload_bytes as f64
    }
}

fn measure(entities: &[Fx], ceiling: u32) -> Measured {
    let boxes: Vec<EntityBox> = entities.iter().map(|e| EntityBox { id: e.id, bounds: e.bounds() }).collect();
    let c = classify(&boxes, STRADDLER_FACTOR).expect("classified");
    let by_id: HashMap<EntityId, &Fx> = entities.iter().map(|e| (e.id, e)).collect();
    let built = build_directory(&c.local).expect("directory over the local entities");
    let mut w = ContainerWriter::new(config());
    for (e, cell) in c.local.iter().zip(&built.cells) {
        push(&mut w, by_id[&e.id], cell.get());
    }
    w.set_spatial_directory(
        built.nodes,
        Bounds { min: built.bounds.min(), max: built.bounds.max() },
        built.bytes.clone(),
    )
    .unwrap();
    let bytes = w.finish().unwrap();
    let r = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let dir = SpatialDirectory::open(&r).unwrap().expect("a directory");
    let slots = scan_payloads(&r).unwrap();
    let at: HashMap<EntityId, PayloadRef> = slots.iter().map(|s| (s.id, s.payload)).collect();
    let placed: Vec<PlacedEntity> =
        c.local.iter().map(|e| PlacedEntity { id: e.id, bounds: e.bounds, payload: at[&e.id] }).collect();

    // the straddlers' payloads are not in a cell (a hot block in no leaf is SPD-10), so they are written
    // as a file of their own and their decoded bytes are added to the payload the references are
    // measured against
    let mut sw = ContainerWriter::new(config());
    for e in &c.long {
        push(&mut sw, by_id[&e.id], 0);
    }
    let long_bytes: u64 = if c.long.is_empty() {
        0
    } else {
        let sr = ContainerReader::open(MemorySource::new(sw.finish().unwrap()), ReaderConfig::default()).unwrap();
        let entries: &[BlockEntry] = sr.entries();
        entries
            .iter()
            .filter(|e| e.block_kind() == Some(BlockKind::Hot))
            .map(|e| u64::from(e.uncompressed_length))
            .sum()
    };
    let total = payload_bytes(r.entries()) + long_bytes;
    let p = place(&dir, &placed, DuplicationPolicy::new(ceiling).unwrap(), total).unwrap();
    Measured {
        straddlers: c.long.len(),
        entities: p.report.entities,
        references: p.report.references,
        mean: p.report.mean,
        p99: p.report.p99,
        max: p.report.max,
        reference_bytes: p.report.reference_bytes,
        payload_bytes: total,
    }
}

fn fixtures() -> Vec<(&'static str, Vec<Fx>)> {
    vec![
        ("rooms", rooms(1, 8, 400, 2000.0)),
        ("stress-long", stress_long()),
        ("identical", identical_boxes()),
        ("nine-cells", nine_cells()),
    ]
}

/// The bound on the plan-like fixtures, after P5.3 (it was 2.0 for all of them before).
const PLAN_BOUND: f64 = 0.8;
/// The bound on the fixture whose entities are all the same box, which no directory can tell apart: every
/// entity reaches the ten cells of its tie, whatever classification does.
const DEGENERATE_BOUND: f64 = 2.0;

#[test]
fn after_p53_the_plan_like_fixtures_stay_under_the_lower_bound() {
    for (name, f) in fixtures() {
        let m = measure(&f, 16);
        eprintln!(
            "{name}: entities {} straddlers {} refs {} mean {:.3} p99 {} max {} ref bytes {} payload bytes {} fraction {:.4}",
            m.entities, m.straddlers, m.references, m.mean, m.p99, m.max, m.reference_bytes, m.payload_bytes, m.fraction()
        );
        let bound = if name == "identical" { DEGENERATE_BOUND } else { PLAN_BOUND };
        assert!(m.fraction() < bound, "{name}: {:.4} of the payload bytes, bound {bound}", m.fraction());
    }
}

#[test]
fn classifying_first_is_what_brings_stress_long_down() {
    let stress = stress_long();
    let m = measure(&stress, 16);
    assert_eq!(m.straddlers, 24, "the 24 long polylines are the straddlers");
    assert!(m.fraction() < 0.65, "stress-long {:.4}, it was 1.644 with the long polylines in the cells", m.fraction());
    assert!(m.max <= 16);
}

#[test]
fn section_2_4_states_the_numbers_this_pipeline_measures() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md")).unwrap();
    assert!(doc.contains("after P5.3"), "section 2.4 does not say its second table is taken after P5.3");
    for (name, f) in fixtures() {
        let m = measure(&f, 16);
        let row = format!(
            "| {} | {} | {} | {:.2} | {} | {} | {:.3} |",
            m.entities, m.straddlers, m.references, m.mean, m.p99, m.max, m.fraction()
        );
        assert!(doc.contains(&row), "docs/STREAMING.md section 2.4 lacks the {name} row {row:?}");
    }
    assert!(doc.contains("**0.8**") && doc.contains("degenerate"), "the lower bound is not stated");
}
