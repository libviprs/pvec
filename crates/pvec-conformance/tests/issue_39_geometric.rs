//! Issue #39: geometric equivalence against an independent evaluation. The evaluators below are
//! closed-form, so nothing here shares a line with the evaluator under test.

mod common;

use pvec_capabilities::EvidenceClass as C;
use pvec_conformance::geometric::*;
use pvec_geometry::{Bezier, Circle, Placement2, Point2, Vector2};
use pvec_numeric::{Policy, Side};
use std::cell::RefCell;
use std::f64::consts::TAU;

/// The unit circle, closed form.
struct UnitCircle {
    own: bool,
}

impl Evaluator for UnitCircle {
    fn source(&self) -> Source {
        if self.own { Source::Own } else { Source::Independent }
    }
    fn domain(&self) -> [f64; 2] {
        [0.0, TAU]
    }
    fn breaks(&self) -> Vec<Break> {
        vec![]
    }
    fn point(&self, t: f64) -> [f64; 3] {
        [t.cos(), t.sin(), 0.0]
    }
    fn tangent(&self, t: f64, _: Side) -> Option<[f64; 3]> {
        Some([-t.sin(), t.cos(), 0.0])
    }
    fn curvature(&self, _: f64, _: Side) -> Option<f64> {
        Some(1.0)
    }
    fn arc_length(&self) -> f64 {
        TAU
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        Some(([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]))
    }
}

/// Wraps an evaluator and adds one smooth bump to the point, centred between two nodes of the
/// 17-point uniform grid, wide enough that the neighbouring nodes see part of it.
struct Planted<E> {
    inner: E,
    amplitude: f64,
}

impl<E: Evaluator> Evaluator for Planted<E> {
    fn source(&self) -> Source {
        Source::Own
    }
    fn domain(&self) -> [f64; 2] {
        self.inner.domain()
    }
    fn breaks(&self) -> Vec<Break> {
        self.inner.breaks()
    }
    fn point(&self, t: f64) -> [f64; 3] {
        let [lo, hi] = self.domain();
        let u = (t - lo) / (hi - lo);
        // Centre 7.5/16; width chosen so a node half a cell away sees 40 percent of the peak.
        let (centre, sigma) = (7.5 / 16.0, 0.0326);
        let bump = self.amplitude * (-((u - centre) / sigma).powi(2)).exp();
        let p = self.inner.point(t);
        [p[0], p[1] + bump, p[2]]
    }
    fn tangent(&self, t: f64, s: Side) -> Option<[f64; 3]> {
        self.inner.tangent(t, s)
    }
    fn curvature(&self, t: f64, s: Side) -> Option<f64> {
        self.inner.curvature(t, s)
    }
    fn arc_length(&self) -> f64 {
        self.inner.arc_length()
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        self.inner.bounds()
    }
}

/// y = t^2 up to t = 1/2, then a parabola with the same position and slope but curvature `k` times
/// larger there: C1 at the break, curvature jumps. Records every derivative request it gets.
struct TwoParabolas {
    own: bool,
    right_k: f64,
    /// A bug to plant: answer the right-hand curvature at the break with the left-hand value.
    left_value_on_right: bool,
    log: RefCell<Vec<(f64, Side, usize)>>,
}

impl TwoParabolas {
    fn new(own: bool) -> TwoParabolas {
        TwoParabolas { own, right_k: 3.0, left_value_on_right: false, log: RefCell::new(vec![]) }
    }
    fn slope(&self, t: f64, right: bool) -> f64 {
        if right { 1.0 + 2.0 * self.right_k * (t - 0.5) } else { 2.0 * t }
    }
}

impl Evaluator for TwoParabolas {
    fn source(&self) -> Source {
        if self.own { Source::Own } else { Source::Independent }
    }
    fn domain(&self) -> [f64; 2] {
        [0.0, 1.0]
    }
    fn breaks(&self) -> Vec<Break> {
        vec![Break { parameter: 0.5, two_sided_order: 1 }]
    }
    fn point(&self, t: f64) -> [f64; 3] {
        if t < 0.5 { [t, t * t, 0.0] } else { [t, 0.25 + (t - 0.5) + self.right_k * (t - 0.5).powi(2), 0.0] }
    }
    fn tangent(&self, t: f64, side: Side) -> Option<[f64; 3]> {
        self.log.borrow_mut().push((t, side, 1));
        let right = t > 0.5 || (t == 0.5 && side == Side::Right);
        let m = self.slope(t, right);
        let n = (1.0 + m * m).sqrt();
        Some([1.0 / n, m / n, 0.0])
    }
    fn curvature(&self, t: f64, side: Side) -> Option<f64> {
        self.log.borrow_mut().push((t, side, 2));
        if t == 0.5 && side == Side::Both {
            return None;
        }
        let right = t > 0.5 || (t == 0.5 && side == Side::Right && !self.left_value_on_right);
        let m = self.slope(t, right);
        let second = if right { 2.0 * self.right_k } else { 2.0 };
        Some(second / (1.0 + m * m).powf(1.5))
    }
    fn arc_length(&self) -> f64 {
        let n = 4000;
        let mut sum = 0.0;
        for right in [false, true] {
            let (a, b) = if right { (0.5, 1.0) } else { (0.0, 0.5) };
            let h = (b - a) / f64::from(n);
            for i in 0..=n {
                let w = if i == 0 || i == n {
                    1.0
                } else if i % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                let m = self.slope(a + h * f64::from(i), right);
                sum += w * (1.0 + m * m).sqrt();
            }
            sum *= h / 3.0;
        }
        sum
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        None
    }
}

const PLAN: Plan = Plan { seed: 7, samples: 24, budget: 17 };

fn metric<'a>(r: &'a Report, name: &str) -> &'a Metric {
    r.metrics.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no `{name}` metric"))
}

#[test]
fn issue_39_every_fixture_reports_a_deviation_and_its_worst_point_on_pass_and_on_fail() {
    let r = compare(
        "circle-001",
        C::KnownTruthExact,
        &UnitCircle { own: false },
        &UnitCircle { own: true },
        PLAN,
    )
    .unwrap();
    assert!(r.passed);
    let tol = Policy::V1.tol(2.0, 1.0);
    for name in ["endpoints", "position", "tangent", "curvature"] {
        let m = metric(&r, name);
        assert!(m.max.is_finite() && m.max >= 0.0, "{name}");
        assert!(m.at.is_some(), "{name} has no worst point");
        assert!(m.tolerance > 0.0 && m.within, "{name}");
    }
    assert_eq!(metric(&r, "position").tolerance, tol);
    let failing = Planted { inner: UnitCircle { own: true }, amplitude: tol * 1.5 };
    let r = compare("circle-001", C::KnownTruthExact, &UnitCircle { own: false }, &failing, PLAN).unwrap();
    assert!(!r.passed);
    let m = metric(&r, "position");
    assert!(m.max > m.tolerance && !m.within && m.at.is_some());
    let text = render(&r);
    for line in text.lines() {
        for needle in ["max_deviation=", "at=", "tolerance=", "KNOWN_TRUTH_EXACT"] {
            assert!(line.contains(needle), "`{needle}` missing from {line}");
        }
    }
}

#[test]
fn issue_39_sample_sets_are_seeded_reproducible_and_a_difference_fails() {
    let reference = TwoParabolas::new(false);
    let a = Run::new(&reference, PLAN);
    let b = Run::new(&reference, PLAN);
    assert_eq!(a, b);
    a.same_as(&b).expect("two runs agree");
    assert_ne!(a, Run::new(&reference, Plan { seed: 8, ..PLAN }));
    let mut moved = b.clone();
    moved.parameters[5] = f64::from_bits(moved.parameters[5].to_bits() + 1);
    assert!(a.same_as(&moved).is_err(), "a one-ulp difference between runs must fail");
    let report = compare("x", C::KnownTruthExact, &reference, &TwoParabolas::new(true), PLAN).unwrap();
    assert_eq!(report.run, a, "the report records the sample set it used");
    assert_eq!(report.run.seed, 7);
}

#[test]
fn issue_39_samples_include_both_endpoints_and_the_knot_with_epsilon_either_side() {
    let reference = TwoParabolas::new(false);
    let run = Run::new(&reference, PLAN);
    let eps = knot_epsilon([0.0, 1.0], 0.5);
    assert!(eps > 0.0 && eps < 0.5);
    for want in [0.0, 1.0, 0.5, 0.5 - eps, 0.5 + eps] {
        assert!(run.parameters.contains(&want), "{want} not sampled: {:?}", run.parameters);
    }
    assert!(run.parameters.iter().all(|t| (0.0..=1.0).contains(t)));
}

#[test]
fn issue_39_adaptive_refinement_finds_a_planted_deviation_that_fixed_step_sampling_misses() {
    let reference = UnitCircle { own: false };
    let tol = position_tolerance(&reference);
    let planted = Planted { inner: UnitCircle { own: true }, amplitude: tol * 1.5 };
    let fixed = fixed_step(&reference, &planted, 17);
    let adaptive = adaptive(&reference, &planted, 17, tol);
    assert!(fixed.max <= tol, "fixed-step sampling was expected to miss: {} vs {tol}", fixed.max);
    assert!(adaptive.max > tol, "adaptive refinement missed it: {} vs {tol}", adaptive.max);
    assert!(adaptive.max > fixed.max);
    let centre = TAU * 7.5 / 16.0;
    assert!((adaptive.at.unwrap() - centre).abs() < TAU / 64.0, "worst point {:?}", adaptive.at);
    // Both results are recorded in the report, next to the budget they shared.
    let r = compare("circle-planted", C::KnownTruthExact, &reference, &planted, Plan { budget: 17, ..PLAN })
        .unwrap();
    assert_eq!(r.refinement.budget, 17);
    assert_eq!(r.refinement.fixed.max, fixed.max);
    assert_eq!(r.refinement.adaptive.max, adaptive.max);
    assert!(!r.passed, "the planted deviation must fail the fixture");
    let text = render(&r);
    assert!(text.contains("fixed-step") && text.contains("adaptive"), "{text}");
}

#[test]
fn issue_39_curvature_is_not_asserted_two_sided_at_a_c1_break() {
    let brk = Break { parameter: 0.5, two_sided_order: 1 };
    assert!(request(1, &brk, Side::Both).is_ok(), "the tangent is continuous at a C1 break");
    assert!(request(2, &brk, Side::Both).is_err(), "a two-sided curvature at a C1 break must fail");
    assert!(request(2, &brk, Side::Left).is_ok() && request(2, &brk, Side::Right).is_ok());

    // The comparison itself never asks for it: it queries the two sides of the break separately.
    let reference = TwoParabolas::new(false);
    let candidate = TwoParabolas::new(true);
    let r = compare("c1-break", C::DualOracleEquivalent, &reference, &candidate, PLAN).unwrap();
    assert!(r.passed, "{}", render(&r));
    for side in [&reference, &candidate] {
        let log = side.log.borrow();
        assert!(
            !log.iter().any(|(t, s, k)| *t == 0.5 && *s == Side::Both && *k == 2),
            "two-sided curvature asked for at the break"
        );
        assert!(log.iter().any(|(t, s, k)| *t == 0.5 && *s == Side::Left && *k == 2));
        assert!(log.iter().any(|(t, s, k)| *t == 0.5 && *s == Side::Right && *k == 2));
    }
}

#[test]
fn issue_39_a_wrong_one_sided_value_at_the_break_is_caught_at_the_break() {
    let mut candidate = TwoParabolas::new(true);
    candidate.left_value_on_right = true;
    let r = compare("c1-break", C::KnownTruthExact, &TwoParabolas::new(false), &candidate, PLAN).unwrap();
    assert!(!r.passed);
    let m = metric(&r, "curvature");
    assert!(!m.within);
    assert_eq!(m.at, Some(0.5));
    assert!(metric(&r, "tangent").within && metric(&r, "position").within);
}

#[test]
fn issue_39_endpoints_bounds_and_arc_length_are_compared_separately() {
    struct WrongLength(UnitCircle);
    impl Evaluator for WrongLength {
        fn source(&self) -> Source {
            Source::Own
        }
        fn domain(&self) -> [f64; 2] {
            self.0.domain()
        }
        fn breaks(&self) -> Vec<Break> {
            vec![]
        }
        fn point(&self, t: f64) -> [f64; 3] {
            self.0.point(t)
        }
        fn tangent(&self, t: f64, s: Side) -> Option<[f64; 3]> {
            self.0.tangent(t, s)
        }
        fn curvature(&self, t: f64, s: Side) -> Option<f64> {
            self.0.curvature(t, s)
        }
        fn arc_length(&self) -> f64 {
            TAU * 1.001
        }
        fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
            Some(([-1.0, -1.0, 0.0], [1.0, 1.5, 0.0]))
        }
    }
    let r = compare(
        "circle-001",
        C::KnownTruthExact,
        &UnitCircle { own: false },
        &WrongLength(UnitCircle { own: true }),
        PLAN,
    )
    .unwrap();
    assert!(!metric(&r, "arc_length").within && !metric(&r, "bounds").within);
    assert!(metric(&r, "position").within && metric(&r, "endpoints").within && metric(&r, "tangent").within);
    assert!(!r.passed);
}

#[test]
fn issue_39_this_repositorys_evaluator_is_never_the_reference() {
    let own = UnitCircle { own: true };
    assert!(compare("x", C::KnownTruthExact, &own, &UnitCircle { own: true }, PLAN).is_err());
    assert!(
        compare("x", C::KnownTruthExact, &UnitCircle { own: false }, &UnitCircle { own: false }, PLAN)
            .is_err()
    );
}

#[test]
fn issue_39_the_geometry_crate_evaluates_a_circle_and_a_parabola_to_the_independent_answer() {
    let placement = Placement2::new(Point2::new(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap();
    let circle = Circle::new(placement, 1.0).unwrap();
    let r = compare(
        "circle-001",
        C::KnownTruthExact,
        &UnitCircle { own: false },
        &Candidate { curve: &circle, breaks: vec![] },
        PLAN,
    )
    .unwrap();
    assert!(r.passed, "{}", render(&r));

    struct Parabola;
    impl Evaluator for Parabola {
        fn source(&self) -> Source {
            Source::Independent
        }
        fn domain(&self) -> [f64; 2] {
            [0.0, 1.0]
        }
        fn breaks(&self) -> Vec<Break> {
            vec![]
        }
        fn point(&self, t: f64) -> [f64; 3] {
            [2.0 * t, 4.0 * t * (1.0 - t), 0.0]
        }
        fn tangent(&self, t: f64, _: Side) -> Option<[f64; 3]> {
            let (x, y): (f64, f64) = (2.0, 4.0 * (1.0 - 2.0 * t));
            let n = x.hypot(y);
            Some([x / n, y / n, 0.0])
        }
        fn curvature(&self, t: f64, _: Side) -> Option<f64> {
            let (dx, dy): (f64, f64) = (2.0, 4.0 * (1.0 - 2.0 * t));
            Some(dx * -8.0 / dx.hypot(dy).powi(3))
        }
        fn arc_length(&self) -> f64 {
            5.0f64.sqrt() + 2.0f64.asinh() / 2.0
        }
        fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
            None
        }
    }
    let bezier =
        Bezier::new(vec![Point2::new(0.0, 0.0), Point2::new(1.0, 2.0), Point2::new(2.0, 0.0)]).unwrap();
    let r = compare(
        "bezier-d2-001",
        C::DualOracleEquivalent,
        &Parabola,
        &Candidate { curve: &bezier, breaks: vec![] },
        PLAN,
    )
    .unwrap();
    assert!(r.passed, "{}", render(&r));
    assert!(metric(&r, "arc_length").max <= metric(&r, "arc_length").tolerance);
}

#[test]
fn issue_39_the_module_writes_no_tolerance_and_a_geometric_pass_is_never_worded_as_structural() {
    common::assert_no_tolerance_literal("geometric.rs");
    let r = compare(
        "circle-001",
        C::RealWorldDifferential,
        &UnitCircle { own: false },
        &UnitCircle { own: true },
        PLAN,
    )
    .unwrap();
    let text = render(&r);
    check_wording(&text).expect("the emitted text passes its own check");
    assert!(text.contains("regression stability") && !text.to_lowercase().contains("structural"), "{text}");
    for bad in
        ["structurally exact", "structural exactness holds", "display fidelity is exact", "pixel-exact"]
    {
        assert!(check_wording(bad).is_err(), "{bad}");
    }
}
