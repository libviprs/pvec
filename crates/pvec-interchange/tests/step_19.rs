//! The STEP AP242 / Part 21 reference export (issue P6.2, pvec#19).
//!
//! Red first: these tests describe the export before it exists. They cover the mapping table, the
//! header, determinism, numbering, the structural round trip through the in-process reader
//! (`step_reader`), the ledger, the transform, and the refusal of previews.

use pvec_capabilities::Registry;
use pvec_corpus::sha256::hex_digest;
use pvec_entity::{EntityId, EntityTable, Representation as EntityRep};
use pvec_geometry::{Curve, Curve2d, Transform2};
use pvec_interchange::step::*;
use pvec_interchange::{Fidelity, FidelityKind, Representation};
use std::collections::BTreeSet;

mod common;
use common::*;
mod step_reader;

const STAMP: &str = "2026-10-24T12:00:00";

fn opts() -> StepOptions {
    StepOptions::new("fixtures.stp", STAMP).unwrap()
}

fn entities() -> Vec<StepEntity> {
    fixtures().into_iter().map(|(id, c)| StepEntity::exact(id, c)).collect()
}

fn run() -> StepExport {
    export(&entities(), &opts()).unwrap()
}

// ---- the mapping table ----------------------------------------------------------------------------------

#[test]
fn every_registry_capability_has_a_named_mapping_or_an_unsupported_row_with_a_reason() {
    for cap in Registry::v1().entries().iter() {
        let rows: Vec<&MappingRow> = MAPPING.iter().filter(|r| r.capability == cap.name).collect();
        assert!(!rows.is_empty(), "capability {} has no row: it would be silently dropped", cap.name);
        for r in rows {
            if r.fidelity == FidelityKind::Unsupported {
                assert!(
                    !r.note.trim().is_empty(),
                    "{} {:?}: an Unsupported row needs a reason",
                    cap.name,
                    r.case
                );
                assert_eq!(
                    r.construct, "none",
                    "{} {:?}: an unsupported row writes nothing",
                    cap.name, r.case
                );
            } else {
                assert_ne!(r.construct, "none", "{} {:?}: a mapped row names its entity", cap.name, r.case);
            }
        }
    }
}

#[test]
fn no_row_is_a_tessellation_and_no_row_is_approximate() {
    assert!(!MAPPING.is_empty());
    for r in MAPPING {
        assert_eq!(r.representation, Representation::Native, "{} {:?}", r.capability, r.case);
        assert_ne!(r.fidelity, FidelityKind::BoundedApproximation, "{} {:?}", r.capability, r.case);
    }
}

#[test]
fn the_geometry_capabilities_name_the_part_42_constructs_of_the_profile() {
    let construct = |cap: &str| -> BTreeSet<&str> {
        MAPPING.iter().filter(|r| r.capability == cap).map(|r| r.construct).collect()
    };
    assert!(construct("cartesian_point").contains("CARTESIAN_POINT"));
    assert!(construct("line").contains("LINE"));
    assert!(construct("polyline").contains("POLYLINE"));
    assert!(construct("circle").contains("CIRCLE"));
    assert!(construct("ellipse").contains("ELLIPSE"));
    assert!(construct("trimmed_curve").contains("TRIMMED_CURVE"));
    assert!(construct("composite_curve").contains("COMPOSITE_CURVE"));
    assert!(construct("bezier_curve").contains("B_SPLINE_CURVE_WITH_KNOTS"));
    assert!(construct("b_spline_curve").contains("B_SPLINE_CURVE_WITH_KNOTS"));
    assert!(construct("rational_b_spline_curve").iter().any(|c| c.contains("RATIONAL_B_SPLINE_CURVE")));
}

#[test]
fn chapter_one_of_the_interchange_document_carries_the_generated_table() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/INTERCHANGE.md")).unwrap();
    let table = mapping_table_markdown();
    assert!(table.lines().count() > 5, "the generated table is empty");
    assert!(doc.contains("## 1. STEP AP242 export"), "chapter 1 is missing");
    assert!(
        doc.contains(&table),
        "docs/INTERCHANGE.md chapter 1 does not contain mapping_table_markdown() byte for byte"
    );
}

// ---- the file ---------------------------------------------------------------------------------------------

#[test]
fn the_file_is_part_21_with_the_ap242_schema_and_only_the_callers_timestamp() {
    let x = run();
    let t = &x.part21;
    assert!(t.starts_with("ISO-10303-21;\nHEADER;\n"));
    assert!(t.trim_end().ends_with("END-ISO-10303-21;"));
    assert!(t.contains("FILE_DESCRIPTION("));
    assert!(t.contains(&format!("FILE_NAME('fixtures.stp','{STAMP}'")));
    assert!(t.contains("FILE_SCHEMA(('AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF"));
    // the only date-like text is the caller's: removing it leaves no `YYYY-MM-DDT` anywhere
    let rest = t.replace(STAMP, "");
    assert!(!rest.contains("T12:"), "a second timestamp is in the header");
    assert!(!rest.contains("2026-"), "a second date is in the header");
}

#[test]
fn the_file_is_ascii_and_every_real_has_a_decimal_point() {
    let x = run();
    assert!(x.part21.is_ascii());
    let file = step_reader::parse(&x.part21); // the reader panics on a real without a point
    assert!(!file.instances.is_empty());
    for tok in x.part21.split(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | 'E' | '-' | '+'))) {
        if tok.contains('E') {
            let (m, e) = tok.split_once('E').unwrap();
            assert!(m.contains('.'), "exponent form without a point: {tok}");
            assert!(e.trim_start_matches(['+', '-']).chars().all(|c| c.is_ascii_digit()) && !e.is_empty());
        }
    }
}

#[test]
fn exporting_twice_is_byte_identical() {
    assert_eq!(run().part21, run().part21);
    assert_eq!(run().ledger.to_text(), run().ledger.to_text());
}

#[test]
fn instance_numbers_start_at_one_are_dense_and_every_reference_resolves() {
    let x = run();
    let file = step_reader::parse(&x.part21);
    let ids: Vec<u32> = file.instances.iter().map(|i| i.id).collect();
    assert_eq!(ids, (1..=ids.len() as u32).collect::<Vec<_>>(), "numbering is dense and ascending");
    let defined: BTreeSet<u32> = ids.into_iter().collect();
    let mut refs = BTreeSet::new();
    for tok in x.part21.split('#').skip(1) {
        let n: String = tok.chars().take_while(char::is_ascii_digit).collect();
        refs.insert(n.parse::<u32>().unwrap());
    }
    assert!(
        refs.is_subset(&defined),
        "dangling reference(s): {:?}",
        refs.difference(&defined).collect::<Vec<_>>()
    );
}

#[test]
fn the_numbering_follows_the_order_given_and_not_the_ids() {
    let mut reversed = entities();
    reversed.reverse();
    let a = export(&entities(), &opts()).unwrap();
    let b = export(&reversed, &opts()).unwrap();
    assert_ne!(a.part21, b.part21);
    let order = |x: &StepExport| -> Vec<u32> {
        step_reader::parse(&x.part21).entities().iter().map(|(n, _)| *n).collect()
    };
    let mut want: Vec<u32> = fixtures().iter().map(|(id, _)| id.index()).collect();
    assert_eq!(order(&a), want);
    want.reverse();
    assert_eq!(order(&b), want);
}

#[test]
fn the_header_strings_are_escaped_and_non_ascii_is_encoded() {
    let o = StepOptions::new("it's \\ \u{e9}.stp", STAMP).unwrap();
    let x = export(&entities(), &o).unwrap();
    assert!(x.part21.is_ascii());
    assert!(x.part21.contains("FILE_NAME('it''s \\\\ \\X2\\00E9\\X0\\.stp'"), "{}", &x.part21[..400]);
}

#[test]
fn bad_header_input_is_refused() {
    assert_eq!(StepOptions::new("", STAMP).unwrap_err(), StepError::BadFileName);
    assert_eq!(StepOptions::new("a\nb", STAMP).unwrap_err(), StepError::BadFileName);
    for bad in ["", "now", "2026-10-24", "2026-10-24 12:00:00", "2026-13-24T12:00:00", "2026-10-24T25:00:00"]
    {
        assert_eq!(StepOptions::new("a.stp", bad).unwrap_err(), StepError::BadTimestamp, "{bad:?}");
    }
}

#[test]
fn a_duplicate_entity_id_is_refused() {
    let mut e = entities();
    e.push(StepEntity::exact(eid(CIRCLE), circle(0.0, 0.0, 1.0)));
    assert_eq!(export(&e, &opts()).unwrap_err(), StepError::DuplicateEntity(eid(CIRCLE)));
}

#[test]
fn an_empty_export_is_a_valid_file() {
    let x = export(&[], &opts()).unwrap();
    step_reader::parse(&x.part21);
    assert!(x.ledger.records().is_empty());
}

// ---- structural exactness -----------------------------------------------------------------------------------

fn extra_fixtures() -> Vec<(EntityId, Curve)> {
    use pvec_geometry::{BSpline, Circle, Line, Placement2, Trimmed, Vector2};
    let line = Line::new(pt(-3.5, 2.25), Vector2::new(3.0, 4.0)).unwrap();
    let rotated = Circle::new(Placement2::new(pt(7.0, -2.0), Vector2::new(0.6, 0.8)).unwrap(), 3.25).unwrap();
    vec![
        (eid(20), Curve::Trimmed(Trimmed::new(Curve::Line(line), 5.0, 1.0, false).unwrap())),
        (eid(21), Curve::Trimmed(Trimmed::new(Curve::Circle(rotated), 5.5, 1.0, true).unwrap())),
        (eid(22), Curve::Trimmed(Trimmed::new(spline(), 0.2, 0.9, true).unwrap())),
        (eid(23), Curve::Trimmed(Trimmed::new(cubic_bezier(), 0.9, 0.1, false).unwrap())),
        (
            eid(24),
            Curve::Trimmed(Trimmed::new(rational(vec![1.0, 0.5, 2.0, 1.0, 1.5]), 0.1, 0.8, true).unwrap()),
        ),
        (eid(25), Curve::Circle(rotated)),
        (
            eid(26),
            Curve::BSpline(
                BSpline::new(
                    2,
                    vec![pt(0.0, 0.0), pt(1.0, 2.0), pt(2.0, 0.0), pt(1.0, -2.0), pt(0.0, 0.0)],
                    vec![0.0, 0.0, 0.0, 0.5, 0.5, 1.0, 1.0, 1.0],
                )
                .unwrap()
                .declare_closed()
                .unwrap(),
            ),
        ),
    ]
}

fn all_entities() -> Vec<(EntityId, Curve)> {
    let mut v = fixtures();
    v.extend(extra_fixtures());
    v
}

fn assert_same_trim(a: &Curve, b: &Curve) {
    let (Curve::Trimmed(x), Curve::Trimmed(y)) = (a, b) else { panic!("both are trimmed curves") };
    assert_eq!(x.basis(), y.basis(), "the basis is carried unchanged");
    let (tx, ty) = (x.trim(), y.trim());
    assert_eq!(tx.reversed, ty.reversed, "the direction is carried");
    assert_eq!(tx.start.to_bits(), ty.start.to_bits(), "the start of the trim domain is carried bit for bit");
    let scale = a.bounds().unwrap().diagonal();
    let tol = pvec_numeric::Policy::V1.tol(scale.max(tx.span), tx.start.abs() + tx.span);
    assert!((tx.span - ty.span).abs() <= tol, "span {} vs {}", tx.span, ty.span);
}

fn assert_same(src: &Curve, back: &Curve) {
    match (src, back) {
        (Curve::Trimmed(_), _) => assert_same_trim(src, back),
        (Curve::Bezier(b), _) => assert_eq!(&step_reader::as_bspline(b), back),
        (Curve::Composite(a), Curve::Composite(b)) => {
            assert_eq!(a.segments().len(), b.segments().len());
            for (x, y) in a.segments().iter().zip(b.segments()) {
                assert_same(x, y);
            }
        }
        _ => assert_eq!(src, back),
    }
}

#[test]
fn the_round_trip_through_the_reader_is_structurally_exact_for_every_entity() {
    let ents: Vec<StepEntity> = all_entities().into_iter().map(|(i, c)| StepEntity::exact(i, c)).collect();
    let x = export(&ents, &opts()).unwrap();
    let file = step_reader::parse(&x.part21);
    let found = file.entities();
    assert_eq!(found.len(), all_entities().len());
    for ((n, inst), (id, curve)) in found.iter().zip(all_entities()) {
        assert_eq!(*n, id.index());
        let back = file.curve(*inst);
        if matches!(curve, Curve::Line(_)) {
            // an unbounded line is written as itself
            assert_eq!(curve, back);
        } else {
            assert_same(&curve, &back);
        }
    }
}

#[test]
fn degree_control_points_knots_multiplicities_weights_and_closure_are_written_as_they_are() {
    let x = run();
    let file = step_reader::parse(&x.part21);
    let inst = file.by_label(&format!("e{BSPLINE}")).unwrap();
    let p = &inst.parts[0];
    assert_eq!(p.name, "B_SPLINE_CURVE_WITH_KNOTS");
    assert_eq!(p.args[1], step_reader::Val::Int(3));
    assert_eq!(p.args[3], step_reader::Val::Enum("UNSPECIFIED".into()));
    assert_eq!(p.args[4], step_reader::Val::Enum("F".into()), "closed_curve");
    assert_eq!(p.args[5], step_reader::Val::Enum("U".into()), "self_intersect is not computed");
    assert_eq!(
        p.args[6],
        step_reader::Val::List(vec![
            step_reader::Val::Int(4),
            step_reader::Val::Int(1),
            step_reader::Val::Int(1),
            step_reader::Val::Int(4)
        ])
    );

    let ents: Vec<StepEntity> = extra_fixtures().into_iter().map(|(i, c)| StepEntity::exact(i, c)).collect();
    let y = export(&ents, &opts()).unwrap();
    let f2 = step_reader::parse(&y.part21);
    let closed = &f2.by_label("e26").unwrap().parts[0];
    assert_eq!(
        closed.args[4],
        step_reader::Val::Enum("T".into()),
        "a declared-closed spline is written closed"
    );

    let rat = file.by_label(&format!("e{RATIONAL}")).unwrap();
    let names: Vec<&str> = rat.parts.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"RATIONAL_B_SPLINE_CURVE") && names.contains(&"B_SPLINE_CURVE_WITH_KNOTS"));
}

#[test]
fn a_reversed_trim_is_written_with_sense_false_and_the_ends_swapped() {
    let ents = vec![StepEntity::exact(eid(20), extra_fixtures().remove(0).1)];
    let x = export(&ents, &opts()).unwrap();
    let file = step_reader::parse(&x.part21);
    let t = &file.by_label("e20").unwrap().parts[0];
    assert_eq!(t.name, "TRIMMED_CURVE");
    assert_eq!(t.args[4], step_reader::Val::Enum("F".into()));
    assert_eq!(t.args[5], step_reader::Val::Enum("PARAMETER".into()));
}

// ---- the ledger ---------------------------------------------------------------------------------------------

#[test]
fn every_entity_has_exactly_one_record_and_the_counts_reconcile() {
    let x = run();
    let n = fixtures().len();
    assert_eq!(x.ledger.records().len(), n);
    let ids: BTreeSet<u32> = x.ledger.records().iter().map(|r| r.entity_id().index()).collect();
    assert_eq!(ids.len(), n, "one record per entity");
    let total: usize = FidelityKind::ALL.into_iter().map(|k| x.ledger.count(k)).sum();
    assert_eq!(total, n, "the per-value counts add up to the entity count");
    assert!(x.ledger.unregistered(&Registry::v1()).is_empty());
}

#[test]
fn the_fidelity_values_are_the_ones_the_table_gives() {
    use FidelityKind::*;
    let x = run();
    let want = [
        (POLYLINE, Exact),
        (TRIMMED_LINE, ExactReparameterized),
        (LINE, Exact),
        (CIRCLE, Exact),
        (ARC, ExactReparameterized),
        (FULL_CIRCLE, ExactReparameterized),
        (ELLIPSE, Exact),
        (ELLIPSE_ARC, ExactReparameterized),
        (BEZIER, Exact),
        (BSPLINE, Exact),
        (RATIONAL, Exact),
        (RATIONAL_EQUAL, Exact),
        (COMPOSITE, ExactReparameterized),
    ];
    for (id, kind) in want {
        let recs = records_of_step(&x, id);
        assert_eq!(recs.len(), 1, "entity {id}");
        assert_eq!(recs[0].fidelity().kind(), kind, "entity {id}");
        assert_eq!(recs[0].target_construct().representation(), Representation::Native);
        assert!(!recs[0].note().is_empty() || kind == Exact);
    }
    assert_eq!(x.ledger.count(BoundedApproximation), 0);
    assert_eq!(x.ledger.count(Unsupported), 0);
}

fn records_of_step(x: &StepExport, id: u32) -> Vec<&pvec_interchange::FidelityRecord> {
    x.ledger.records().iter().filter(|r| r.entity_id().index() == id).collect()
}

#[test]
fn a_ledger_record_names_the_entity_it_wrote() {
    let x = run();
    let r = records_of_step(&x, CIRCLE)[0];
    assert_eq!(r.target_construct().name(), "CIRCLE");
    let r = records_of_step(&x, RATIONAL)[0];
    assert!(r.target_construct().name().contains("RATIONAL_B_SPLINE_CURVE"));
    assert_eq!(r.source_capability(), Registry::v1().by_name("rational_b_spline_curve").unwrap().id);
}

// ---- transforms --------------------------------------------------------------------------------------------

#[test]
fn a_similarity_is_applied_to_the_geometry_and_a_circle_stays_a_circle() {
    let t = Transform2::rotate(0.5).unwrap().then(&Transform2::translate(10.0, -4.0).unwrap()).unwrap();
    let o = opts().with_transform(t);
    let x = export(&entities(), &o).unwrap();
    let file = step_reader::parse(&x.part21);
    let inst = file.by_label(&format!("e{CIRCLE}")).unwrap();
    assert_eq!(inst.parts[0].name, "CIRCLE");
    let want = fixtures().into_iter().find(|(i, _)| i.index() == CIRCLE).unwrap().1.transform(&t).unwrap().0;
    assert_eq!(file.curve(inst.id), want);
}

#[test]
fn a_non_similarity_promotes_a_circle_to_an_ellipse_exactly() {
    let t = Transform2::scale(2.0, 0.5).unwrap();
    let x = export(&entities(), &opts().with_transform(t)).unwrap();
    let file = step_reader::parse(&x.part21);
    let inst = file.by_label(&format!("e{CIRCLE}")).unwrap();
    assert_eq!(inst.parts[0].name, "ELLIPSE");
    let want = fixtures().into_iter().find(|(i, _)| i.index() == CIRCLE).unwrap().1.transform(&t).unwrap().0;
    assert_eq!(file.curve(inst.id), want);
    let r = records_of_step(&x, CIRCLE)[0];
    assert_eq!(r.fidelity().kind(), FidelityKind::Exact, "the image is an ellipse and is written as one");
}

#[test]
fn a_transform_the_curve_model_refuses_is_an_unsupported_record_with_the_reason() {
    // scaling a circle by a million squared in one axis makes an ellipse more eccentric than NP-07 allows
    let t = Transform2::scale(1.0e7, 1.0).unwrap();
    let x = export(&entities(), &opts().with_transform(t)).unwrap();
    let recs = records_of_step(&x, CIRCLE);
    assert_eq!(recs.len(), 1);
    match recs[0].fidelity() {
        Fidelity::Unsupported(why) => assert!(why.as_str().contains("NP-07"), "{}", why.as_str()),
        other => panic!("expected Unsupported, got {other:?}"),
    }
    let file = step_reader::parse(&x.part21);
    assert!(file.by_label(&format!("e{CIRCLE}")).is_none(), "nothing is written for it");
    let total: usize = FidelityKind::ALL.into_iter().map(|k| x.ledger.count(k)).sum();
    assert_eq!(total, fixtures().len(), "the counts still reconcile");
}

#[test]
fn the_transform_is_stated_in_the_header_so_the_file_says_what_was_done() {
    let t = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    let x = export(&entities(), &opts().with_transform(t)).unwrap();
    assert!(x.part21.contains("1.5") && x.part21.contains("40."), "the matrix is in FILE_DESCRIPTION");
    let plain = run();
    assert!(plain.part21.contains("identity"), "an untransformed file says so");
}

// ---- no preview ---------------------------------------------------------------------------------------------

#[test]
fn a_preview_entity_is_refused_when_the_table_says_it_is_one() {
    let mut table = EntityTable::new();
    let curve = circle(1.0, 2.0, 3.0);
    let b = curve.bounds().unwrap();
    let exact = table.push(curve.kind(), b, EntityRep::Exact).unwrap();
    let preview = table.push(curve.kind(), b, EntityRep::Preview).unwrap();
    assert!(StepEntity::from_table(&table, exact, curve.clone()).is_ok());
    assert_eq!(
        StepEntity::from_table(&table, preview, curve.clone()).unwrap_err(),
        StepError::Preview(preview)
    );
    let missing = EntityId::new(9).unwrap();
    assert_eq!(
        StepEntity::from_table(&table, missing, curve).unwrap_err(),
        StepError::UnknownEntity(missing)
    );
}

#[test]
fn the_writer_documents_the_compile_fail_cases_that_keep_tessellations_out() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/step/writer.rs")).unwrap();
    assert!(
        src.matches("```compile_fail").count() >= 2,
        "doctests must show the writer refusing other input types"
    );
}

// ---- the digest -----------------------------------------------------------------------------------------------

/// The SHA-256 of the fixture export. The same constant has to come out on arm64 and on native x86_64,
/// which the docker-tests lane checks on both (nothing in the file depends on the host).
const FIXTURE_DIGEST: &str = "pending";

#[test]
fn the_fixture_export_has_the_pinned_digest_on_every_architecture() {
    let x = run();
    assert_eq!(hex_digest(x.part21.as_bytes()), FIXTURE_DIGEST, "arch {}", std::env::consts::ARCH);
}
