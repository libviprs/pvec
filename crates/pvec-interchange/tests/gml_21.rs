//! The GML export (issue P6.4, pvec#21): the mapping, the fidelity records, the evidence behind them,
//! determinism and the caller-supplied CRS transform. Schema validation, which needs docker, is in
//! `gml_21_schema.rs`.

use pvec_capabilities::Registry;
use pvec_geometry::{
    Composite, Curve, Curve2d, Line, Point2, Polyline, RationalBSpline, Transform2, Trimmed, Vector2,
};
use pvec_interchange::gml::*;
use pvec_interchange::{Fidelity, FidelityKind, Representation};
use pvec_numeric::Policy as Numeric;
use std::fs;
use std::path::{Path, PathBuf};

mod common;
use common::*;

// ---- reading the XML back ------------------------------------------------------------------------------

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let a = s.find(open)? + open.len();
    let b = s[a..].find(close)? + a;
    Some(&s[a..b])
}

/// The `<gml:Curve>` element of entity `id`.
fn member(xml: &str, id: u32) -> Option<&str> {
    let start = xml.find(&format!("<gml:Curve gml:id=\"e{id}\">"))?;
    let end = xml[start..].find("</gml:Curve>")? + start;
    Some(&xml[start..end])
}

fn numbers(s: &str) -> Vec<f64> {
    s.split_whitespace().map(|t| t.parse().unwrap()).collect()
}

fn pos_lists(s: &str) -> Vec<Vec<Point2>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(i) = rest.find("<gml:posList") {
        let after = &rest[i..];
        let body = between(after, ">", "</gml:posList>").unwrap();
        let v = numbers(body);
        out.push(v.chunks(2).map(|c| pt(c[0], c[1])).collect());
        rest = &after[1..];
    }
    out
}

fn segments_of<'a>(xml: &'a str, id: u32, tag: &str) -> Vec<&'a str> {
    let m = member(xml, id).unwrap_or("");
    let open = format!("<gml:{tag}");
    let close = format!("</gml:{tag}>");
    let mut out = Vec::new();
    let mut rest = m;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i..];
        let end = after.find(&close).unwrap() + close.len();
        out.push(&after[..end]);
        rest = &after[end..];
    }
    out
}

/// Degree, control points, and the knot vector expanded by multiplicity, as read from the XML.
fn read_spline(seg: &str) -> (usize, Vec<Point2>, Vec<f64>) {
    let degree = between(seg, "<gml:degree>", "</gml:degree>").unwrap().parse().unwrap();
    let cps = pos_lists(seg).remove(0);
    let mut knots = Vec::new();
    let mut rest = seg;
    while let Some(i) = rest.find("<gml:Knot>") {
        let k = &rest[i..];
        let value: f64 = between(k, "<gml:value>", "</gml:value>").unwrap().parse().unwrap();
        let mult: usize = between(k, "<gml:multiplicity>", "</gml:multiplicity>").unwrap().parse().unwrap();
        knots.extend(std::iter::repeat_n(value, mult));
        rest = &k[1..];
    }
    (degree, cps, knots)
}

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn pbits(v: &[Point2]) -> Vec<(u64, u64)> {
    v.iter().map(|p| (p.x.to_bits(), p.y.to_bits())).collect()
}

// ---- measuring the approximation against the shared evaluator --------------------------------------------

fn samples(c: &Curve, n: usize) -> Vec<Point2> {
    let d = c.domain();
    (0..=n).map(|i| c.eval(d.lo() + d.width() * (i as f64 / n as f64)).unwrap()).collect()
}

fn dist_seg(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p.x - a.x) * dx + (p.y - a.y) * dy) / l2).clamp(0.0, 1.0) };
    p.distance(pt(a.x + t * dx, a.y + t * dy))
}

fn dist_poly(p: Point2, poly: &[Point2]) -> f64 {
    poly.windows(2).map(|w| dist_seg(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// Distance from a point to the curve: the nearest of a dense sampling, refined by golden-section
/// search on the two neighbouring intervals.
fn dist_curve(p: Point2, c: &Curve, dense: &[Point2]) -> f64 {
    let d = c.domain();
    let n = dense.len() - 1;
    let (i, _) = dense
        .iter()
        .enumerate()
        .map(|(i, q)| (i, p.distance(*q)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap();
    let t_of = |k: f64| d.lo() + d.width() * (k / n as f64);
    let (mut lo, mut hi) = (t_of(i.saturating_sub(1) as f64), t_of((i + 1).min(n) as f64));
    let f = |t: f64| p.distance(c.eval(t.clamp(d.lo(), d.hi())).unwrap());
    for _ in 0..80 {
        let (m1, m2) = (lo + (hi - lo) * 0.382, lo + (hi - lo) * 0.618);
        if f(m1) < f(m2) { hi = m2 } else { lo = m1 }
    }
    f(0.5 * (lo + hi))
}

/// The largest distance in either direction between the exported polyline and the exact curve.
fn hausdorff(poly: &[Point2], c: &Curve) -> f64 {
    let dense = samples(c, 20000);
    let a = dense.iter().map(|&p| dist_poly(p, poly)).fold(0.0, f64::max);
    let mut b = 0.0f64;
    for w in poly.windows(2) {
        for k in 0..=8 {
            let f = k as f64 / 8.0;
            b = b.max(dist_curve(
                pt(w[0].x + (w[1].x - w[0].x) * f, w[0].y + (w[1].y - w[0].y) * f),
                c,
                &dense,
            ));
        }
    }
    a.max(b)
}

fn moved(c: &Curve, t: &Transform2) -> Curve {
    c.transform(t).unwrap().0
}

// ---- the mapping, per case ---------------------------------------------------------------------------------

#[test]
fn every_fixture_gets_the_documented_value_and_construct() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    use FidelityKind::*;
    let expect: [(u32, Vec<FidelityKind>, Vec<&str>); 13] = [
        (POLYLINE, vec![Exact], vec!["gml:LineStringSegment"]),
        (TRIMMED_LINE, vec![ExactReparameterized], vec!["gml:LineStringSegment"]),
        (LINE, vec![Unsupported], vec!["none"]),
        (CIRCLE, vec![EquivalentCompound], vec!["gml:Circle"]),
        (ARC, vec![EquivalentCompound], vec!["gml:Arc"]),
        (FULL_CIRCLE, vec![EquivalentCompound], vec!["gml:Circle"]),
        (ELLIPSE, vec![BoundedApproximation], vec!["gml:LineStringSegment"]),
        (ELLIPSE_ARC, vec![BoundedApproximation], vec!["gml:LineStringSegment"]),
        (BEZIER, vec![Exact], vec!["gml:Bezier"]),
        (BSPLINE, vec![Exact], vec!["gml:BSpline"]),
        (RATIONAL, vec![BoundedApproximation], vec!["gml:LineStringSegment"]),
        (RATIONAL_EQUAL, vec![EquivalentCompound], vec!["gml:BSpline"]),
        (
            COMPOSITE,
            vec![Exact, EquivalentCompound, Exact],
            vec!["gml:LineStringSegment", "gml:Arc", "gml:Bezier"],
        ),
    ];
    for (id, kinds, constructs) in expect {
        assert_eq!(kinds_of(&x, id), kinds, "entity {id}");
        assert_eq!(constructs_of(&x, id), constructs, "entity {id}");
    }
    assert_eq!(x.ledger.records().len(), 15);
}

#[test]
fn the_exact_and_the_compound_values_are_never_tessellated() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    assert!(!x.ledger.records().is_empty());
    for r in x.ledger.records() {
        let tess = r.target_construct().representation() == Representation::Tessellated;
        match r.fidelity().kind() {
            FidelityKind::BoundedApproximation => assert!(tess, "entity {}", r.entity_id().index()),
            FidelityKind::Unsupported => {}
            _ => assert!(!tess, "entity {}", r.entity_id().index()),
        }
    }
}

#[test]
fn an_unsupported_entity_is_in_the_ledger_with_a_reason_and_absent_from_the_xml() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let r = records_of(&x, LINE);
    assert_eq!(r.len(), 1);
    assert!(matches!(r[0].fidelity(), Fidelity::Unsupported(why) if why.as_str().contains("unbounded")));
    assert!(member(&x.xml, LINE).is_none());
    assert!(member(&x.xml, POLYLINE).is_some());
}

#[test]
fn polyline_vertices_are_carried_bit_for_bit() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let seg = segments_of(&x.xml, POLYLINE, "LineStringSegment");
    assert_eq!(seg.len(), 1);
    let got = pos_lists(seg[0]).remove(0);
    let want = [pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 5.0)];
    assert_eq!(pbits(&got), pbits(&want));
}

#[test]
fn a_trimmed_line_is_its_two_end_points() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let got = pos_lists(segments_of(&x.xml, TRIMMED_LINE, "LineStringSegment")[0]).remove(0);
    assert_eq!(pbits(&got), pbits(&[pt(1.0, 1.0), pt(6.0, 1.0)]));
}

#[test]
fn a_circle_is_three_points_on_it_and_the_circle_through_them_is_the_source() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let seg = segments_of(&x.xml, CIRCLE, "Circle");
    assert_eq!(seg.len(), 1);
    let p = pos_lists(seg[0]).remove(0);
    assert_eq!(p.len(), 3);
    for q in &p {
        assert!((q.distance(pt(100.0, 200.0)) - 25.0).abs() <= Numeric::V1.tol(25.0, 225.0));
    }
    // the first point is the start of the circle's own parameterisation
    assert_eq!(pbits(&p[..1]), pbits(&[pt(125.0, 200.0)]));
}

#[test]
fn an_arc_keeps_its_end_points_and_direction() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let p = pos_lists(segments_of(&x.xml, ARC, "Arc")[0]).remove(0);
    let src = &fixtures()[4].1;
    let (a, b) = (src.eval(src.domain().lo()).unwrap(), src.eval(src.domain().hi()).unwrap());
    assert_eq!(pbits(&[p[0], p[2]]), pbits(&[a, b]));
    // the middle point is on the arc side, not the far side
    let mid = src.eval(0.5 * src.domain().hi()).unwrap();
    assert!(p[1].distance(mid) <= Numeric::V1.tol(12.0, 52.0));
}

#[test]
fn a_reversed_arc_runs_the_other_way() {
    let basis = circle(0.0, 0.0, 10.0);
    let fwd = Curve::Trimmed(Trimmed::new(basis.clone(), 0.5, 2.0, true).unwrap());
    let rev = Curve::Trimmed(Trimmed::new(basis, 0.5, 2.0, false).unwrap());
    let x = run(&[(eid(1), fwd), (eid(2), rev)], Transform2::IDENTITY, 0.01);
    let a = pos_lists(segments_of(&x.xml, 1, "Arc")[0]).remove(0);
    let b = pos_lists(segments_of(&x.xml, 2, "Arc")[0]).remove(0);
    let near = |p: Point2, q: Point2| Numeric::V1.near_point([p.x, p.y], [q.x, q.y], 10.0);
    assert!(near(a[0], b[2]) && near(a[2], b[0]));
}

#[test]
fn a_bezier_curve_is_a_gml_bezier_with_the_same_parameters() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let seg = segments_of(&x.xml, BEZIER, "Bezier");
    assert_eq!(seg.len(), 1);
    let (degree, cps, knots) = read_spline(seg[0]);
    let Curve::Bezier(src) = cubic_bezier() else { unreachable!() };
    assert_eq!(degree, src.degree());
    assert_eq!(pbits(&cps), pbits(src.control_points()));
    assert_eq!(bits(&knots), bits(&[0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0]));
}

#[test]
fn a_bspline_round_trips_degree_control_points_and_knots_exactly() {
    for t in [Transform2::IDENTITY, Transform2::new([2.0, 0.0, 0.0, 3.0, 1000.0, 2000.0]).unwrap()] {
        let x = run(&fixtures(), t, 0.01);
        let seg = segments_of(&x.xml, BSPLINE, "BSpline");
        assert_eq!(seg.len(), 1);
        let (degree, cps, knots) = read_spline(seg[0]);
        let Curve::BSpline(src) = moved(&spline(), &t) else { panic!("a spline stays a spline") };
        assert_eq!(degree, src.degree());
        assert_eq!(pbits(&cps), pbits(src.control_points()));
        assert_eq!(bits(&knots), bits(src.knots()));
        // distinct knots with their multiplicities, as GML wants them
        assert_eq!(seg[0].matches("<gml:Knot>").count(), 4);
    }
}

#[test]
fn equal_weights_drop_out_and_the_spline_keeps_its_parameters() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let (degree, cps, knots) = read_spline(segments_of(&x.xml, RATIONAL_EQUAL, "BSpline")[0]);
    let Curve::RationalBSpline(src) = rational(vec![2.0; 5]) else { unreachable!() };
    assert_eq!(degree, src.degree());
    assert_eq!(pbits(&cps), pbits(src.control_points()));
    assert_eq!(bits(&knots), bits(src.knots()));
}

#[test]
fn unequal_weights_are_never_written_as_a_polynomial_spline() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    assert!(segments_of(&x.xml, RATIONAL, "BSpline").is_empty());
    assert_eq!(segments_of(&x.xml, RATIONAL, "LineStringSegment").len(), 1);
}

#[test]
fn a_composite_is_one_curve_with_its_segments_in_order() {
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    let m = member(&x.xml, COMPOSITE).unwrap();
    let order: Vec<usize> =
        ["<gml:LineStringSegment>", "<gml:Arc>", "<gml:Bezier"].iter().map(|t| m.find(t).unwrap()).collect();
    assert!(order[0] < order[1] && order[1] < order[2]);
    assert_eq!(m.matches("<gml:segments>").count(), 1);
}

#[test]
fn a_composite_with_a_member_that_cannot_be_written_is_unsupported_as_a_whole() {
    // the exact members need no budget; an ellipse member does, and then the whole composite is left out
    let x = run(&[(eid(1), composite())], Transform2::IDENTITY, f64::MIN_POSITIVE);
    assert!(x.xml.contains("<gml:Arc>"), "the arc does not need the budget");
    let y = run(
        &[(eid(1), Curve::Composite(Composite::new(vec![trimmed(ellipse(), 0.0, 1.0)]).unwrap()))],
        Transform2::IDENTITY,
        f64::MIN_POSITIVE,
    );
    assert_eq!(kinds_of(&y, 1), vec![FidelityKind::Unsupported]);
    assert!(member(&y.xml, 1).is_none());
}

// ---- the evidence for an approximation --------------------------------------------------------------------

#[test]
fn the_recorded_error_bounds_the_measured_deviation_from_the_exact_curve() {
    let t = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    for b in [0.5, 0.05] {
        let x = run(&fixtures(), t, b);
        for id in [ELLIPSE, ELLIPSE_ARC, RATIONAL] {
            let r = records_of(&x, id);
            assert_eq!(r.len(), 1);
            let bound = r[0].error_bound().expect("an approximation carries its bound").get();
            assert!(bound > 0.0 && bound <= b, "entity {id}: bound {bound} against budget {b}");
            let poly = pos_lists(segments_of(&x.xml, id, "LineStringSegment")[0]).remove(0);
            let src = moved(&fixtures().into_iter().find(|(i, _)| i.index() == id).unwrap().1, &t);
            let measured = hausdorff(&poly, &src);
            assert!(measured <= bound, "entity {id}: measured {measured} exceeds recorded {bound}");
            // the bound is not vacuous: it is within a small factor of what was measured
            assert!(
                bound <= 4.0 * measured.max(b / 64.0),
                "entity {id}: bound {bound} is far above {measured}"
            );
        }
    }
}

#[test]
fn a_tighter_budget_gives_more_vertices_and_a_smaller_bound() {
    let loose = run(&fixtures(), Transform2::IDENTITY, 0.5);
    let tight = run(&fixtures(), Transform2::IDENTITY, 0.005);
    let n = |x: &GmlExport| pos_lists(segments_of(&x.xml, ELLIPSE, "LineStringSegment")[0])[0].len();
    assert!(n(&tight) > n(&loose));
    let bound = |x: &GmlExport| records_of(x, ELLIPSE)[0].error_bound().unwrap().get();
    assert!(bound(&tight) < bound(&loose));
}

#[test]
fn a_spline_is_cut_at_its_knots_so_a_corner_is_never_cut_across() {
    // a degree 1 rational spline is a polyline with a corner at each knot, with weights that differ
    let cps = vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 10.0)];
    let c = Curve::RationalBSpline(
        RationalBSpline::new(1, cps, vec![0.0, 0.0, 0.5, 1.0, 1.0], vec![1.0, 2.0, 1.0]).unwrap(),
    );
    let x = run(&[(eid(1), c.clone())], Transform2::IDENTITY, 0.05);
    let poly = pos_lists(segments_of(&x.xml, 1, "LineStringSegment")[0]).remove(0);
    let bound = records_of(&x, 1)[0].error_bound().unwrap().get();
    assert!(hausdorff(&poly, &c) <= bound);
    assert!(poly.iter().any(|p| p.distance(pt(10.0, 0.0)) <= bound), "the corner is a vertex");
}

#[test]
fn a_budget_that_needs_more_vertices_than_a_polyline_may_have_is_unsupported() {
    let x = run(&[(eid(1), ellipse())], Transform2::IDENTITY, f64::MIN_POSITIVE);
    let r = records_of(&x, 1);
    assert_eq!(r.len(), 1);
    assert!(matches!(r[0].fidelity(), Fidelity::Unsupported(why) if why.as_str().contains("vertices")));
    assert!(member(&x.xml, 1).is_none());
}

#[test]
fn a_very_small_arc_either_proves_its_three_points_or_falls_back_to_a_bounded_polyline() {
    let c = trimmed(circle(1000.0, 1000.0, 1000.0), 1.0, 1.00000001);
    let x = run(&[(eid(1), c.clone())], Transform2::IDENTITY, 0.001);
    let r = records_of(&x, 1);
    assert_eq!(r.len(), 1);
    match r[0].fidelity() {
        Fidelity::EquivalentCompound => {
            assert_eq!(r[0].target_construct().name(), "gml:Arc");
            let p = pos_lists(segments_of(&x.xml, 1, "Arc")[0]).remove(0);
            // the claim is checked here from the XML alone: the arc's end points are the source's
            let (a, b) = (c.eval(c.domain().lo()).unwrap(), c.eval(c.domain().hi()).unwrap());
            assert_eq!(pbits(&[p[0], p[2]]), pbits(&[a, b]));
        }
        Fidelity::BoundedApproximation(bound) => {
            assert_eq!(r[0].target_construct().representation(), Representation::Tessellated);
            let poly = pos_lists(segments_of(&x.xml, 1, "LineStringSegment")[0]).remove(0);
            assert!(hausdorff(&poly, &c) <= bound.get());
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ---- the CRS and the transform ---------------------------------------------------------------------------

#[test]
fn the_crs_name_is_required_and_the_export_never_picks_one() {
    assert_eq!(CrsTransform::new("", Transform2::IDENTITY), Err(GmlError::BadSrsName));
    assert_eq!(CrsTransform::new("   ", Transform2::IDENTITY), Err(GmlError::BadSrsName));
    assert_eq!(CrsTransform::new("EPSG:1\n", Transform2::IDENTITY), Err(GmlError::BadSrsName));
    assert!(CrsTransform::new(SRS, Transform2::IDENTITY).is_ok());
}

#[test]
fn the_output_names_the_crs_and_records_the_transform() {
    let t = Transform2::new([2.0, 0.0, 0.0, 3.0, 1000.5, -2000.25]).unwrap();
    let x = run(&fixtures(), t, 0.01);
    assert!(x.xml.contains(&format!("srsName=\"{SRS}\"")));
    let d = between(&x.xml, "<gml:description>", "</gml:description>").unwrap();
    for needle in ["a=2", "b=0", "tx=1000.5", "c=0", "d=3", "ty=-2000.25", "GML 3.2.1"] {
        assert!(d.contains(needle), "{needle} missing from {d}");
    }
}

#[test]
fn the_transform_is_applied_to_the_geometry() {
    let t = Transform2::new([1.0, 0.0, 0.0, 1.0, 1000.0, 2000.0]).unwrap();
    let x = run(&fixtures(), t, 0.01);
    let got = pos_lists(segments_of(&x.xml, POLYLINE, "LineStringSegment")[0]).remove(0);
    assert_eq!(pbits(&got), pbits(&[pt(1000.0, 2000.0), pt(1010.0, 2000.0), pt(1010.0, 2005.0)]));
}

#[test]
fn a_circle_under_a_non_uniform_scale_becomes_an_ellipse_and_is_approximated() {
    let t = Transform2::new([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    let x = run(&[(eid(1), circle(0.0, 0.0, 10.0))], t, 0.01);
    assert_eq!(kinds_of(&x, 1), vec![FidelityKind::BoundedApproximation]);
}

#[test]
fn the_srs_name_is_escaped() {
    let c = CrsTransform::new("urn:x&y\"<z>", Transform2::IDENTITY).unwrap();
    let x = export(&fixtures(), &c, budget(0.01)).unwrap();
    assert!(x.xml.contains("srsName=\"urn:x&amp;y&quot;&lt;z&gt;\""));
}

#[test]
fn bad_inputs_are_refused() {
    assert_eq!(Budget::new(0.0), Err(GmlError::BadBudget));
    assert_eq!(Budget::new(-1.0), Err(GmlError::BadBudget));
    assert_eq!(Budget::new(f64::NAN), Err(GmlError::BadBudget));
    assert_eq!(Budget::new(f64::INFINITY), Err(GmlError::BadBudget));
    let dup = vec![fixtures().remove(0), fixtures().remove(0)];
    assert_eq!(
        export(&dup, &crs(Transform2::IDENTITY), budget(0.01)).unwrap_err(),
        GmlError::DuplicateEntity(eid(POLYLINE))
    );
}

// ---- determinism -------------------------------------------------------------------------------------------

#[test]
fn identical_input_gives_byte_identical_xml_and_ledger() {
    let t = Transform2::new([1.5, 0.5, -0.25, 2.0, 40.0, -10.0]).unwrap();
    let a = run(&fixtures(), t, 0.02);
    let b = run(&fixtures(), t, 0.02);
    assert_eq!(a.xml.as_bytes(), b.xml.as_bytes());
    assert_eq!(a.ledger.to_text(), b.ledger.to_text());
    assert!(!a.xml.is_empty());
}

#[test]
fn the_document_is_exactly_this_for_a_small_input() {
    let ents = vec![
        (eid(7), Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(1.5, -0.0)]).unwrap())),
        (eid(9), trimmed(Curve::Line(Line::new(pt(0.0, 0.0), Vector2::new(0.0, 1.0)).unwrap()), 0.0, 2.0)),
    ];
    let x = run(&ents, Transform2::IDENTITY, 0.5);
    let want = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<gml:MultiCurve xmlns:gml=\"http://www.opengis.net/gml/3.2\" gml:id=\"pvec-export\" srsName=\"urn:ogc:def:crs:EPSG::25832\">\n",
        "<gml:description>PVec export, GML 3.2.1. Source-to-CRS affine transform, rows [a b tx] and [c d ty]: a=1 b=0 tx=0 c=0 d=1 ty=0. Approximated curves stay within 0.5 of the source.</gml:description>\n",
        "<gml:curveMember><gml:Curve gml:id=\"e7\"><gml:identifier codeSpace=\"urn:pvec:entity\">7</gml:identifier><gml:segments><gml:LineStringSegment><gml:posList srsDimension=\"2\">0 0 1.5 0</gml:posList></gml:LineStringSegment></gml:segments></gml:Curve></gml:curveMember>\n",
        "<gml:curveMember><gml:Curve gml:id=\"e9\"><gml:identifier codeSpace=\"urn:pvec:entity\">9</gml:identifier><gml:segments><gml:LineStringSegment><gml:posList srsDimension=\"2\">0 0 0 2</gml:posList></gml:LineStringSegment></gml:segments></gml:Curve></gml:curveMember>\n",
        "</gml:MultiCurve>\n",
    );
    assert_eq!(x.xml, want);
}

// ---- the mapping table and the document ----------------------------------------------------------------------

#[test]
fn every_capability_has_a_row_and_every_row_has_exactly_one_value() {
    let registry = Registry::v1();
    for cap in registry.entries() {
        let rows: Vec<_> = MAPPING.iter().filter(|r| r.capability == cap.name).collect();
        assert!(!rows.is_empty(), "capability {} has no row", cap.name);
    }
    for r in MAPPING {
        assert!(registry.by_name(r.capability).is_some(), "row for unknown capability {}", r.capability);
        assert!(!r.note.trim().is_empty());
    }
}

#[test]
fn a_row_without_an_exact_construct_is_never_exact() {
    for r in MAPPING {
        let exact = matches!(
            r.fidelity,
            FidelityKind::Exact | FidelityKind::ExactReparameterized | FidelityKind::EquivalentCompound
        );
        if r.representation == Representation::Tessellated {
            assert_eq!(r.fidelity, FidelityKind::BoundedApproximation, "{r:?}");
        }
        if r.construct == "none" {
            assert_eq!(r.fidelity, FidelityKind::Unsupported, "{r:?}");
        }
        if r.fidelity == FidelityKind::Unsupported {
            assert_eq!(r.construct, "none", "{r:?}");
        }
        if exact {
            assert_eq!(r.representation, Representation::Native, "{r:?}");
        }
    }
}

#[test]
fn every_emitted_record_is_a_row_of_the_table() {
    let registry = Registry::v1();
    let x = run(&fixtures(), Transform2::IDENTITY, 0.01);
    for r in x.ledger.records() {
        if r.fidelity().kind() == FidelityKind::Unsupported {
            continue;
        }
        let name = registry.get(r.source_capability()).unwrap().name;
        assert!(
            MAPPING.iter().any(|m| m.capability == name
                && m.construct == r.target_construct().name()
                && m.fidelity == r.fidelity().kind()),
            "no row for {name} -> {} as {:?}",
            r.target_construct().name(),
            r.fidelity().kind()
        );
    }
    assert!(x.ledger.unregistered(&registry).is_empty());
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_interchange_document_declares_the_profile_and_carries_the_table() {
    let path = repo().join("docs/INTERCHANGE.md");
    let mut doc = fs::read_to_string(&path).expect("docs/INTERCHANGE.md");
    if std::env::var_os("PVEC_BLESS_INTERCHANGE").is_some() {
        // PVEC_BLESS_INTERCHANGE=1 cargo test -p pvec-interchange --test gml_21 rewrites the generated table
        let (a, b) = ("<!-- mapping-table:start -->\n", "<!-- mapping-table:end -->");
        let (i, j) = (doc.find(a).unwrap() + a.len(), doc.find(b).unwrap());
        doc = format!("{}{}{}", &doc[..i], mapping_table_markdown(), &doc[j..]);
        fs::write(&path, &doc).unwrap();
    }
    assert!(doc.contains("## 3. GML export"));
    assert!(doc.contains(&format!("GML {GML_VERSION}")));
    assert!(doc.contains(GML_NAMESPACE));
    let table = mapping_table_markdown();
    assert!(!table.is_empty());
    assert!(doc.contains(&table), "the mapping table in docs/INTERCHANGE.md differs from the code's");
}
