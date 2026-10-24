//! Issue 93: a periodic B-spline is unwrapped exactly into the clamped form.
//!
//! A periodic spline is given as a decoder reports it: the degree, the `n` unique control points and
//! the `n + 1` knots of one period (the first and the last name the same knot, one period apart).
//! The curve it describes is the one with the control points repeated and the knots extended by the
//! period. `BSpline::from_periodic` returns that locus as a clamped, closed `BSpline` plus a bound on
//! how far the f64 arithmetic can have moved the control points. The oracle below never calls it: it
//! builds the wrapped, unclamped form by hand and evaluates that with the one evaluator.
#![allow(missing_docs)]

mod common;
use common::*;
use pvec_geometry::*;

const U: f64 = 1.0 / 9007199254740992.0; // 2^-53, the unit roundoff

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

/// The corpus probe `closed-periodic-001` (tag `corpus-r0-open`, params in pvec-gold-fixtures):
/// a periodic cubic, six unique control points, seven knots.
fn probe() -> (usize, Vec<Point2>, Vec<f64>) {
    let cps = vec![
        pt(3., 0.),
        pt(1.5, 2.6),
        pt(-1.5, 2.6),
        pt(-3., 0.),
        pt(-1.5, -2.6),
        pt(1.5, -2.6),
    ];
    (3, cps, vec![0., 0.1, 0.25, 0.4, 0.6, 0.8, 1.])
}

/// The wrapped, unclamped form, built independently of the code under test. Control point `i`
/// is the one of the unique list at `i mod n`; knot `j` of the extended vector is `k_{j-p}` with
/// the period added or removed outside `0..=n`.
fn wrapped(p: usize, cps: &[Point2], k: &[f64]) -> BSpline {
    let n = cps.len();
    let t = k[n] - k[0];
    let pts: Vec<Point2> = (0..n + p).map(|i| cps[i % n]).collect();
    let mut u = Vec::new();
    for j in 0..(n + 2 * p + 1) {
        let q = j as isize - p as isize; // index into the period, may be outside 0..=n
        let (lap, r) = (q.div_euclid(n as isize), q.rem_euclid(n as isize) as usize);
        // r == 0 with lap > 0 is the knot k_0 one period later, which is k_n for lap == 1
        let v = if r == 0 && lap == 1 { k[n] } else { k[r] + lap as f64 * t };
        u.push(v);
    }
    BSpline::new(p, pts, u).expect("the wrapped form is a legal unclamped B-spline")
}

fn max_abs(cps: &[Point2]) -> f64 {
    cps.iter().fold(0.0f64, |m, q| m.max(q.x.abs()).max(q.y.abs()))
}

fn dist(a: Point2, b: Point2) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// Largest distance between the unwrapped curve and the wrapped one over `m + 1` parameters.
fn worst(p: usize, cps: &[Point2], k: &[f64], m: usize) -> (f64, f64) {
    let un = BSpline::from_periodic(p, cps.to_vec(), k.to_vec()).expect("unwraps");
    let w = wrapped(p, cps, k);
    let (lo, hi) = (un.curve.domain(), w.domain());
    assert_eq!((lo.lo(), lo.hi()), (hi.lo(), hi.hi()), "same domain");
    let mut worst = 0.0f64;
    for i in 0..=m {
        let t = lo.lo() + (lo.hi() - lo.lo()) * (i as f64 / m as f64);
        worst = worst.max(dist(un.curve.eval(t).unwrap(), w.eval(t).unwrap()));
    }
    (worst, un.deviation_bound)
}

#[test]
fn the_probe_unwraps_to_a_clamped_closed_cubic_on_its_own_domain() {
    let (p, cps, k) = probe();
    let un = BSpline::from_periodic(p, cps, k).expect("the probe unwraps");
    let c = &un.curve;
    assert_eq!(c.degree(), 3);
    assert!(c.is_declared_closed(), "closure is declared under NP-20, never inferred by a reader");
    let kk = c.knots();
    assert_eq!(kk.len(), c.control_points().len() + 4);
    assert!(kk[..4].iter().all(|&x| x == 0.0), "clamped at the start: {kk:?}");
    assert!(kk[kk.len() - 4..].iter().all(|&x| x == 1.0), "clamped at the end: {kk:?}");
    let d = c.domain();
    assert_eq!((d.lo(), d.hi()), (0.0, 1.0));
    let (a, b) = (c.eval(0.0).unwrap(), c.eval(1.0).unwrap());
    assert!(near(a, b, 6.0), "the ends meet: {a:?} {b:?}");
}

#[test]
fn the_probe_locus_equals_the_wrapped_form_inside_the_stated_bound() {
    let (p, cps, k) = probe();
    let (w, bound) = worst(p, &cps, &k, 2000);
    let m = max_abs(&cps);
    // The bound is a few roundoffs of the largest coordinate, far inside the NP-04 tolerance.
    assert!(bound > 0.0 && bound < 1e3 * U * m, "bound {bound}");
    // The two evaluations carry their own de Boor rounding; the slack is that, not the unwrap.
    assert!(w <= bound + 200.0 * U * m, "deviation {w} bound {bound}");
}

#[test]
fn a_uniform_cubic_hits_the_textbook_values_at_the_knots() {
    // k_j = j, six points: C(j) = (P_j + 4 P_{j+1} + P_{j+2}) / 6, indices modulo 6.
    let (_, cps, _) = probe();
    let k: Vec<f64> = (0..=6).map(|j| j as f64).collect();
    let un = BSpline::from_periodic(3, cps.clone(), k).unwrap();
    for j in 0..6usize {
        let (a, b, c) = (cps[j], cps[(j + 1) % 6], cps[(j + 2) % 6]);
        let want = pt((a.x + 4.0 * b.x + c.x) / 6.0, (a.y + 4.0 * b.y + c.y) / 6.0);
        let got = un.curve.eval(j as f64).unwrap();
        assert!(dist(got, want) < 50.0 * U * 3.0, "t={j}: {got:?} vs {want:?}");
    }
}

#[test]
fn a_cubic_with_c0_breaks_on_the_seam_and_inside_matches_the_wrapped_form() {
    let cps: Vec<Point2> = (0..9)
        .map(|i| {
            let a = i as f64 * 0.7;
            pt(4.0 * a.cos() + 0.3 * i as f64, 3.0 * a.sin())
        })
        .collect();
    // interior knot 0.4 at multiplicity 3 (the degree); the seam knot is k_0 = k_1 and k_8 = k_9, and
    // the first and the last name one knot, so its multiplicity is 2 + 2 - 1 = 3, also the degree
    let k = vec![0., 0., 0.2, 0.4, 0.4, 0.4, 0.8, 0.9, 1., 1.];
    let (w, bound) = worst(3, &cps, &k, 3000);
    assert!(w <= bound + 400.0 * U * max_abs(&cps), "deviation {w} bound {bound}");
    // a break is a corner: the curve passes through a control point at 0.4 and at the seam
    let un = BSpline::from_periodic(3, cps, k).unwrap();
    assert!(un.curve.derivative(0.4, 1, Side::Both).is_err(), "no two-sided derivative at a break");
    assert!(un.curve.derivative(0.4, 1, Side::Left).is_ok());
}

#[test]
fn many_random_periodic_splines_agree_with_the_wrapped_form() {
    let mut r = Rng(0x0093_5eed);
    let mut worst_ratio = 0.0f64;
    for case in 0..300 {
        let p = 1 + r.below(6);
        let n = p + 1 + r.below(8);
        let cps: Vec<Point2> = (0..n).map(|_| pt(r.range(-50., 50.), r.range(-50., 50.))).collect();
        // gaps of 0.01 .. 1, an occasional repeated knot (multiplicity two, never above the degree)
        // (never at the seam, which has its own test)
        let mut k = vec![r.range(-5., 5.)];
        for i in 0..n {
            let last = *k.last().unwrap();
            let repeat = p >= 2 && i > 0 && i + 1 < n && r.below(5) == 0;
            k.push(last + if repeat { 0.0 } else { r.range(0.01, 1.0) });
        }
        let un = BSpline::from_periodic(p, cps.clone(), k.clone())
            .unwrap_or_else(|e| panic!("case {case} p={p} n={n} k={k:?}: {e}"));
        let wrapped = wrapped(p, &cps, &k);
        let d = un.curve.domain();
        let m = max_abs(&cps);
        for i in 0..=400 {
            let t = d.lo() + (d.hi() - d.lo()) * (i as f64 / 400.0);
            let e = dist(un.curve.eval(t).unwrap(), wrapped.eval(t).unwrap());
            worst_ratio = worst_ratio.max(e / (un.deviation_bound + 200.0 * U * m));
            assert!(e <= un.deviation_bound + 200.0 * U * m, "case {case} p={p} n={n} t={t} e={e}");
        }
    }
    println!("worst measured deviation over the stated bound plus evaluator slack: {worst_ratio:.3e}");
    assert!(worst_ratio <= 1.0);
}

#[test]
fn the_wrapped_unclamped_layout_is_already_a_legal_bspline() {
    // A producer that stores the wrapped control points and the extended knots (n + p points,
    // n + 2p + 1 knots) needs nothing new: BSpline::new takes it and the ends meet.
    let (p, cps, k) = probe();
    let w = wrapped(p, &cps, &k).declare_closed().unwrap();
    assert!(w.is_declared_closed());
    // and the unwrap of the unique layout is the same locus
    let (dev, bound) = worst(p, &cps, &k, 500);
    assert!(dev <= bound + 200.0 * U * 3.0);
}

#[test]
fn refusals_name_the_rule() {
    let (p, cps, k) = probe();
    let ok = |c: Vec<Point2>, kk: Vec<f64>, d: usize| BSpline::from_periodic(d, c, kk);
    // degree outside 1..=MAX_DEGREE, and fewer unique points than degree + 1
    assert_eq!(rule_of(ok(cps.clone(), k.clone(), 0)), "NP-18");
    assert_eq!(rule_of(ok(cps.clone(), k.clone(), 17)), "NP-18");
    assert_eq!(rule_of(ok(cps[..3].to_vec(), k[..4].to_vec(), 3)), "NP-18");
    // knot count must be n + 1: the clamped layout (n + p + 1) is not this layout
    assert_eq!(rule_of(ok(cps.clone(), vec![0., 0., 0., 0., 0.3, 0.6, 1., 1., 1., 1.], p)), "NP-14");
    assert_eq!(rule_of(ok(cps.clone(), k[..6].to_vec(), p)), "NP-14");
    // decreasing, empty period, ambiguous pair
    assert_eq!(rule_of(ok(cps.clone(), vec![0., 0.2, 0.1, 0.4, 0.6, 0.8, 1.], p)), "NP-14");
    assert!(ok(cps.clone(), vec![1.; 7], p).is_err());
    let mut amb = k.clone();
    amb[3] = amb[2] + 1e-14;
    assert_eq!(rule_of(ok(cps.clone(), amb, p)), "NP-14");
    // the seam knot at multiplicity degree + 1 is a break in the curve itself
    assert_eq!(rule_of(ok(cps.clone(), vec![0., 0., 0., 0.5, 0.8, 1., 1.], 3)), "NP-14");
    assert_eq!(rule_of(ok(cps.clone(), vec![0., 0., 0., 0., 0.5, 0.8, 1.], 3)), "NP-14");
    // values
    let mut bad = cps.clone();
    bad[2] = pt(f64::NAN, 0.0);
    assert_eq!(rule_of(ok(bad, k.clone(), p)), "NP-01");
    assert_eq!(rule_of(ok(cps.clone(), vec![0., 0.1, f64::INFINITY, 0.4, 0.6, 0.8, 1.], p)), "NP-01");
    assert_eq!(rule_of(ok(vec![pt(2f64.powi(41), 0.0); 6], k.clone(), p)), "NP-02");
}

#[test]
fn the_result_survives_a_transform_and_stays_closed() {
    let (p, cps, k) = probe();
    let un = BSpline::from_periodic(p, cps, k).unwrap();
    let m = Transform2::rotate(0.7).unwrap().then(&Transform2::translate(5.0, -2.0).unwrap()).unwrap();
    let (c, _) = un.curve.transform(&m).unwrap();
    match c {
        Curve::BSpline(b) => assert!(b.is_declared_closed()),
        other => panic!("a spline stays a spline, got {other:?}"),
    }
}
