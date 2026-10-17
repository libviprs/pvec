//! Construction-time validation (issue P3.1): every refusal is typed and names its rule, and no
//! refused input can be unwrapped into a curve.
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_geometry::*;
use pvec_numeric::{
    MAX_ABS_COORD, MAX_COMPOSITE_SEGMENTS, MAX_CONTROL_POINTS, MAX_DEGREE, MAX_POLYLINE_VERTICES,
};

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

#[test]
fn a_non_finite_coordinate_is_refused_by_every_constructor_with_np01() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(rule_of(Point2::checked(bad, 0.0)), "NP-01");
        assert_eq!(rule_of(Placement2::new(pt(bad, 0.0), v(1.0, 0.0))), "NP-01");
        assert_eq!(rule_of(Placement2::new(pt(0.0, 0.0), v(bad, 0.0))), "NP-01");
        assert_eq!(rule_of(Line::new(pt(0.0, bad), v(1.0, 0.0))), "NP-01");
        assert_eq!(rule_of(Circle::new(place(0., 0., 1., 0.), bad)), "NP-01");
        assert_eq!(rule_of(Ellipse::new(place(0., 0., 1., 0.), 2.0, bad)), "NP-01");
        assert_eq!(rule_of(Polyline::new(vec![pt(0., 0.), pt(bad, 1.)])), "NP-01");
        assert_eq!(rule_of(Bezier::new(vec![pt(0., 0.), pt(1., bad)])), "NP-01");
        assert_eq!(rule_of(BSpline::new(1, vec![pt(0., 0.), pt(bad, 1.)], vec![0., 0., 1., 1.])), "NP-01");
        assert_eq!(rule_of(BSpline::new(1, vec![pt(0., 0.), pt(1., 1.)], vec![0., bad, 1., 1.])), "NP-01");
        assert_eq!(
            rule_of(RationalBSpline::new(
                1,
                vec![pt(0., 0.), pt(1., 1.)],
                vec![0., 0., 1., 1.],
                vec![1., bad]
            )),
            "NP-01"
        );
        assert_eq!(
            rule_of(Trimmed::new(
                Curve::Circle(Circle::new(place(0., 0., 1., 0.), 1.0).unwrap()),
                0.0,
                bad,
                true
            )),
            "NP-01"
        );
        assert_eq!(rule_of(Transform2::new([1., 0., 0., 1., bad, 0.])), "NP-01");
    }
}

#[test]
fn subnormal_and_out_of_range_values_are_refused() {
    let tiny = f64::MIN_POSITIVE / 2.0;
    assert_eq!(rule_of(Point2::checked(tiny, 0.0)), "NP-01");
    assert_eq!(rule_of(Point2::checked(MAX_ABS_COORD * 2.0, 0.0)), "NP-02");
    assert_eq!(rule_of(Point2::checked(0.0, -MAX_ABS_COORD * 2.0)), "NP-02");
    assert_eq!(rule_of(Line::new(pt(MAX_ABS_COORD * 2.0, 0.0), v(1.0, 0.0))), "NP-02");
    assert!(Point2::checked(MAX_ABS_COORD, -MAX_ABS_COORD).is_ok());
}

#[test]
fn negative_zero_is_stored_as_positive_zero() {
    let p = Point2::checked(-0.0, -0.0).unwrap();
    assert_eq!(p.x.to_bits(), 0.0f64.to_bits());
    assert_eq!(p.y.to_bits(), 0.0f64.to_bits());
    let l = Line::new(pt(-0.0, 1.0), v(1.0, 0.0)).unwrap();
    assert_eq!(l.origin().x.to_bits(), 0.0f64.to_bits());
}

#[test]
fn a_non_monotonic_knot_vector_is_refused_with_np14() {
    let cps = vec![pt(0., 0.), pt(1., 1.), pt(2., 0.)];
    assert_eq!(rule_of(BSpline::new(2, cps.clone(), vec![0., 0., 0., 1., 0.5, 1.])), "NP-14");
    // wrong length: degree + control points + 1 = 6 entries are required
    assert_eq!(rule_of(BSpline::new(2, cps.clone(), vec![0., 0., 0., 1., 1.])), "NP-14");
    assert_eq!(rule_of(BSpline::new(2, cps.clone(), vec![0., 0., 0., 1., 1., 1., 1.])), "NP-14");
    // two distinct knots closer than the parameter tolerance are ambiguous
    let near_pair = 1.0 + P.tol(1.0, 1.0) / 4.0;
    assert_eq!(rule_of(BSpline::new(2, cps.clone(), vec![0., 0., 0., 1., near_pair, near_pair])), "NP-14");
    // an interior knot of multiplicity degree + 1 would make the curve discontinuous
    let quad = vec![pt(0., 0.), pt(1., 1.), pt(2., 0.), pt(3., 1.)];
    assert_eq!(rule_of(BSpline::new(1, quad, vec![0., 0., 0.5, 0.5, 1., 1.])), "NP-14");
}

#[test]
fn counts_above_the_maxima_are_refused_with_np18() {
    let line = |n: usize| (0..n).map(|i| pt(i as f64, (i % 7) as f64)).collect::<Vec<_>>();
    assert_eq!(rule_of(Polyline::new(line(MAX_POLYLINE_VERTICES + 1))), "NP-18");
    assert_eq!(rule_of(Polyline::new(vec![pt(0., 0.)])), "NP-18");
    assert_eq!(rule_of(Polyline::new(Vec::new())), "NP-18");
    assert_eq!(rule_of(Bezier::new(line(MAX_DEGREE + 2))), "NP-18");
    assert_eq!(rule_of(Bezier::new(vec![pt(0., 0.)])), "NP-18");

    // degree above the maximum
    let d = MAX_DEGREE + 1;
    let knots: Vec<f64> = (0..2 * (d + 1)).map(|i| if i <= d { 0.0 } else { 1.0 }).collect();
    assert_eq!(rule_of(BSpline::new(d, line(d + 1), knots)), "NP-18");
    assert_eq!(rule_of(BSpline::new(0, line(2), vec![0., 1.])), "NP-18");

    // control points above the maximum: the count is checked before the knot vector is read
    let n = MAX_CONTROL_POINTS + 1;
    assert_eq!(rule_of(BSpline::new(1, line(n), vec![0.0, 1.0])), "NP-18");
    // too few control points for the degree
    assert_eq!(rule_of(BSpline::new(3, line(3), vec![0.; 7])), "NP-18");

    // composite segments above the maximum
    let seg = Curve::Polyline(Polyline::new(vec![pt(0., 0.), pt(1., 0.)]).unwrap());
    let many: Vec<Curve> = (0..=MAX_COMPOSITE_SEGMENTS).map(|_| seg.clone()).collect();
    assert_eq!(rule_of(Composite::new(many)), "NP-18");
    assert_eq!(rule_of(Composite::new(Vec::new())), "NP-18");
}

#[test]
fn degenerate_shapes_are_refused_with_the_rule_that_defines_them() {
    // NP-07: radius degenerate, and an ellipse past the conditioning bound
    assert_eq!(rule_of(Circle::new(place(0., 0., 1., 0.), P.abs * 0.5)), "NP-07");
    assert_eq!(rule_of(Circle::new(place(0., 0., 1., 0.), 0.0)), "NP-07");
    assert_eq!(rule_of(Circle::new(place(0., 0., 1., 0.), -1.0)), "NP-07");
    assert_eq!(rule_of(Ellipse::new(place(0., 0., 1., 0.), P.axis_ratio_max * 2.0, 1.0)), "NP-07");
    // NP-06: zero-length polyline segment
    assert_eq!(rule_of(Polyline::new(vec![pt(1., 1.), pt(1., 1.), pt(2., 2.)])), "NP-06");
    // NP-10: zero direction
    assert_eq!(rule_of(Line::new(pt(0., 0.), v(0., 0.))), "NP-10");
    assert_eq!(rule_of(Placement2::new(pt(0., 0.), v(0., 0.))), "NP-10");
    // NP-08: all control points coincide
    assert_eq!(rule_of(Bezier::new(vec![pt(2., 2.), pt(2., 2.), pt(2., 2.)])), "NP-08");
    // NP-15: weights
    let w = |ws: Vec<f64>| {
        RationalBSpline::new(2, vec![pt(0., 0.), pt(1., 5.), pt(2., 0.)], vec![0., 0., 0., 1., 1., 1.], ws)
    };
    assert_eq!(rule_of(w(vec![1., 0.0, 1.])), "NP-15");
    assert_eq!(rule_of(w(vec![1., -1.0, 1.])), "NP-15");
    assert_eq!(rule_of(w(vec![1., P.weight_floor * 0.5, 1.])), "NP-15");
    assert_eq!(rule_of(w(vec![1., 1.])), "NP-15");
    assert!(w(vec![1., P.weight_floor * 2.0, 1.]).is_ok());
    // NP-19: singular transform
    assert_eq!(rule_of(Transform2::new([1., 2., 2., 4., 0., 0.])), "NP-19");
    assert_eq!(rule_of(Transform2::new([0.0; 6])), "NP-19");
}

#[test]
fn trims_follow_np11_and_np12() {
    let c = || Curve::Circle(Circle::new(place(0., 0., 1., 0.), 1.0).unwrap());
    assert_eq!(rule_of(Trimmed::new(c(), 1.0, 1.0, true)), "NP-12");
    assert_eq!(rule_of(Trimmed::new(c(), 0.0, 7.0, true)), "NP-11");
    let line = || Curve::Line(Line::new(pt(0., 0.), v(1., 0.)).unwrap());
    assert_eq!(rule_of(Trimmed::new(line(), 2.0, 1.0, true)), "NP-12");
    let spline = Curve::BSpline(BSpline::new(1, vec![pt(0., 0.), pt(1., 1.)], vec![0., 0., 1., 1.]).unwrap());
    assert_eq!(rule_of(Trimmed::new(spline, 0.5, 2.0, true)), "NP-12");
}

#[test]
fn only_the_profile_bases_can_be_trimmed_or_composed() {
    let poly = Curve::Polyline(Polyline::new(vec![pt(0., 0.), pt(1., 0.), pt(1., 1.)]).unwrap());
    let comp = Curve::Composite(Composite::new(vec![poly.clone()]).unwrap());
    assert_eq!(rule_of(Trimmed::new(poly, 0.0, 1.0, true)), "PROFILE-8");
    assert_eq!(rule_of(Trimmed::new(comp, 0.0, 1.0, true)), "PROFILE-8");
    let arc = Curve::Trimmed(
        Trimmed::new(Curve::Circle(Circle::new(place(0., 0., 1., 0.), 1.0).unwrap()), 0.0, 1.0, true)
            .unwrap(),
    );
    assert_eq!(rule_of(Trimmed::new(arc.clone(), 0.0, 0.5, true)), "PROFILE-8");
    // a bare circle, a line and a nested composite are not segments (profile section 9)
    let circle = Curve::Circle(Circle::new(place(0., 0., 1., 0.), 1.0).unwrap());
    assert_eq!(rule_of(Composite::new(vec![circle])), "PROFILE-9");
    let inner = Curve::Composite(Composite::new(vec![arc]).unwrap());
    assert_eq!(rule_of(Composite::new(vec![inner])), "PROFILE-9");
}

#[test]
fn composite_segments_must_join_np17_and_a_closed_claim_must_hold_np20() {
    let a = Curve::Polyline(Polyline::new(vec![pt(0., 0.), pt(1., 0.)]).unwrap());
    let gap = Curve::Polyline(Polyline::new(vec![pt(1.5, 0.), pt(2., 0.)]).unwrap());
    assert_eq!(rule_of(Composite::new(vec![a.clone(), gap])), "NP-17");
    let b = Curve::Polyline(Polyline::new(vec![pt(1., 0.), pt(1., 1.)]).unwrap());
    let open = Composite::new(vec![a.clone(), b]).unwrap();
    assert!(!open.is_declared_closed());
    assert_eq!(rule_of(open.declare_closed()), "NP-20");
    let loop_back = Curve::Polyline(Polyline::new(vec![pt(1., 1.), pt(0., 0.)]).unwrap());
    let closed = Composite::new(vec![
        a,
        Curve::Polyline(Polyline::new(vec![pt(1., 0.), pt(1., 1.)]).unwrap()),
        loop_back,
    ])
    .unwrap();
    assert!(closed.declare_closed().unwrap().is_declared_closed());
}

#[test]
fn trimmed_from_normalised_parts_validates_and_keeps_them_bit_exact() {
    let c = Curve::Circle(Circle::new(place(0., 0., 1., 0.), 2.0).unwrap());
    let t = Trimmed::new(c.clone(), 5.5, 1.0, true).unwrap();
    let parts = t.trim();
    let back = Trimmed::from_normalised(c.clone(), parts).unwrap();
    assert_eq!(back.trim().start.to_bits(), parts.start.to_bits());
    assert_eq!(back.trim().span.to_bits(), parts.span.to_bits());
    assert_eq!(back.trim().reversed, parts.reversed);
    let mut bad = parts;
    bad.start = 7.0;
    assert!(Trimmed::from_normalised(c.clone(), bad).is_err());
    bad = parts;
    bad.span = 0.0;
    assert!(Trimmed::from_normalised(c.clone(), bad).is_err());
    bad = parts;
    bad.span = f64::NAN;
    assert!(Trimmed::from_normalised(c, bad).is_err());
}

#[test]
fn error_messages_name_the_rule() {
    let e = Circle::new(place(0., 0., 1., 0.), 0.0).unwrap_err();
    assert!(e.to_string().contains("NP-07"), "{e}");
    let e = Trimmed::new(Curve::Polyline(Polyline::new(vec![pt(0., 0.), pt(1., 0.)]).unwrap()), 0., 1., true)
        .unwrap_err();
    assert!(e.to_string().contains("PROFILE-8"), "{e}");
}

#[test]
fn closed_flag_on_splines_is_checked_not_trusted() {
    let open =
        BSpline::new(2, vec![pt(0., 0.), pt(1., 1.), pt(2., 0.)], vec![0., 0., 0., 1., 1., 1.]).unwrap();
    assert_eq!(rule_of(open.declare_closed()), "NP-20");
    let ring = Bezier::new(vec![pt(0., 0.), pt(1., 2.), pt(2., 2.), pt(0., 0.)]).unwrap();
    assert!(ring.declare_closed().unwrap().is_declared_closed());
}

#[test]
fn bounds_can_be_built_from_two_corners_and_are_validated() {
    let b = Bounds::new(pt(0.0, 1.0), pt(2.0, 3.0)).unwrap();
    assert_eq!((b.min(), b.max()), (pt(0.0, 1.0), pt(2.0, 3.0)));
    // a degenerate box (a point, or a horizontal segment) is a legal bound
    assert!(Bounds::new(pt(1.0, 1.0), pt(1.0, 1.0)).is_ok());
    assert!(Bounds::new(pt(0.0, 1.0), pt(5.0, 1.0)).is_ok());
    // corners in the wrong order, and non-finite or out of range corners, are refused
    assert_eq!(rule_of(Bounds::new(pt(2.0, 0.0), pt(1.0, 3.0))), "GEOM-01");
    assert_eq!(rule_of(Bounds::new(pt(0.0, 0.0), pt(f64::NAN, 3.0))), "NP-01");
    assert_eq!(rule_of(Bounds::new(pt(0.0, 0.0), pt(MAX_ABS_COORD * 2.0, 3.0))), "NP-02");
}

#[test]
fn stored_axes_are_validated_not_renormalised_so_a_decoder_keeps_the_bits() {
    // Normalising an already unit vector can move its last bit; a decoder must not do that.
    let mut r = Rng(0x57_0ED);
    let mut moved = 0;
    for _ in 0..20_000 {
        let p = Placement2::new(pt(1.0, 2.0), v(r.range(-1.0, 1.0), r.range(-1.0, 1.0))).unwrap();
        let a = p.x_axis();
        let q = Placement2::from_stored(p.origin(), a).unwrap();
        assert_eq!((q.x_axis().x.to_bits(), q.x_axis().y.to_bits()), (a.x.to_bits(), a.y.to_bits()));
        let l = Line::from_stored(p.origin(), a).unwrap();
        assert_eq!((l.direction().x.to_bits(), l.direction().y.to_bits()), (a.x.to_bits(), a.y.to_bits()));
        let again = Placement2::new(p.origin(), a).unwrap();
        if again.x_axis().x.to_bits() != a.x.to_bits() || again.x_axis().y.to_bits() != a.y.to_bits() {
            moved += 1;
        }
    }
    assert!(moved > 0, "re-normalising must be shown to move bits for from_stored to be needed");
}

#[test]
fn stored_axes_that_are_not_unit_zero_or_not_finite_are_refused() {
    assert_eq!(rule_of(Placement2::from_stored(pt(0., 0.), v(2.0, 0.0))), "NP-10");
    assert_eq!(rule_of(Placement2::from_stored(pt(0., 0.), v(0.0, 0.0))), "NP-10");
    assert_eq!(rule_of(Placement2::from_stored(pt(0., 0.), v(f64::NAN, 0.0))), "NP-01");
    assert_eq!(rule_of(Placement2::from_stored(pt(f64::INFINITY, 0.), v(1.0, 0.0))), "NP-01");
    assert_eq!(rule_of(Line::from_stored(pt(0., 0.), v(0.6, 0.6))), "NP-10");
    assert_eq!(rule_of(Line::from_stored(pt(0., 0.), v(f64::NAN, 1.0))), "NP-01");
    // a unit vector within the NP-05 tolerance of length one is accepted as it is
    assert!(Line::from_stored(pt(0., 0.), v(0.6, 0.8)).is_ok());
}
