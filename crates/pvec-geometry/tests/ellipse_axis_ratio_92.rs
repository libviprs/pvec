//! Issue #92: an ellipse built from a centre, a major-axis vector and a minor/major ratio, the three
//! numbers a DWG `ELLIPSE` stores. The one product `ratio * |vector|` belongs to PVec, so a reader
//! adapter does no arithmetic on source values (NG-01, NG-B1).
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_geometry::*;

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

#[test]
fn the_semi_axes_are_the_vector_length_and_the_length_times_the_ratio() {
    let e = Ellipse::from_major_axis(pt(1.0, 2.0), v(3.0, 4.0), 0.5).unwrap();
    assert_eq!(e.semi_axis_1(), 5.0);
    assert_eq!(e.semi_axis_2(), 2.5);
}

#[test]
fn the_frame_is_the_vector_normalised_at_the_centre() {
    let e = Ellipse::from_major_axis(pt(1.0, 2.0), v(3.0, 4.0), 0.5).unwrap();
    assert_eq!(e.placement().origin(), pt(1.0, 2.0));
    assert_eq!(e.placement().x_axis(), v(0.6, 0.8));
}

#[test]
fn it_is_bit_identical_to_building_the_same_ellipse_by_hand() {
    // The harness measurement in the issue: PVec's own `ellipse` from the worked-out numbers.
    for (c, axis, ratio) in [
        (pt(0.0, 0.0), v(2.0, 0.0), 0.25),
        (pt(-7.5, 3.25), v(-1.5, 6.0), 0.3),
        (pt(1000000.0, -1000000.0), v(123.456, -789.012), 0.999),
        (pt(0.0, 0.0), v(0.0, 40.0), 1.0 / 3.0),
    ] {
        let a = axis.length();
        let by_hand = Ellipse::new(Placement2::new(c, axis).unwrap(), a, ratio * a).unwrap();
        assert_eq!(Ellipse::from_major_axis(c, axis, ratio).unwrap(), by_hand);
    }
}

#[test]
fn the_major_axis_stays_first_and_the_curve_runs_through_the_vector_end() {
    let e = Ellipse::from_major_axis(pt(10.0, -4.0), v(0.0, -6.0), 0.5).unwrap();
    assert!(e.semi_axis_1() >= e.semi_axis_2());
    let end = e.eval(0.0).unwrap();
    assert!(near(end, pt(10.0, -10.0), 6.0), "t = 0 is the end of the major-axis vector, got {end:?}");
    let minor = e.eval(core::f64::consts::FRAC_PI_2).unwrap();
    // The second axis is the first rotated by +90 degrees: (0, -1) becomes (1, 0).
    assert!(near(minor, pt(13.0, -4.0), 6.0), "t = pi/2 is the minor vertex, got {minor:?}");
}

#[test]
fn a_ratio_of_one_is_an_ellipse_and_is_not_converted_to_a_circle() {
    let e = Ellipse::from_major_axis(pt(0.0, 0.0), v(2.0, 0.0), 1.0).unwrap();
    assert_eq!(e.kind(), CurveKind::Ellipse);
    assert_eq!(e.semi_axis_1(), e.semi_axis_2());
}

#[test]
fn a_zero_vector_is_refused_under_np10() {
    assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(0.0, 0.0), 0.5)), "NP-10");
    assert_eq!(
        rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(0.0000000000001, -0.0000000000001), 0.5)),
        "NP-10"
    );
}

#[test]
fn a_ratio_outside_zero_to_one_is_refused_under_np07() {
    for ratio in [0.0, -0.0, -0.5, f64::from_bits(1.0f64.to_bits() + 1), 2.0, 1000000000.0] {
        assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(3.0, 4.0), ratio)), "NP-07", "{ratio}");
    }
}

#[test]
fn a_ratio_past_the_conditioning_bound_is_refused_under_np07() {
    assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(3.0, 4.0), 0.0000001)), "NP-07");
    // Well inside the bound of 1e6 is fine.
    assert!(Ellipse::from_major_axis(pt(0.0, 0.0), v(3.0, 4.0), 0.000002).is_ok());
}

#[test]
fn a_non_finite_input_is_refused_under_np01() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(3.0, 4.0), bad)), "NP-01");
        assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(bad, 4.0), 0.5)), "NP-01");
        assert_eq!(rule_of(Ellipse::from_major_axis(pt(bad, 0.0), v(3.0, 4.0), 0.5)), "NP-01");
    }
}

#[test]
fn a_ratio_that_makes_the_minor_axis_subnormal_or_a_vector_too_long_is_refused() {
    assert_eq!(rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(3.0, 4.0), f64::from_bits(1))), "NP-01");
    assert_eq!(
        rule_of(Ellipse::from_major_axis(pt(0.0, 0.0), v(f64::MAX / 4.0, f64::MAX / 4.0), 0.5)),
        "NP-02"
    );
}

#[test]
fn a_huge_vector_does_not_overflow_the_length() {
    // The length is a hypot, so a 3-4-5 triangle scaled to hundreds of billions is fine (2^40 is about 1.1 trillion).
    let e = Ellipse::from_major_axis(pt(0.0, 0.0), v(300000000000.0, 400000000000.0), 0.5).unwrap();
    assert_eq!(e.semi_axis_1(), 500000000000.0);
}
