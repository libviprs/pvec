//! Issue #101: the representation gaps the canonical generator refused. What pvec can hold exactly
//! and deterministically is generated now, what it cannot stays refused with the capability named.

mod common;

use common::*;
use pvec_corpus::FixtureRegistry;
use pvec_geometry::{BSpline, Curve, Point2, Trimmed};
use pvec_numeric::Policy;
use pvec_testkit::detmath::{atan, atan2};
use pvec_testkit::generate::{Artifact, Cause, generate_all};
use pvec_testkit::reference::{ReadEntity, read};
use std::collections::BTreeMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};

const ABS: f64 = Policy::V1.abs;
const REL: f64 = Policy::V1.rel;

fn all() -> BTreeMap<String, Artifact> {
    generate_all(&bundle())
        .unwrap()
        .into_iter()
        .filter_map(|o| o.result.ok().map(|a| (o.fixture, a)))
        .collect()
}

fn record_of(id: &str) -> pvec_corpus::ExpectedMath {
    let b = bundle();
    let r = FixtureRegistry::load(
        &b,
        &pvec_capabilities::Registry::v1(),
        include_str!("../../../docs/ISO_PROFILE.md"),
    )
    .unwrap();
    r.get(id).unwrap().expected.clone().unwrap()
}

fn ulps(a: f64, b: f64) -> u64 {
    if a == b {
        return 0;
    }
    a.to_bits().abs_diff(b.to_bits())
}

// ------------------------------------------------------------------ the arithmetic

#[test]
fn atan_is_within_one_ulp_of_the_c_library_over_every_branch() {
    // every branch of the reduction: tiny, below 7/16, 7/16..11/16, 11/16..19/16, 19/16..39/16, beyond
    let mut xs = vec![
        f64::MIN_POSITIVE * 4.0,
        0.000_000_000_1,
        0.1,
        0.4,
        0.4375,
        0.5,
        0.6875,
        0.9,
        1.0,
        1.1875,
        1.5,
        2.0,
        2.4375,
        3.0,
        10.0,
        100_000.0,
        100_000_000_000_000_000_000.0,
    ];
    let mut v = 0.001;
    while v < 1_000_000.0 {
        xs.push(v);
        v *= 1.37;
    }
    for x in xs {
        for s in [x, -x] {
            assert!(ulps(atan(s), s.atan()) <= 1, "atan({s}) = {} against {}", atan(s), s.atan());
        }
    }
    assert_eq!(atan(0.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(atan(-0.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(atan(f64::INFINITY), FRAC_PI_2);
    assert_eq!(atan(f64::NEG_INFINITY), -FRAC_PI_2);
}

#[test]
fn atan2_is_within_one_ulp_of_the_c_library_in_every_quadrant() {
    let vs = [0.28, 0.96, 1.0, 1.75, 3.5, 0.0003, 120.0, 0.000_01, 7_000_000_000.0];
    for y in vs {
        for x in vs {
            for (sy, sx) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
                let (yy, xx) = (sy * y, sx * x);
                assert!(ulps(atan2(yy, xx), yy.atan2(xx)) <= 1, "atan2({yy}, {xx})");
            }
        }
    }
}

#[test]
fn atan2_on_the_axes_is_exact_and_keeps_the_sign_of_zero() {
    assert_eq!(atan2(0.0, 1.0).to_bits(), 0.0f64.to_bits());
    assert_eq!(atan2(-0.0, 1.0).to_bits(), (-0.0f64).to_bits());
    assert_eq!(atan2(0.0, -1.0), PI);
    assert_eq!(atan2(-0.0, -1.0), -PI);
    assert_eq!(atan2(1.0, 0.0), FRAC_PI_2);
    assert_eq!(atan2(-1.0, 0.0), -FRAC_PI_2);
    assert_eq!(atan2(1.0, 1.0), PI / 4.0);
    assert!(atan2(f64::NAN, 1.0).is_nan());
}

#[test]
fn the_arithmetic_is_pinned_bit_for_bit_so_every_machine_gets_the_same_angle() {
    // values computed once and committed: the same bits on x64 and arm64 is the point of the module
    assert_eq!(atan2(0.28, 0.96).to_bits(), 0x3fd2_29ae_c476_38dd_u64);
    assert_eq!(atan(0.4).to_bits(), 0x3fd8_5a37_6b67_7dc0_u64);
}

// ------------------------------------------------------------------ the fixtures

/// The two vertices and the bulge the record authors for a bulge fixture.
fn bulge_of(id: &str) -> ([f64; 2], [f64; 2], f64) {
    let rec = record_of(id);
    let g = &rec.entities[0].geometry;
    let v = g.get("vertices").unwrap().as_list().unwrap();
    let pt = |n: &pvec_corpus::expected::Node| {
        let l = n.as_list().unwrap();
        [l[0].as_scalar().unwrap().value(), l[1].as_scalar().unwrap().value()]
    };
    let b = g.get("bulges").unwrap().as_list().unwrap()[0].as_scalar().unwrap().value();
    (pt(&v[0]), pt(&v[1]), b)
}

#[test]
fn the_five_fixtures_that_were_refused_for_an_angle_or_a_periodic_knot_vector_are_generated() {
    let got = all();
    for id in
        ["bulge-arc-001", "bulge-arc-neg-001", "ellipse-001", "elliptical-arc-001", "closed-periodic-001"]
    {
        assert!(got.contains_key(id), "{id} is still refused");
    }
}

/// A bulge fixture holds two entities: the polyline with its bulge, and the equivalent arc the
/// corpus authored. The first is derived and the second is stored as authored.
fn circle_arc(a: &Artifact, index: usize) -> ([f64; 2], f64, [f64; 2]) {
    let o = read(&a.bytes).unwrap();
    assert_eq!(o.entities.len(), 2);
    let ReadEntity::CircleArc { centre, radius, start_sweep, .. } = &o.entities[index] else {
        panic!("a circle arc")
    };
    (*centre, *radius, *start_sweep)
}

#[test]
fn a_bulge_is_stored_as_the_arc_between_its_vertices_with_the_sweep_four_atan_of_the_bulge() {
    let got = all();
    for id in ["bulge-arc-001", "bulge-arc-neg-001"] {
        let (c, r, [start, sweep]) = circle_arc(&got[id], 0);
        let (p0, p1, bulge) = bulge_of(id);
        assert_eq!(sweep.is_sign_positive(), bulge > 0.0, "{id}: counter-clockwise is a positive sweep");
        assert!(ulps(sweep.abs(), 4.0 * atan(bulge.abs())) <= 1, "{id}: sweep {sweep}");
        assert!((0.0..TAU).contains(&start), "{id}: start {start}");
        let at = |t: f64| [c[0] + r * t.cos(), c[1] + r * t.sin()];
        for (got_pt, want) in [(at(start), p0), (at(start + sweep), p1)] {
            let d = (got_pt[0] - want[0]).hypot(got_pt[1] - want[1]);
            assert!(d < ABS, "{id}: the arc ends {d} from its vertex");
        }
        let chord = (p1[0] - p0[0]).hypot(p1[1] - p0[1]);
        let want_r = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
        assert!(((r - want_r) / want_r).abs() < REL, "{id}: radius {r} against {want_r}");
    }
}

#[test]
fn the_derived_arc_is_the_arc_the_corpus_authored_beside_it_to_the_last_few_bits() {
    // The record authors the equivalent arc itself. It is a check on the derivation that does not go
    // through the C library: the derived arc has to be the same locus within its stated bound.
    let got = all();
    for id in ["bulge-arc-001", "bulge-arc-neg-001"] {
        let (c, r, [start, sweep]) = circle_arc(&got[id], 0);
        let (ac, ar, [astart, asweep]) = circle_arc(&got[id], 1);
        let rec = record_of(id);
        let authored = &rec.entities[1].geometry;
        let real = |k: &str| authored.get(k).unwrap().as_scalar().unwrap().value();
        assert_eq!(ar.to_bits(), real("radius").to_bits(), "{id}: the authored arc is stored as authored");
        assert_eq!(astart.to_bits(), real("start_angle").to_bits());
        assert_eq!(asweep.to_bits(), real("sweep_angle").to_bits());
        let bound: f64 = got[id].derived.iter().map(|d| d.bound).fold(0.0, f64::max);
        assert!(bound > 0.0 && bound < ABS, "{id}: bound {bound}");
        assert!((c[0] - ac[0]).abs() <= bound && (c[1] - ac[1]).abs() <= bound, "{id}: centre");
        assert!((r - ar).abs() <= bound, "{id}: radius {r} against {ar}");
        assert!((sweep.abs() - asweep).abs() <= bound, "{id}: sweep {sweep} against {asweep}");
        // a clockwise arc starts where the authored counter-clockwise one ends
        let lo = if sweep < 0.0 { start + sweep } else { start }.rem_euclid(TAU);
        assert!((lo - astart).abs() <= bound, "{id}: start {lo} against {astart}");
    }
}

#[test]
fn a_bulge_arc_agrees_with_the_semantic_models_own_constructor() {
    let got = all();
    for id in ["bulge-arc-001", "bulge-arc-neg-001"] {
        let (c, r, [start, sweep]) = circle_arc(&got[id], 0);
        let (p0, p1, bulge) = bulge_of(id);
        let t = Trimmed::from_bulge(Point2::new(p0[0], p0[1]), Point2::new(p1[0], p1[1]), bulge).unwrap();
        let Curve::Circle(circle) = t.basis() else { panic!("a circle basis") };
        let o = circle.placement().origin();
        assert!((o.x - c[0]).abs() < ABS && (o.y - c[1]).abs() < ABS, "{id}: centre");
        assert!((circle.radius() - r).abs() < ABS, "{id}: radius");
        let trim = t.trim();
        let (s, w) =
            if trim.reversed { (trim.start + trim.span, -trim.span) } else { (trim.start, trim.span) };
        assert!((s - start).rem_euclid(TAU).min(TAU - (s - start).rem_euclid(TAU)) < ABS, "{id}: start");
        assert!((w - sweep).abs() < ABS, "{id}: sweep");
    }
}

#[test]
fn a_rotated_ellipse_keeps_its_authored_centre_and_radii_and_gets_the_angle_of_its_major_axis() {
    let got = all();
    let o = read(&got["ellipse-001"].bytes).unwrap();
    let ReadEntity::EllipseArc { centre, radii, angles, .. } = &o.entities[0] else { panic!("an ellipse") };
    assert_eq!((centre[0].to_bits(), centre[1].to_bits()), (2.0f64.to_bits(), 1.0f64.to_bits()));
    assert_eq!((radii[0].to_bits(), radii[1].to_bits()), (3.0f64.to_bits(), 2.94f64.to_bits()));
    assert_eq!(angles[0].to_bits(), atan2(0.28, 0.96).to_bits(), "rotation");
    assert!(ulps(angles[0], 0.28f64.atan2(0.96)) <= 1);
    assert_eq!((angles[1].to_bits(), angles[2].to_bits()), (0.0f64.to_bits(), TAU.to_bits()));
}

#[test]
fn a_rotated_elliptical_arc_keeps_the_authored_start_and_sweep_bit_for_bit() {
    let got = all();
    let o = read(&got["elliptical-arc-001"].bytes).unwrap();
    let ReadEntity::EllipseArc { centre, radii, angles, .. } = &o.entities[0] else {
        panic!("an ellipse arc")
    };
    assert_eq!((centre[0].to_bits(), centre[1].to_bits()), (2.0f64.to_bits(), 1.0f64.to_bits()));
    assert_eq!((radii[0].to_bits(), radii[1].to_bits()), (3.0f64.to_bits(), 2.94f64.to_bits()));
    assert_eq!(angles[0].to_bits(), atan2(0.28, 0.96).to_bits());
    assert_eq!((angles[1].to_bits(), angles[2].to_bits()), (0.3f64.to_bits(), 2.2f64.to_bits()));
}

#[test]
fn a_periodic_spline_is_stored_as_the_clamped_closed_unwrap_and_says_how_far_it_may_deviate() {
    let got = all();
    let a = &got["closed-periodic-001"];
    let o = read(&a.bytes).unwrap();
    let ReadEntity::Spline { degree, closed, knots, x, y, weights, .. } = &o.entities[0] else {
        panic!("a spline")
    };
    assert_eq!((*degree, *closed, weights.is_none()), (3, true, true));
    // the same numbers the model's own unwrap gives
    let rec = record_of("closed-periodic-001");
    let g = &rec.entities[0].geometry;
    let cps: Vec<Point2> = g
        .get("control_points")
        .unwrap()
        .as_list()
        .unwrap()
        .iter()
        .map(|p| {
            let l = p.as_list().unwrap();
            Point2::new(l[0].as_scalar().unwrap().value(), l[1].as_scalar().unwrap().value())
        })
        .collect();
    let ks: Vec<f64> = g
        .get("knots")
        .unwrap()
        .as_list()
        .unwrap()
        .iter()
        .map(|k| k.get("value").unwrap().as_scalar().unwrap().value())
        .collect();
    let u = BSpline::from_periodic(3, cps, ks).unwrap();
    let want: Vec<u64> = u.curve.knots().iter().map(|k| k.to_bits()).collect();
    assert_eq!(knots.iter().map(|k| k.to_bits()).collect::<Vec<_>>(), want);
    assert_eq!(x.len(), u.curve.control_points().len());
    assert_eq!(x[0].to_bits(), u.curve.control_points()[0].x.to_bits());
    assert_eq!(y[0].to_bits(), u.curve.control_points()[0].y.to_bits());
    // the artifact carries the bound, not just the numbers
    let d = a.derived.iter().find(|d| d.entity == rec.entities[0].id).expect("the derivation is recorded");
    assert_eq!(d.bound.to_bits(), u.periodic_deviation_bound.to_bits());
    assert!(d.bound.is_finite() && d.bound > 0.0);
    assert!(d.what.contains("periodic"), "{}", d.what);
}

#[test]
fn every_number_that_is_derived_is_listed_and_an_authored_one_never_is() {
    let got = all();
    for id in ["line-001", "circle-001", "bspline-d3-001", "polyline-001"] {
        assert!(got[id].derived.is_empty(), "{id}: nothing is derived");
    }
    for id in
        ["bulge-arc-001", "bulge-arc-neg-001", "ellipse-001", "elliptical-arc-001", "closed-periodic-001"]
    {
        assert!(!got[id].derived.is_empty(), "{id}: its derived numbers are said");
        for d in &got[id].derived {
            assert!(d.bound.is_finite() && d.bound >= 0.0, "{id}: {d:?}");
        }
    }
}

#[test]
fn the_only_refusals_left_are_the_deliberate_one_and_the_quarantined() {
    let outcomes = generate_all(&bundle()).unwrap();
    let refused: BTreeMap<String, Cause> =
        outcomes.into_iter().filter_map(|o| o.result.err().map(|r| (o.fixture, r.cause))).collect();
    assert!(!refused.contains_key("layer-state-001"), "layer state travels in the layer stream (#119)");
    assert_eq!(refused["near-singular-001"], Cause::NotEmitted);
    assert_eq!(
        refused.len(),
        1 + 3,
        "the deliberate refusal and the three quarantined: the seven instance fixtures are emitted (#200)"
    );
}

#[test]
fn what_is_still_outside_the_drawing_plane_or_the_axis_is_still_refused_for_a_synthetic_record() {
    let o = [0.0, 0.0, 0.0];
    let x = [1.0, 0.0, 0.0];
    // an ellipse in a tilted plane, and one with a major axis that has no direction
    let r = generate_record(&record("r", &[ellipse("tilted", o, [0.0, 1.0, 0.0], 3.0, 2.0)]), REVISION);
    assert!(r.is_ok(), "a major axis along y is an in-plane rotation: {r:?}");
    let r = generate_record(&record("r", &[ellipse("up", o, [0.0, 0.0, 1.0], 3.0, 2.0)]), REVISION);
    assert!(r.is_err(), "a major axis out of the plane is not a plane ellipse");
    let r = generate_record(&record("r", &[ellipse("none", o, [0.0, 0.0, 0.0], 3.0, 2.0)]), REVISION);
    assert!(r.is_err(), "a zero major axis has no direction");
    let r = generate_record(
        &record("r", &[circle("turned", o, [0.0, 0.0, 1.0], [0.0, 1.0, 0.0], 1.0)]),
        REVISION,
    );
    assert!(r.is_err(), "a rotated circle frame is still refused");
    let _ = x;
}

use pvec_testkit::generate::generate_record;
