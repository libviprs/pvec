//! Exact GML for a trimmed spline (issue P6.4b, pvec#134). A trimmed Bezier or B-spline is written as a
//! `gml:BSpline` of the sub-range (knot insertion at both trim parameters, then a cut), tagged
//! `ExactReparameterized`. A rational spline with unequal weights has no exact GML 3.2.1 construct and
//! stays a measured polyline.

use pvec_geometry::{BSpline, Curve, Curve2d, Point2, Transform2, Trimmed};
use pvec_interchange::FidelityKind;
use pvec_interchange::gml::*;

mod common;
use common::*;

/// Slack for comparing a knot-inserted spline with its source: a few rounded convex combinations on
/// coordinates of size 10, far below any drawing tolerance.
const ROUNDOFF_SLACK: f64 = 0.000_000_001;

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse().unwrap()).collect()
}

/// The `<gml:Curve>` element of entity `id`.
fn member(xml: &str, id: u32) -> &str {
    let start = xml.find(&format!("<gml:Curve gml:id=\"e{id}\">")).expect("the entity is in the document");
    let end = xml[start..].find("</gml:Curve>").unwrap() + start;
    &xml[start..end]
}

/// Degree, control points and the expanded knot vector of the one `gml:BSpline` of entity `id`.
fn read_bspline(xml: &str, id: u32) -> (usize, Vec<Point2>, Vec<f64>) {
    let m = member(xml, id);
    assert_eq!(m.matches("<gml:BSpline").count(), 1, "entity {id} is one gml:BSpline: {m}");
    let degree = between(m, "<gml:degree>", "</gml:degree>").unwrap().parse().unwrap();
    let list = numbers(between(m, ">", "</gml:posList>").and_then(|s| s.rsplit_once('>')).unwrap().1);
    let cps = list.chunks(2).map(|c| pt(c[0], c[1])).collect();
    let mut knots = Vec::new();
    let mut rest = m;
    while let Some(i) = rest.find("<gml:Knot>") {
        let k = &rest[i..];
        let v: f64 = between(k, "<gml:value>", "</gml:value>").unwrap().parse().unwrap();
        let mult: usize = between(k, "<gml:multiplicity>", "</gml:multiplicity>").unwrap().parse().unwrap();
        knots.extend(std::iter::repeat_n(v, mult));
        rest = &k[1..];
    }
    (degree, cps, knots)
}

fn export_one(c: Curve, t: Transform2) -> GmlExport {
    run(&[(eid(1), c)], t, 0.01)
}

fn trim_of(basis: Curve, t1: f64, t2: f64, sense: bool) -> Curve {
    Curve::Trimmed(Trimmed::new(basis, t1, t2, sense).unwrap())
}

/// The exported spline, read back and evaluated, against the trimmed source at its own parameter.
/// The written spline runs over `[0, span]`, which is the trimmed curve's parameter.
fn assert_same_curve(x: &GmlExport, source: &Curve) {
    let (degree, cps, knots) = read_bspline(&x.xml, 1);
    let written = BSpline::new(degree, cps, knots.clone()).expect("the written spline is a valid spline");
    let d = source.domain();
    assert_eq!(knots[0].to_bits(), d.lo().to_bits(), "the spline starts at the trimmed curve's start");
    assert!(
        (written.domain().hi() - d.hi()).abs() <= ROUNDOFF_SLACK * d.hi().max(1.0),
        "and ends at its end"
    );
    assert_eq!(knots.iter().filter(|&&k| k == knots[0]).count(), degree + 1, "clamped at the start");
    assert_eq!(knots.iter().filter(|&&k| k == *knots.last().unwrap()).count(), degree + 1, "and the end");
    for i in 0..=200 {
        let t = d.lo() + d.width() * (i as f64 / 200.0);
        let (a, b) = (source.eval(t).unwrap(), written.eval(t.min(written.domain().hi())).unwrap());
        assert!(a.distance(b) <= ROUNDOFF_SLACK, "tau {t}: {a:?} vs {b:?}");
    }
}

fn assert_exact_reparameterized(x: &GmlExport) {
    assert_eq!(kinds_of(x, 1), vec![FidelityKind::ExactReparameterized]);
    assert_eq!(constructs_of(x, 1), vec!["gml:BSpline"]);
    assert!(!member(&x.xml, 1).contains("LineStringSegment"), "no polyline fallback");
    assert!(records_of(x, 1)[0].error_bound().is_none(), "an exact value carries no bound");
}

#[test]
fn a_trimmed_cubic_spline_is_one_exact_gml_bspline() {
    let c = trim_of(spline(), 0.1, 0.6, true);
    let x = export_one(c.clone(), Transform2::IDENTITY);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c);
}

#[test]
fn a_trim_on_interior_knots_keeps_them_as_the_new_ends() {
    // the spline's interior knots are 0.25 and 0.75
    let c = trim_of(spline(), 0.25, 0.75, true);
    let x = export_one(c.clone(), Transform2::IDENTITY);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c);
    let (degree, cps, _) = read_bspline(&x.xml, 1);
    assert_eq!(cps.len(), degree + 1, "a Bezier piece between two full knots, no spare control points");
}

#[test]
fn a_trim_of_the_whole_domain_is_the_spline_itself() {
    let c = trim_of(spline(), 0.0, 1.0, true);
    let x = export_one(c.clone(), Transform2::IDENTITY);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c);
}

#[test]
fn a_trim_inside_one_knot_span_is_exact() {
    let c = trim_of(spline(), 0.3, 0.4, true);
    let x = export_one(c.clone(), Transform2::IDENTITY);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c);
}

#[test]
fn a_reversed_trim_is_written_in_the_direction_the_curve_runs() {
    let c = trim_of(spline(), 0.7, 0.15, false);
    let x = export_one(c.clone(), Transform2::IDENTITY);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c);
    let (_, cps, _) = read_bspline(&x.xml, 1);
    let start = c.eval(c.domain().lo()).unwrap();
    assert!(
        cps[0].distance(start) <= ROUNDOFF_SLACK,
        "the first control point is where the trimmed curve starts"
    );
}

#[test]
fn a_trimmed_bezier_is_an_exact_gml_bspline() {
    for (t1, t2, sense) in [(0.2, 0.9, true), (0.9, 0.2, false)] {
        let c = trim_of(cubic_bezier(), t1, t2, sense);
        let x = export_one(c.clone(), Transform2::IDENTITY);
        assert_exact_reparameterized(&x);
        assert_same_curve(&x, &c);
    }
}

#[test]
fn the_crs_transform_is_applied_before_the_trim_is_cut() {
    let t = Transform2::new([2.0, 0.0, 0.0, 3.0, 1000.0, 2000.0]).unwrap();
    let c = trim_of(spline(), 0.1, 0.6, true);
    let x = export_one(c.clone(), t);
    assert_exact_reparameterized(&x);
    assert_same_curve(&x, &c.transform(&t).unwrap().0);
}

#[test]
fn the_ledger_note_writes_the_parameter_map_down() {
    let x = export_one(trim_of(spline(), 0.1, 0.6, true), Transform2::IDENTITY);
    let note = records_of(&x, 1)[0].note().to_string();
    assert!(note.contains("tau"), "the note gives the map from the trimmed parameter: {note}");
}

#[test]
fn the_export_is_deterministic() {
    let c = trim_of(spline(), 0.1, 0.6, true);
    assert_eq!(export_one(c.clone(), Transform2::IDENTITY).xml, export_one(c, Transform2::IDENTITY).xml);
}

#[test]
fn the_mapping_table_has_the_exact_trim_row() {
    let row = MAPPING
        .iter()
        .find(|r| r.capability == "trimmed_curve" && r.construct == "gml:BSpline")
        .expect("a trimmed_curve row that writes a gml:BSpline");
    assert_eq!(row.fidelity, FidelityKind::ExactReparameterized);
    assert!(mapping_table_markdown().contains("`trimmed_curve` | TrimmedSpline"));
}

#[test]
fn the_interchange_document_carries_the_weights_finding() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/INTERCHANGE.md")).unwrap();
    assert!(doc.contains("TrimmedSpline"), "the table row is in docs/INTERCHANGE.md");
    assert!(doc.contains("control point weights"), "the finding on weights is written down");
    assert!(doc.contains("3.2.1") && doc.contains("default"), "3.2.1 stays the default profile");
}

// A guard rather than a red test: GML 3.2.1 has no weights, so this must not change.
#[test]
fn a_trimmed_rational_spline_with_unequal_weights_stays_a_measured_polyline() {
    let c = trim_of(rational(vec![1.0, 0.5, 2.0, 1.0, 1.5]), 0.1, 0.9, true);
    let x = export_one(c, Transform2::IDENTITY);
    assert_eq!(kinds_of(&x, 1), vec![FidelityKind::BoundedApproximation]);
    assert_eq!(constructs_of(&x, 1), vec!["gml:LineStringSegment"]);
}
