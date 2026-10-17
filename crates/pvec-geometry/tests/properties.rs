//! Property and hostile-input tests (P3.1), driven by a seeded generator so a failure prints the
//! seed that reproduces it. No external property-testing dependency, on purpose.
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::TAU;
use pvec_geometry::*;

const SEED: u64 = 0x5eed_0001;
// Iteration counts are modest: the whole suite also runs on a loaded shared machine.
const CASES: usize = 400;

fn random_frame(r: &mut Rng) -> Placement2 {
    let ang = r.range(0.0, TAU);
    Placement2::new(pt(r.range(-50.0, 50.0), r.range(-50.0, 50.0)), v(ang.cos(), ang.sin())).unwrap()
}

fn random_transform(r: &mut Rng) -> Transform2 {
    loop {
        let m = [
            r.range(-4.0, 4.0),
            r.range(-4.0, 4.0),
            r.range(-4.0, 4.0),
            r.range(-4.0, 4.0),
            r.range(-20.0, 20.0),
            r.range(-20.0, 20.0),
        ];
        // Keep the image well conditioned: the property is about exactness, not about NP-07.
        if (m[0] * m[3] - m[1] * m[2]).abs() > 0.5 {
            if let Ok(t) = Transform2::new(m) {
                return t;
            }
        }
    }
}

/// A valid clamped B-spline with random interior knots, random control points and optional weights.
fn random_spline(r: &mut Rng, rational: bool) -> Curve {
    loop {
        let degree = 1 + r.below(5);
        let interior = r.below(5);
        let n = degree + 1 + interior;
        let cps: Vec<Point2> = (0..n).map(|_| pt(r.range(-30.0, 30.0), r.range(-30.0, 30.0))).collect();
        let mut inner: Vec<f64> = (0..interior).map(|_| r.range(0.05, 0.95)).collect();
        inner.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut knots = vec![0.0; degree + 1];
        knots.extend(inner);
        knots.extend(vec![1.0; degree + 1]);
        let made = if rational {
            let w: Vec<f64> = (0..n).map(|_| r.range(0.5, 3.0)).collect();
            RationalBSpline::new(degree, cps, knots, w).map(Curve::RationalBSpline)
        } else {
            BSpline::new(degree, cps, knots).map(Curve::BSpline)
        };
        if let Ok(c) = made {
            return c;
        }
    }
}

#[test]
fn transform_then_evaluate_equals_evaluate_then_transform_for_random_conics() {
    let mut r = Rng(SEED);
    for case in 0..CASES {
        let f = random_frame(&mut r);
        let a = r.range(0.5, 20.0);
        let b = a * r.range(0.3, 1.0);
        let curve = if case % 3 == 0 {
            Curve::Circle(Circle::new(f, a).unwrap())
        } else {
            Curve::Ellipse(Ellipse::new(f, a, b).unwrap())
        };
        let m = random_transform(&mut r);
        let (img, pm) = curve.transform(&m).unwrap_or_else(|e| panic!("seed {SEED:#x} case {case}: {e}"));
        for i in 0..=24 {
            let t = TAU * i as f64 / 24.0;
            let want = m.apply_point(curve.eval(t).unwrap());
            let got = img.eval(pm.apply(t)).unwrap();
            let scale = want.x.abs().max(want.y.abs()).max(a);
            assert!(near(got, want, scale), "seed {SEED:#x} case {case} t={t}: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn transform_then_evaluate_equals_evaluate_then_transform_for_random_splines() {
    let mut r = Rng(SEED + 1);
    for case in 0..CASES {
        let curve = random_spline(&mut r, case % 2 == 0);
        let m = random_transform(&mut r);
        let (img, pm) = curve.transform(&m).unwrap();
        assert_eq!(pm, ParamMap::IDENTITY);
        assert_eq!(img.domain(), curve.domain());
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            let want = m.apply_point(curve.eval(t).unwrap());
            let got = img.eval(t).unwrap();
            let scale = want.x.abs().max(want.y.abs()).max(1.0);
            assert!(near(got, want, scale), "seed {:#x} case {case} t={t}", SEED + 1);
        }
    }
}

#[test]
fn spline_points_stay_inside_the_control_hull_box_and_the_reported_bounds() {
    let mut r = Rng(SEED + 2);
    for case in 0..CASES {
        let curve = random_spline(&mut r, case % 2 == 1);
        let b = curve.bounds().unwrap();
        let slack = P.tol(b.diagonal(), b.magnitude());
        for i in 0..=40 {
            let q = curve.eval(i as f64 / 40.0).unwrap();
            assert!(
                q.x >= b.min().x - slack
                    && q.x <= b.max().x + slack
                    && q.y >= b.min().y - slack
                    && q.y <= b.max().y + slack,
                "seed {:#x} case {case}",
                SEED + 2
            );
        }
    }
}

#[test]
fn spline_first_derivative_matches_a_central_difference_away_from_knots() {
    let mut r = Rng(SEED + 3);
    for case in 0..CASES {
        let curve = random_spline(&mut r, case % 2 == 0);
        let t = r.range(0.01, 0.99);
        let h = 1.0 / 100_000.0; // finite-difference step, not a tolerance
        let (a, b) = (curve.eval(t - h).unwrap(), curve.eval(t + h).unwrap());
        let num = ((b.x - a.x) / (2.0 * h), (b.y - a.y) / (2.0 * h));
        let d = curve.derivative(t, 1, Side::Right).unwrap();
        let scale = d.x.abs().max(d.y.abs()).max(1.0);
        // A knot inside the stencil changes the polynomial piece; allow a loose difference quotient.
        assert!(
            (d.x - num.0).abs() <= scale / 100.0 && (d.y - num.1).abs() <= scale / 100.0,
            "seed {:#x} case {case} t={t}: {d:?} vs {num:?}",
            SEED + 3
        );
    }
}

#[test]
fn trimmed_arcs_start_and_end_at_the_basis_points_for_random_trims() {
    let mut r = Rng(SEED + 4);
    let mut accepted = 0;
    for case in 0..CASES {
        let f = random_frame(&mut r);
        let basis = Curve::Ellipse(Ellipse::new(f, r.range(1.0, 10.0), r.range(0.5, 1.0)).unwrap());
        let (t1, t2) = (r.range(-10.0, 10.0), r.range(-10.0, 10.0));
        let sense = r.below(2) == 0;
        let Ok(tc) = Trimmed::new(basis.clone(), t1, t2, sense) else { continue };
        accepted += 1;
        let d = tc.domain();
        let scale = 10.0;
        assert!(
            near(tc.eval(d.lo()).unwrap(), basis.eval(t1).unwrap(), scale),
            "seed {:#x} case {case}",
            SEED + 4
        );
        assert!(
            near(tc.eval(d.hi()).unwrap(), basis.eval(t2).unwrap(), scale),
            "seed {:#x} case {case}",
            SEED + 4
        );
        // the normalised form survives a rebuild bit for bit
        let again = Trimmed::from_normalised(basis, tc.trim()).unwrap();
        assert_eq!(again.trim().start.to_bits(), tc.trim().start.to_bits());
        assert_eq!(again.trim().span.to_bits(), tc.trim().span.to_bits());
    }
    assert!(accepted > CASES / 4, "the generator should produce mostly valid trims, got {accepted}");
}

#[test]
fn composition_of_random_valid_pieces_never_panics_and_joints_are_continuous() {
    let mut r = Rng(SEED + 5);
    for case in 0..100 {
        let n = 2 + r.below(5);
        let mut at = pt(r.range(-10.0, 10.0), r.range(-10.0, 10.0));
        let start = at;
        let mut segs = Vec::new();
        for _ in 0..n {
            let next = pt(at.x + r.range(1.0, 5.0), at.y + r.range(-5.0, 5.0));
            segs.push(Curve::Polyline(Polyline::new(vec![at, next]).unwrap()));
            at = next;
        }
        let c = Composite::new(segs).unwrap();
        let end = Curve::Composite(c.clone()).eval(n as f64).unwrap();
        assert!(near(end, at, 50.0), "case {case}");
        assert!(near(Curve::Composite(c).eval(0.0).unwrap(), start, 50.0));
    }
}

// ---------------------------------------------------------------- hostile input

#[test]
fn arbitrary_bit_patterns_are_either_refused_or_produce_a_curve_that_evaluates_without_panicking() {
    let mut r = Rng(SEED + 6);
    let mut built = 0usize;
    for _ in 0..CASES * 10 {
        let mut f = || if r.below(3) == 0 { r.any_f64() } else { r.range(-5.0, 5.0) };
        let (a, b, c, d) = (f(), f(), f(), f());
        let mut made: Vec<Curve> = Vec::new();
        if let Ok(l) = Line::new(Point2::new(a, b), Vector2::new(c, d)) {
            made.push(Curve::Line(l));
        }
        if let Ok(p) = Placement2::new(Point2::new(a, b), Vector2::new(c, d)) {
            if let Ok(ci) = Circle::new(p, d) {
                made.push(Curve::Circle(ci));
            }
            if let Ok(e) = Ellipse::new(p, c, d) {
                made.push(Curve::Ellipse(e));
            }
        }
        if let Ok(p) = Polyline::new(vec![Point2::new(a, b), Point2::new(c, d), Point2::new(a, d)]) {
            made.push(Curve::Polyline(p));
        }
        if let Ok(z) = Bezier::new(vec![Point2::new(a, b), Point2::new(c, d), Point2::new(b, c)]) {
            made.push(Curve::Bezier(z));
        }
        if let Ok(s) = BSpline::new(
            2,
            vec![Point2::new(a, b), Point2::new(c, d), Point2::new(b, c)],
            vec![a, b, c, d, a, b],
        ) {
            made.push(Curve::BSpline(s));
        }
        if let Ok(s) = RationalBSpline::new(
            2,
            vec![Point2::new(a, b), Point2::new(c, d), Point2::new(b, c)],
            vec![0., 0., 0., 1., 1., 1.],
            vec![a, b, c],
        ) {
            made.push(Curve::RationalBSpline(s));
        }
        if let Some(Curve::Circle(ci)) = made.iter().find(|m| m.kind() == CurveKind::Circle) {
            if let Ok(t) = Trimmed::new(Curve::Circle(*ci), a, b, c > 0.0) {
                made.push(Curve::Trimmed(t));
            }
        }
        if let Ok(m) = Transform2::new([a, b, c, d, a, d]) {
            for cv in made.clone() {
                let _ = cv.transform(&m);
            }
        }
        for cv in &made {
            built += 1;
            let dom = cv.domain();
            for t in [dom.lo(), dom.hi(), 0.5 * (dom.lo() + dom.hi()), a, b, f64::NAN] {
                if let Ok(q) = cv.eval(t) {
                    assert!(q.x.is_finite() && q.y.is_finite(), "{:?} at {t}", cv.kind());
                }
                for side in [Side::Left, Side::Right, Side::Both] {
                    let _ = cv.derivative(t, 1, side);
                    let _ = cv.derivative(t, 3, side);
                }
            }
            let _ = cv.bounds();
        }
    }
    assert!(built > 100, "the generator should build some curves, built {built}");
}

#[test]
fn a_construction_error_can_never_be_unwrapped_into_a_curve() {
    let mut r = Rng(SEED + 7);
    for _ in 0..CASES {
        let n = 2 + r.below(6);
        let pts: Vec<Point2> = (0..n).map(|_| Point2::new(r.any_f64(), r.any_f64())).collect();
        match Polyline::new(pts.clone()) {
            Ok(p) => {
                assert!(p.vertices().iter().all(|q| q.x.is_finite() && q.y.is_finite()));
                assert!(p.vertices().iter().all(|q| q.x.abs() <= pvec_numeric::MAX_ABS_COORD));
            }
            Err(e) => assert!(!e.rule().is_empty()),
        }
    }
}

#[test]
fn deep_and_wide_inputs_stay_inside_the_documented_limits() {
    // The structure itself bounds nesting at 3 (composite, trimmed, basis); a wide spline is fine.
    let (_, comp) = samples().into_iter().find(|(n, _)| *n == "composite").unwrap();
    assert!(comp.nesting_depth() <= pvec_numeric::MAX_NESTING_DEPTH);
    assert_eq!(comp.nesting_depth(), 3);
    let n = pvec_numeric::MAX_CONTROL_POINTS;
    let cps: Vec<Point2> = (0..n).map(|i| pt(i as f64, (i % 13) as f64)).collect();
    let mut knots = vec![0.0; 4];
    knots.extend((1..n - 3).map(|i| i as f64 / (n - 3) as f64));
    knots.extend(vec![1.0; 4]);
    let s = BSpline::new(3, cps, knots).unwrap();
    assert!(s.eval(0.5).is_ok());
}
