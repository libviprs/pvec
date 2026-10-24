//! The bulge-to-arc constructor (issue 91): a chord and a bulge become the trimmed circle of
//! profile section 8, with the sign convention of the DWG polyline (positive counter-clockwise from
//! the first vertex to the second, negative clockwise and held as a reversed trim).
//!
//! The expected numbers are the `expected.math.json` arcs of the r0-open probes `bulge-arc-001`
//! and `bulge-arc-neg-001` (libviprs/pvec-gold-fixtures, tag `corpus-r0-open`), copied here as
//! decimal literals that round trip to the stored f64 values.
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::{PI, TAU};
use pvec_geometry::*;

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

struct Expect {
    p0: Point2,
    p1: Point2,
    bulge: f64,
    centre: Point2,
    radius: f64,
    start: f64,
    sweep: f64,
    reversed: bool,
}

fn positive() -> Expect {
    Expect {
        p0: pt(1.0, 0.5),
        p1: pt(4.5, 2.25),
        bulge: 0.4,
        centre: pt(1.83125, 3.2125),
        radius: 2.837011246452858,
        start: 4.415023835160766,
        sweep: 1.5220255084494596,
        reversed: false,
    }
}

fn negative() -> Expect {
    Expect {
        p0: pt(3.0, -1.0),
        p1: pt(0.5, 2.5),
        bulge: -0.3,
        centre: pt(4.404166666666667, 2.6458333333333335),
        radius: 3.906889392115193,
        start: 3.1789285506168805,
        sweep: 1.1658271779114684,
        reversed: true,
    }
}

fn circle_of(t: &Trimmed) -> &Circle {
    match t.basis() {
        Curve::Circle(c) => c,
        other => panic!("basis is not a circle: {other:?}"),
    }
}

fn check_against_corpus(e: &Expect) {
    let arc = Trimmed::from_bulge(e.p0, e.p1, e.bulge).expect("the corpus probe is a valid bulge");
    let c = circle_of(&arc);
    let tr = arc.trim();
    assert!(near(c.placement().origin(), e.centre, e.radius), "centre {:?}", c.placement().origin());
    assert!(P.near(c.radius(), e.radius, e.radius), "radius {}", c.radius());
    assert!(P.near(tr.start, e.start, TAU), "start {}", tr.start);
    assert!(P.near(tr.span, e.sweep, TAU), "span {}", tr.span);
    assert_eq!(tr.reversed, e.reversed);
    // The trimmed circle runs from the first vertex to the second, whichever way round it goes.
    let chord = e.p0.distance(e.p1);
    let d = arc.domain();
    assert!(near(arc.eval(d.lo()).unwrap(), e.p0, chord), "start point");
    assert!(near(arc.eval(d.hi()).unwrap(), e.p1, chord), "end point");
}

#[test]
fn the_positive_bulge_probe_matches_the_corpus_arc() {
    check_against_corpus(&positive());
}

#[test]
fn the_negative_bulge_probe_matches_the_corpus_arc_and_is_a_reversed_trim() {
    check_against_corpus(&negative());
}

#[test]
fn swapping_the_end_points_and_the_sign_gives_the_same_locus_run_the_other_way() {
    for e in [positive(), negative()] {
        let a = Trimmed::from_bulge(e.p0, e.p1, e.bulge).unwrap();
        let b = Trimmed::from_bulge(e.p1, e.p0, -e.bulge).unwrap();
        let (ca, cb) = (circle_of(&a), circle_of(&b));
        assert!(near(ca.placement().origin(), cb.placement().origin(), ca.radius()));
        assert!(P.near(ca.radius(), cb.radius(), ca.radius()));
        assert!(P.near(a.trim().start, b.trim().start, TAU));
        assert!(P.near(a.trim().span, b.trim().span, TAU));
        assert_ne!(a.trim().reversed, b.trim().reversed);
    }
}

#[test]
fn a_bulge_of_one_is_a_half_circle_on_the_chord() {
    let arc = Trimmed::from_bulge(pt(0.0, 0.0), pt(2.0, 0.0), 1.0).unwrap();
    let c = circle_of(&arc);
    assert!(near(c.placement().origin(), pt(1.0, 0.0), 1.0));
    assert!(P.near(c.radius(), 1.0, 1.0));
    assert!(P.near(arc.trim().span, PI, TAU));
    assert!(!arc.trim().reversed);
    // Counter-clockwise from (0, 0) to (2, 0) passes below the chord.
    assert!(P.near(arc.eval(arc.domain().hi() / 2.0).unwrap().y, -1.0, 1.0));
}

#[test]
fn the_span_is_four_atan_of_the_bulge_magnitude_for_either_sign() {
    for b in [0.05, 0.2, 0.4142, 0.9, 1.0, 1.7, 5.0, 40.0] {
        for sign in [1.0, -1.0] {
            let arc = Trimmed::from_bulge(pt(-3.0, 2.0), pt(5.0, -1.0), sign * b).unwrap();
            assert!(P.near(arc.trim().span, 4.0 * f64::atan(b), TAU), "bulge {b}");
            assert_eq!(arc.trim().reversed, sign < 0.0);
        }
    }
}

#[test]
fn random_chords_and_bulges_end_on_both_vertices() {
    let mut rng = Rng(0x91);
    for _ in 0..500 {
        let p0 = pt(rng.unit() * 200.0 - 100.0, rng.unit() * 200.0 - 100.0);
        let p1 = pt(rng.unit() * 200.0 - 100.0, rng.unit() * 200.0 - 100.0);
        let b = (rng.unit() * 4.0 - 2.0) * 1.0;
        if b == 0.0 {
            continue;
        }
        let arc = Trimmed::from_bulge(p0, p1, b).unwrap();
        let chord = p0.distance(p1);
        let d = arc.domain();
        assert!(near(arc.eval(d.lo()).unwrap(), p0, chord));
        assert!(near(arc.eval(d.hi()).unwrap(), p1, chord));
        assert_eq!(arc.trim().reversed, b < 0.0);
    }
}

#[test]
fn the_same_input_gives_the_same_bits() {
    let e = negative();
    let a = Trimmed::from_bulge(e.p0, e.p1, e.bulge).unwrap();
    let b = Trimmed::from_bulge(e.p0, e.p1, e.bulge).unwrap();
    assert_eq!(a, b);
}

#[test]
fn the_closing_segment_of_a_closed_polyline_uses_the_same_constructor() {
    // Two half circles make a full ring: the second segment is the closing one.
    let (a, b) = (pt(0.0, 0.0), pt(2.0, 0.0));
    let s0 = Trimmed::from_bulge(a, b, 1.0).unwrap();
    let s1 = Trimmed::from_bulge(b, a, 1.0).unwrap();
    let ring = Composite::new(vec![Curve::Trimmed(s0), Curve::Trimmed(s1)]).unwrap();
    assert!(ring.declare_closed().is_ok());
}

#[test]
fn a_zero_bulge_is_refused_as_a_line_under_geom_02() {
    for z in [0.0, -0.0] {
        assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1.0, 0.0), z)), "GEOM-02");
    }
}

#[test]
fn a_non_finite_or_subnormal_bulge_is_refused_under_np01() {
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MIN_POSITIVE / 2.0] {
        assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1.0, 0.0), bad)), "NP-01");
    }
}

#[test]
fn a_non_finite_end_point_is_refused_under_np01_and_a_far_one_under_np02() {
    assert_eq!(rule_of(Trimmed::from_bulge(pt(f64::NAN, 0.0), pt(1.0, 0.0), 0.5)), "NP-01");
    assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(0.0, f64::INFINITY), 0.5)), "NP-01");
    assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1e13, 0.0), 0.5)), "NP-02");
}

#[test]
fn a_degenerate_chord_is_refused_under_np06() {
    assert_eq!(rule_of(Trimmed::from_bulge(pt(1.0, 1.0), pt(1.0, 1.0), 0.5)), "NP-06");
    assert_eq!(rule_of(Trimmed::from_bulge(pt(1.0, 1.0), pt(1.0 + 1e-13, 1.0), 0.5)), "NP-06");
}

#[test]
fn a_radius_the_policy_cannot_hold_is_refused_under_np07_or_np02() {
    // Huge bulge: the radius overflows to infinity, which NP-07 refuses.
    assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1.0, 0.0), 1e308)), "NP-07");
    // Tiny bulge: the radius is finite but the centre falls outside the coordinate bound (NP-02).
    assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1.0, 0.0), 1e-20)), "NP-02");
    // Tinier still: the radius overflows to infinity, again NP-07.
    assert_eq!(rule_of(Trimmed::from_bulge(pt(0.0, 0.0), pt(1.0, 0.0), 1e-308)), "NP-07");
}
