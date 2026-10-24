//! Issue #94: a trim of a circle or ellipse that covers the whole period is canonicalised inside PVec.
//!
//! A DWG `ELLIPSE` always carries a start and an end parameter, and a whole ellipse is the pair
//! 0 and 2 pi. Which of `ellipse` and `trimmed_curve over ellipse` that is must be decided by NP-11,
//! not by a copy of the test in a reader adapter (N1.2 ADR, NG-10). The decision is recorded in the
//! amendment to ADR 0002: only the identity parameterisation (start at the origin of the period, running
//! forward) is the carrier itself; a whole-period trim that starts elsewhere or runs backwards keeps its
//! start and direction, because dropping them would change what `eval` returns.
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::{FRAC_PI_2, PI, TAU};
use pvec_geometry::*;

fn circle() -> Curve {
    Curve::Circle(Circle::new(place(1.0, 2.0, 1.0, 0.0), 3.0).unwrap())
}

fn ellipse() -> Curve {
    Curve::Ellipse(Ellipse::new(place(-4.0, 0.5, 3.0, 4.0), 5.0, 2.0).unwrap())
}

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

fn trimmed(c: &Curve) -> &Trimmed {
    match c {
        Curve::Trimmed(t) => t,
        other => panic!("expected a trimmed curve, got {:?}", other.kind()),
    }
}

#[test]
fn zero_to_two_pi_is_the_untrimmed_carrier() {
    let (c, e) = (circle(), ellipse());
    assert_eq!(Curve::trimmed_conic(c.clone(), 0.0, TAU, true).unwrap(), c);
    assert_eq!(Curve::trimmed_conic(e.clone(), 0.0, TAU, true).unwrap(), e);
    assert_eq!(Curve::trimmed_conic(e, 0.0, TAU, true).unwrap().kind(), CurveKind::Ellipse);
}

#[test]
fn a_whole_period_written_with_other_whole_turns_is_still_the_carrier() {
    let c = circle();
    for (t1, t2) in [(-TAU, 0.0), (TAU, 2.0 * TAU), (-TAU, 0.0)] {
        assert_eq!(Curve::trimmed_conic(c.clone(), t1, t2, true).unwrap(), c, "{t1} {t2}");
    }
}

#[test]
fn the_decision_uses_the_np11_tolerance_for_the_end_and_for_the_start() {
    let e = ellipse();
    // NP-11 tp for a period of 2 pi is about 6.3e-10 (REL * 2 pi): inside it is the carrier ...
    assert_eq!(Curve::trimmed_conic(e.clone(), 0.0, TAU - 0.0000000001, true).unwrap(), e);
    assert_eq!(Curve::trimmed_conic(e.clone(), 0.0000000001, TAU, true).unwrap(), e);
    assert_eq!(Curve::trimmed_conic(e.clone(), -0.0000000001, TAU - 0.0000000001, true).unwrap(), e);
    // ... and outside it is an arc, a hair short of a full turn.
    let short = Curve::trimmed_conic(e.clone(), 0.0, TAU - 0.000000002, true).unwrap();
    assert!(trimmed(&short).trim().span < TAU);
    assert!(!trimmed(&short).is_full_period());
}

#[test]
fn a_whole_period_that_starts_elsewhere_keeps_its_start() {
    let c = circle();
    let out = Curve::trimmed_conic(c.clone(), 1.0, 1.0 + TAU, true).unwrap();
    let t = trimmed(&out);
    assert_eq!(t.trim().start, 1.0);
    assert!(t.is_full_period());
    // The point that tau = 0 names is still the point at basis parameter 1, not the one at 0.
    assert_eq!(t.eval(0.0).unwrap(), c.eval(1.0).unwrap());
    let started_at_pi = Curve::trimmed_conic(c, PI, PI + TAU, true).unwrap();
    assert_eq!(trimmed(&started_at_pi).trim().start, PI);
}

#[test]
fn a_whole_period_that_runs_backwards_keeps_its_direction() {
    let e = ellipse();
    let out = Curve::trimmed_conic(e.clone(), TAU, 0.0, false).unwrap();
    let t = trimmed(&out);
    assert!(t.trim().reversed);
    assert!(t.is_full_period());
    assert_eq!(t.eval(FRAC_PI_2).unwrap(), e.eval(TAU - FRAC_PI_2).unwrap());
}

#[test]
fn equal_ends_are_an_empty_arc_and_a_negative_full_turn_is_ambiguous_as_before() {
    for basis in [circle(), ellipse()] {
        assert_eq!(rule_of(Curve::trimmed_conic(basis.clone(), 1.0, 1.0, true)), "NP-12");
        assert_eq!(rule_of(Curve::trimmed_conic(basis.clone(), 0.0, TAU, false)), "NP-11");
        assert_eq!(rule_of(Curve::trimmed_conic(basis.clone(), 0.0, -TAU, true)), "NP-11");
        // More than one period is refused under NP-11 rule 1.
        assert_eq!(rule_of(Curve::trimmed_conic(basis, 0.0, TAU + 0.1, true)), "NP-11");
    }
}

#[test]
fn a_partial_arc_is_exactly_what_trimmed_new_builds() {
    for basis in [circle(), ellipse()] {
        for (t1, t2, sense) in [(0.0, PI, true), (1.0, 5.0, true), (5.0, 1.0, false), (-1.0, 1.0, true)] {
            let want = Curve::Trimmed(Trimmed::new(basis.clone(), t1, t2, sense).unwrap());
            assert_eq!(Curve::trimmed_conic(basis.clone(), t1, t2, sense).unwrap(), want);
        }
    }
}

#[test]
fn the_canonical_curve_is_the_same_curve_as_the_full_trim() {
    let (c, e) = (circle(), ellipse());
    for basis in [c, e] {
        let full = Trimmed::new(basis.clone(), 0.0, TAU, true).unwrap();
        let canon = Curve::trimmed_conic(basis, 0.0, TAU, true).unwrap();
        assert_eq!(canon.domain().lo(), 0.0);
        assert_eq!(canon.domain().hi(), full.domain().hi());
        for k in 0..=32 {
            let tau = TAU * f64::from(k) / 32.0;
            assert_eq!(canon.eval(tau).unwrap(), full.eval(tau).unwrap(), "tau = {tau}");
        }
        assert_eq!(canon.bounds().unwrap(), full.bounds().unwrap());
    }
}

#[test]
fn only_a_circle_or_an_ellipse_is_a_conic_basis() {
    let line = Curve::Line(Line::new(pt(0.0, 0.0), v(1.0, 0.0)).unwrap());
    let bez = Curve::Bezier(Bezier::new(vec![pt(0., 0.), pt(1., 1.), pt(2., 0.)]).unwrap());
    let trimmed_circle = Curve::Trimmed(Trimmed::new(circle(), 0.0, 1.0, true).unwrap());
    for basis in [line, bez, trimmed_circle] {
        assert_eq!(rule_of(Curve::trimmed_conic(basis, 0.0, TAU, true)), "PROFILE-8");
    }
}

#[test]
fn trimmed_new_is_unchanged_and_still_stores_a_full_period_trim() {
    // A decoder rebuilds with Trimmed::new / from_normalised and must not see a different kind.
    let t = Trimmed::new(circle(), 0.0, TAU, true).unwrap();
    assert_eq!(t.trim().span, TAU);
    assert_eq!(t.kind(), CurveKind::TrimmedCurve);
}

#[test]
fn is_full_period_applies_the_np11_tolerance_to_the_stored_span() {
    assert!(Trimmed::new(circle(), 0.0, TAU, true).unwrap().is_full_period());
    assert!(Trimmed::new(ellipse(), 1.0, 1.0 + TAU, true).unwrap().is_full_period());
    assert!(Trimmed::new(ellipse(), TAU, 0.0, false).unwrap().is_full_period());
    assert!(!Trimmed::new(circle(), 0.0, PI, true).unwrap().is_full_period());
    let near_full = Trim { start: 0.0, span: TAU - 0.0000000001, reversed: false };
    assert!(Trimmed::from_normalised(circle(), near_full).unwrap().is_full_period());
    let short = Trim { start: 0.0, span: TAU - 0.000000002, reversed: false };
    assert!(!Trimmed::from_normalised(circle(), short).unwrap().is_full_period());
}

#[test]
fn a_trimmed_line_or_spline_is_never_a_full_period() {
    let line = Curve::Line(Line::new(pt(0.0, 0.0), v(1.0, 0.0)).unwrap());
    assert!(!Trimmed::new(line, 0.0, TAU, true).unwrap().is_full_period());
    let bez = Curve::Bezier(Bezier::new(vec![pt(0., 0.), pt(1., 1.), pt(2., 0.)]).unwrap());
    assert!(!Trimmed::new(bez, 0.0, 1.0, true).unwrap().is_full_period());
}
