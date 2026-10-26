//! Per-export fidelity reports (issue P6.8, pvec#25).
//!
//! Red first: these tests describe `pvec_interchange::report` before it exists. Every export has to
//! produce a report, the counts reconcile with the artifact, a tessellated target claims no exactness,
//! the error is the measured one, an `Unsupported` row says why, and the JSON is byte-stable and
//! versioned.

mod common;
mod report_json;

use common::*;
use pvec_capabilities::ids;
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Line, Placement2, Point2, Transform2, Vector2};
use pvec_interchange::mvt::{self, MvtEntity, MvtOptions, MvtTile, Projection, TileId};
use pvec_interchange::pmtiles::{self, Provenance};
use pvec_interchange::report::{Report, ReportError, SCHEMA_VERSION};
use pvec_interchange::step::{StepEntity, StepOptions};
use pvec_interchange::{
    gpkg, step, ErrorBound, Fidelity, FidelityKind, FidelityRecord, Reason, Representation, TargetConstruct,
};
use report_json::{json, key_paths, J};
use std::collections::BTreeSet;

fn digest() -> [u8; 32] {
    let mut d = [0u8; 32];
    for (i, b) in d.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(7).wrapping_add(3);
    }
    d
}

fn hex(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn ids_of(entities: &[(EntityId, Curve)]) -> BTreeSet<u32> {
    entities.iter().map(|(id, _)| id.index()).collect()
}

// ---- the export fixtures --------------------------------------------------------------------------------

fn step_report() -> (Report, BTreeSet<u32>) {
    let all = all_entities();
    let ents: Vec<StepEntity> = all.iter().cloned().map(|(id, c)| StepEntity::exact(id, c)).collect();
    let out = step::export(&ents, &StepOptions::new("f.stp", "2026-10-24T12:00:00").unwrap()).unwrap();
    (out.report(digest(), all.len() as u32).unwrap(), ids_of(&all))
}

fn gml_report() -> (Report, BTreeSet<u32>) {
    let all = all_entities();
    let out = run(&all, Transform2::IDENTITY, 0.01);
    (out.report(digest(), all.len() as u32).unwrap(), ids_of(&all))
}

fn gpkg_report(tolerance: f64) -> (Report, BTreeSet<u32>) {
    let all = all_entities();
    let ents: Vec<gpkg::GpkgEntity> =
        all.iter().cloned().map(|(entity_id, curve)| gpkg::GpkgEntity { entity_id, curve }).collect();
    let out = gpkg::export(&ents, &gpkg::GpkgOptions::new(tolerance).unwrap()).unwrap();
    (out.report(digest(), all.len() as u32).unwrap(), ids_of(&all))
}

fn mvt_entities() -> Vec<MvtEntity> {
    let p = |x, y| Point2::new(x, y);
    let unbounded = Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    let circle = |cx, cy, r| {
        Curve::Circle(
            pvec_geometry::Circle::new(Placement2::new(p(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap(),
        )
    };
    vec![
        MvtEntity::new(eid(1), "circles", circle(100.0, 100.0, 80.0)).unwrap(),
        MvtEntity::new(eid(2), "circles", circle(60.0, 40.0, 20.0)).unwrap(),
        MvtEntity::new(eid(3), "lines", unbounded).unwrap(),
    ]
}

const MAX_Z: u8 = 3;

fn tiles(ents: &[MvtEntity]) -> Vec<MvtTile> {
    let pr = Projection::new(Transform2::IDENTITY, 256.0, 256.0).unwrap();
    let mut out = Vec::new();
    for z in 0..=MAX_Z {
        for x in 0..(1u32 << z) {
            for y in 0..(1u32 << z) {
                out.push(
                    mvt::export_tile(ents, &pr, TileId::new(z, x, y).unwrap(), &MvtOptions::default()).unwrap(),
                );
            }
        }
    }
    out
}

fn mvt_report() -> (Report, BTreeSet<u32>) {
    let ents = mvt_entities();
    let ids = ents.iter().map(|e| e.id().index()).collect();
    (mvt::report(&tiles(&ents), digest(), ents.len() as u32).unwrap(), ids)
}

fn provenance() -> Provenance {
    Provenance {
        source_digest: Some(digest()),
        tessellation_tolerance_px: Some(0.25),
        tested_zooms: Some((0, MAX_Z)),
        created: None,
    }
}

fn pmtiles_report() -> (Report, BTreeSet<u32>) {
    let ents = mvt_entities();
    let ids = ents.iter().map(|e| e.id().index()).collect();
    (pmtiles::report(&tiles(&ents), &provenance(), ents.len() as u32).unwrap(), ids)
}

fn every_export() -> Vec<(&'static str, Report, BTreeSet<u32>)> {
    let mut v = Vec::new();
    for (name, (r, ids)) in [
        ("STEP", step_report()),
        ("GML", gml_report()),
        ("GeoPackage", gpkg_report(0.05)),
        ("MVT", mvt_report()),
        ("PMTiles", pmtiles_report()),
    ] {
        v.push((name, r, ids));
    }
    v
}

// ---- a ledger with every class in it ----------------------------------------------------------------------

fn record(entity: u32, cap: u32, rep: Representation, construct: &str, f: Fidelity) -> FidelityRecord {
    FidelityRecord::new(eid(entity), pvec_capabilities::CapabilityId::new(cap), TargetConstruct::new(construct, rep).unwrap(), f, "")
        .unwrap()
}

fn kitchen_sink() -> Report {
    let n = Representation::Native;
    let records = [
        record(0, ids::LINE.get(), n, "LINE", Fidelity::Exact),
        record(1, ids::TRIMMED_CURVE.get(), n, "TRIMMED_CURVE", Fidelity::ExactReparameterized),
        record(2, ids::ELLIPSE.get(), n, "ARCS", Fidelity::EquivalentCompound),
        record(3, ids::BEZIER_CURVE.get(), n, "POLYLINE", Fidelity::BoundedApproximation(ErrorBound::new(0.004).unwrap())),
        record(4, ids::TEXT.get(), n, "none", Fidelity::Unsupported(Reason::new("a component stream is not geometry").unwrap())),
    ];
    Report::new("kitchen sink", "1", digest(), 5, records.iter()).unwrap()
}

// ---- helpers over the document ----------------------------------------------------------------------------

fn doc(r: &Report) -> J {
    json(&r.to_json())
}

fn rows(d: &J) -> Vec<&J> {
    d.get("entities").get("rows").arr().iter().collect()
}

fn count(d: &J, kind: FidelityKind) -> u64 {
    d.get("entities").get("by_fidelity").get(kind.name()).num() as u64
}

// ---- every export produces a report -----------------------------------------------------------------------

#[test]
fn issue_25_every_export_produces_a_report_with_the_version_and_the_digest() {
    for (name, r, _) in every_export() {
        let d = doc(&r);
        assert_eq!(d.get("schema_version").str(), SCHEMA_VERSION, "{name}");
        assert_eq!(d.get("schema").str(), "pvec-fidelity-report", "{name}");
        assert_eq!(d.get("source_digest").str(), format!("sha256:{}", hex(&digest())), "{name}");
        assert!(!d.get("target").get("name").str().is_empty(), "{name}");
        assert!(!d.get("target").get("version").str().is_empty(), "{name}");
        assert!(!r.to_markdown().is_empty(), "{name}");
    }
}

#[test]
fn issue_25_each_export_names_its_target() {
    let names: Vec<String> =
        every_export().iter().map(|(_, r, _)| doc(r).get("target").get("name").str().to_string()).collect();
    assert_eq!(names.len(), 5);
    let distinct: BTreeSet<&String> = names.iter().collect();
    assert_eq!(distinct.len(), 5, "{names:?}");
}

// ---- the counts reconcile --------------------------------------------------------------------------------

#[test]
fn issue_25_counts_reconcile_with_the_artifact_for_every_export() {
    for (name, r, ids) in every_export() {
        let d = doc(&r);
        let total = d.get("entities").get("total").num() as u64;
        assert_eq!(total, ids.len() as u64, "{name}");
        let sum: u64 = FidelityKind::ALL.iter().map(|k| count(&d, *k)).sum();
        assert_eq!(sum, total, "{name}: per-class counts must sum to the total");
        let listed: Vec<u32> = rows(&d).iter().map(|r| r.get("entity").num() as u32).collect();
        let set: BTreeSet<u32> = listed.iter().copied().collect();
        assert_eq!(listed.len(), set.len(), "{name}: an entity appears twice");
        assert_eq!(set, ids, "{name}: an entity is missing or invented");
        for k in FidelityKind::ALL {
            let in_class = rows(&d).iter().filter(|r| r.get("fidelity").str() == k.name()).count() as u64;
            assert_eq!(in_class, count(&d, k), "{name} {}", k.name());
        }
    }
}

#[test]
fn issue_25_a_ledger_that_does_not_cover_the_artifact_is_refused() {
    let r = record(0, ids::LINE.get(), Representation::Native, "LINE", Fidelity::Exact);
    let e = Report::new("t", "1", digest(), 2, [&r]).unwrap_err();
    assert_eq!(e, ReportError::EntityCount { expected: 2, found: 1 });
    let e = Report::new("t", "1", digest(), 0, [&r]).unwrap_err();
    assert_eq!(e, ReportError::EntityCount { expected: 0, found: 1 });
}

#[test]
fn issue_25_an_entity_with_several_records_is_counted_once_in_its_weakest_class() {
    let n = Representation::Native;
    let a = record(0, ids::LINE.get(), n, "LINE", Fidelity::Exact);
    let b = record(0, ids::LINE.get(), n, "ARC", Fidelity::BoundedApproximation(ErrorBound::new(0.5).unwrap()));
    let c = record(0, ids::LINE.get(), n, "ARC", Fidelity::BoundedApproximation(ErrorBound::new(0.25).unwrap()));
    let d = doc(&Report::new("t", "1", digest(), 1, [&a, &b, &c]).unwrap());
    assert_eq!(d.get("entities").get("total").num(), 1.0);
    assert_eq!(count(&d, FidelityKind::BoundedApproximation), 1);
    assert_eq!(count(&d, FidelityKind::Exact), 0);
    let row = rows(&d)[0];
    assert_eq!(row.get("records").num(), 3.0);
    assert_eq!(row.get("measured_error").num(), 0.5, "the entity's error is the largest of its records");
}

#[test]
fn issue_25_every_non_exact_class_lists_its_entities_in_full() {
    for (name, r, _) in every_export().into_iter().chain([("sink", kitchen_sink(), BTreeSet::new())]) {
        let d = doc(&r);
        for k in FidelityKind::ALL.into_iter().filter(|k| *k != FidelityKind::Exact) {
            let listed: Vec<u32> =
                d.get("entities").get("non_exact").get(k.name()).arr().iter().map(|j| j.num() as u32).collect();
            let from_rows: Vec<u32> = rows(&d)
                .iter()
                .filter(|r| r.get("fidelity").str() == k.name())
                .map(|r| r.get("entity").num() as u32)
                .collect();
            assert_eq!(listed, from_rows, "{name} {}", k.name());
        }
        assert!(!d.get("entities").get("non_exact").has("Exact"), "{name}: Exact has no list");
    }
}

// ---- conformance classes ---------------------------------------------------------------------------------

#[test]
fn issue_25_a_tessellated_target_makes_no_structural_or_geometric_claim() {
    for (name, r, _) in [("MVT", mvt_report().0, ()), ("PMTiles", pmtiles_report().0, ())] {
        let d = doc(&r);
        assert_eq!(d.get("target").get("representation").str(), "Tessellated", "{name}");
        for class in ["structural", "geometric"] {
            let c = d.get("conformance").get(class);
            assert_eq!(c, &J::Obj([("claimed".to_string(), J::Bool(false)), ("entities".to_string(), J::Num(0.0))].into()), "{name} {class}");
        }
        assert!(d.get("conformance").get("display").get("claimed") == &J::Bool(true), "{name}");
        for k in [FidelityKind::Exact, FidelityKind::ExactReparameterized, FidelityKind::EquivalentCompound] {
            assert_eq!(count(&d, k), 0, "{name} {}", k.name());
        }
        assert!(rows(&d).iter().all(|r| {
            matches!(r.get("fidelity").str(), "BoundedApproximation" | "Unsupported")
        }));
        let text = r.to_json();
        for banned in ["\"Exact\":[", "structural exact", "geometric exact"] {
            assert!(!text.contains(banned), "{name}: {banned}");
        }
    }
}

#[test]
fn issue_25_a_native_target_separates_the_three_claim_classes() {
    let d = doc(&kitchen_sink());
    let ent = |c: &str| d.get("conformance").get(c).get("entities").num() as u32;
    assert_eq!(ent("structural"), 2, "Exact and ExactReparameterized");
    assert_eq!(ent("geometric"), 3, "plus EquivalentCompound");
    assert_eq!(ent("display"), 1, "BoundedApproximation only");
    assert_eq!(d.get("target").get("representation").str(), "Native");
    for c in ["structural", "geometric", "display"] {
        assert_eq!(d.get("conformance").get(c).get("claimed"), &J::Bool(true), "{c}");
    }
}

// ---- measured error --------------------------------------------------------------------------------------

#[test]
fn issue_25_every_bounded_row_carries_the_measured_error_not_the_requested_tolerance() {
    let requested = 0.5;
    let all = all_entities();
    let ents: Vec<gpkg::GpkgEntity> =
        all.iter().cloned().map(|(entity_id, curve)| gpkg::GpkgEntity { entity_id, curve }).collect();
    let out = gpkg::export(&ents, &gpkg::GpkgOptions::new(requested).unwrap()).unwrap();
    let d = doc(&out.report(digest(), all.len() as u32).unwrap());
    let mut seen = 0;
    for row in rows(&d).into_iter().filter(|r| r.get("fidelity").str() == "BoundedApproximation") {
        seen += 1;
        let e = row.get("measured_error").num();
        let measured = out
            .ledger
            .records()
            .iter()
            .filter(|r| r.entity_id().index() == row.get("entity").num() as u32)
            .filter_map(|r| r.error_bound())
            .map(|b| b.get())
            .fold(0.0, f64::max);
        assert_eq!(e, measured, "the row reports what the ledger measured");
        assert!(e > 0.0 && e <= requested);
        assert_ne!(e, requested, "a requested tolerance is not a measurement");
    }
    assert!(seen > 0, "the fixture must approximate something");
}

#[test]
fn issue_25_rows_that_are_not_approximations_carry_no_error() {
    for (name, r, _) in every_export() {
        for row in rows(&doc(&r)) {
            let bounded = row.get("fidelity").str() == "BoundedApproximation";
            assert_eq!(row.get("measured_error") != &J::Null, bounded, "{name}");
        }
    }
}

#[test]
fn issue_25_the_error_summary_is_the_maximum_and_the_nearest_rank_p99_of_the_rows() {
    let n = Representation::Native;
    let recs: Vec<FidelityRecord> = (1..=200u32)
        .map(|i| record(i, ids::BEZIER_CURVE.get(), n, "P", Fidelity::BoundedApproximation(ErrorBound::new(i as f64 / 1000.0).unwrap())))
        .collect();
    let d = doc(&Report::new("t", "1", digest(), 200, recs.iter()).unwrap());
    let a = d.get("approximation_error");
    assert_eq!(a.get("basis").str(), "measured");
    assert_eq!(a.get("entities").num(), 200.0);
    assert_eq!(a.get("max").num(), 0.2);
    assert_eq!(a.get("p99").num(), 0.198, "ceil(0.99 * 200) = the 198th value");

    let none = doc(&Report::new("t", "1", digest(), 1, [&record(0, ids::LINE.get(), n, "LINE", Fidelity::Exact)]).unwrap());
    assert_eq!(none.get("approximation_error").get("max"), &J::Null);
    assert_eq!(none.get("approximation_error").get("p99"), &J::Null);
}

#[test]
fn issue_25_the_error_summary_agrees_with_the_rows_in_every_export() {
    for (name, r, _) in every_export() {
        let d = doc(&r);
        let errs: Vec<f64> = rows(&d).iter().filter(|r| r.get("measured_error") != &J::Null).map(|r| r.get("measured_error").num()).collect();
        let a = d.get("approximation_error");
        assert_eq!(a.get("entities").num() as usize, errs.len(), "{name}");
        match errs.iter().cloned().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x)))) {
            Some(m) => {
                assert_eq!(a.get("max").num(), m, "{name}");
                assert!(a.get("p99").num() <= m, "{name}");
            }
            None => assert_eq!(a.get("max"), &J::Null, "{name}"),
        }
    }
}

// ---- unsupported -----------------------------------------------------------------------------------------

#[test]
fn issue_25_every_unsupported_row_carries_a_reason_and_a_capability_id() {
    let (r, _) = mvt_report();
    let d = doc(&r);
    let list = d.get("unsupported").arr();
    assert_eq!(list.len() as u64, count(&d, FidelityKind::Unsupported));
    assert!(!list.is_empty(), "the unbounded line cannot be tiled");
    for u in list {
        assert!(!u.get("reason").str().trim().is_empty());
        assert!(u.get("capability_id").num() >= 1.0);
        assert!(!u.get("capability").str().is_empty());
        assert!(!u.get("target_construct").str().is_empty());
    }
    assert_eq!(list[0].get("entity").num(), 3.0);
    assert_eq!(list[0].get("capability_id").num() as u32, ids::LINE.get());

    let s = doc(&kitchen_sink());
    let u = &s.get("unsupported").arr()[0];
    assert_eq!(u.get("reason").str(), "a component stream is not geometry");
    assert_eq!(u.get("capability_id").num() as u32, ids::TEXT.get());
}

// ---- reproducibility -------------------------------------------------------------------------------------

#[test]
fn issue_25_the_same_artifact_and_options_give_a_byte_identical_report() {
    let first: Vec<(String, String)> = every_export().iter().map(|(_, r, _)| (r.to_json(), r.to_markdown())).collect();
    let second: Vec<(String, String)> = every_export().iter().map(|(_, r, _)| (r.to_json(), r.to_markdown())).collect();
    assert_eq!(first, second);
    assert!(first.iter().all(|(j, _)| j.ends_with("}\n") && !j.contains('\r')));
}

#[test]
fn issue_25_the_order_the_records_arrive_in_does_not_change_the_report() {
    let n = Representation::Native;
    let a = record(2, ids::LINE.get(), n, "LINE", Fidelity::Exact);
    let b = record(1, ids::LINE.get(), n, "LINE", Fidelity::Exact);
    let x = Report::new("t", "1", digest(), 2, [&a, &b]).unwrap().to_json();
    let y = Report::new("t", "1", digest(), 2, [&b, &a]).unwrap().to_json();
    assert_eq!(x, y);
}

#[test]
fn issue_25_the_markdown_is_rendered_from_the_same_data() {
    let k = kitchen_sink();
    let md = k.to_markdown();
    let d = doc(&k);
    assert!(md.contains(&format!("sha256:{}", hex(&digest()))));
    assert!(md.contains(SCHEMA_VERSION));
    for kind in FidelityKind::ALL {
        assert!(md.contains(&format!("| {} | {} |", kind.name(), count(&d, kind))), "{}", kind.name());
    }
    assert!(md.contains("a component stream is not geometry"));
    assert!(md.contains("0.004"));
    for (name, r, _) in every_export() {
        let d = doc(&r);
        let md = r.to_markdown();
        for row in rows(&d) {
            assert!(md.contains(&format!("| {} |", row.get("entity").num() as u32)), "{name}");
        }
    }
}

// ---- the schema is versioned -----------------------------------------------------------------------------

fn golden() -> Vec<(String, Vec<String>)> {
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/report_schema_25.txt")).unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split(' ');
            (it.next().unwrap().to_string(), it.map(str::to_string).collect())
        })
        .collect()
}

fn schema_matches_golden(version: &str, keys: &[String]) -> bool {
    golden().iter().any(|(v, k)| v == version && k == keys)
}

#[test]
fn issue_25_the_schema_matches_the_recorded_keys_for_its_version() {
    let keys = key_paths(&doc(&kitchen_sink()));
    assert!(
        schema_matches_golden(SCHEMA_VERSION, &keys),
        "the report keys changed without a schema version bump: bump SCHEMA_VERSION and append a line to \
         tests/report_schema_25.txt for it; current keys: {}",
        keys.join(" ")
    );
}

#[test]
fn issue_25_a_schema_change_without_a_version_bump_fails() {
    let mut keys = key_paths(&doc(&kitchen_sink()));
    assert!(schema_matches_golden(SCHEMA_VERSION, &keys));
    keys.push("an.added.key".to_string());
    assert!(!schema_matches_golden(SCHEMA_VERSION, &keys), "an added key under the same version must not pass");
    keys.pop();
    keys.remove(0);
    assert!(!schema_matches_golden(SCHEMA_VERSION, &keys), "a removed key under the same version must not pass");
}

#[test]
fn issue_25_the_recorded_versions_only_ever_increase() {
    let g = golden();
    assert!(g.iter().any(|(v, _)| v == SCHEMA_VERSION), "the current version has a recorded line");
    let parse = |v: &str| {
        let (a, b) = v.split_once('.').unwrap();
        (a.parse::<u32>().unwrap(), b.parse::<u32>().unwrap())
    };
    for w in g.windows(2) {
        assert!(parse(&w[0].0) < parse(&w[1].0), "{} then {}", w[0].0, w[1].0);
    }
    assert_eq!(g.last().unwrap().0, SCHEMA_VERSION);
}

#[test]
fn issue_25_chapter_7_documents_the_schema() {
    let doc_text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/INTERCHANGE.md")).unwrap();
    let at = doc_text.find("## 7. Fidelity reports").expect("chapter 7");
    let ch = &doc_text[at..];
    assert!(ch.contains(&format!("`{SCHEMA_VERSION}`")), "chapter 7 states the schema version");
    for key in key_paths(&doc(&kitchen_sink())) {
        let leaf = key.rsplit('.').next().unwrap().trim_end_matches("[]");
        assert!(ch.contains(&format!("`{leaf}`")), "chapter 7 does not document `{leaf}`");
    }
}
