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
mod exact;
use common::*;
use exact::*;
use pvec_geometry::*;
use pvec_numeric::MAX_CONTROL_POINTS;

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
    let cps = vec![pt(3., 0.), pt(1.5, 2.6), pt(-1.5, 2.6), pt(-3., 0.), pt(-1.5, -2.6), pt(1.5, -2.6)];
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
        // The seam knot is one knot listed twice, so a knot equal to it stays equal a period away.
        let v = if lap == 1 && k[r] == k[0] {
            k[n]
        } else if lap == -1 && k[r] == k[n] {
            k[0]
        } else {
            k[r] + lap as f64 * t
        };
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

/// Exact (rational) reference curves, none of which goes through f64 arithmetic.
struct Exact {
    clamped: (Vec<(Rat, Rat)>, Vec<Rat>),
    wrapped_f64: (Vec<(Rat, Rat)>, Vec<Rat>),
    periodic: (Vec<(Rat, Rat)>, Vec<Rat>),
}

fn rat_pts(c: &[Point2]) -> Vec<(Rat, Rat)> {
    c.iter().map(|q| (Rat::from_f64(q.x), Rat::from_f64(q.y))).collect()
}

fn rat_knots(k: &[f64]) -> Vec<Rat> {
    k.iter().map(|&x| Rat::from_f64(x)).collect()
}

/// The periodic curve with exact knots `k[j] + m T` (T the exact period), no f64 rounding of the extension.
fn exact_periodic(p: usize, cps: &[Point2], k: &[f64]) -> (Vec<(Rat, Rat)>, Vec<Rat>) {
    let n = cps.len();
    let t = Rat::from_f64(k[n]).sub(&Rat::from_f64(k[0]));
    let pts = rat_pts(&(0..n + p).map(|i| cps[i % n]).collect::<Vec<_>>());
    let mut u = Vec::new();
    for j in 0..(n + 2 * p + 1) {
        let q = j as isize - p as isize;
        let (lap, r) = (q.div_euclid(n as isize), q.rem_euclid(n as isize) as usize);
        let mut v = Rat::from_f64(k[r]);
        for _ in 0..lap.unsigned_abs() {
            v = if lap > 0 { v.add(&t) } else { v.sub(&t) };
        }
        u.push(v);
    }
    (pts, u)
}

fn exact_of(p: usize, cps: &[Point2], k: &[f64], un: &Unwrapped) -> Exact {
    let w = wrapped(p, cps, k);
    Exact {
        clamped: (rat_pts(un.curve.control_points()), rat_knots(un.curve.knots())),
        wrapped_f64: (rat_pts(w.control_points()), rat_knots(w.knots())),
        periodic: exact_periodic(p, cps, k),
    }
}

/// Smallest number of halvings of each bound that the exact deviation still survives, over `m + 1`
/// parameters. Panics if either bound is exceeded: no slack anywhere.
fn exact_check(p: usize, cps: &[Point2], k: &[f64], m: usize) -> (u32, u32) {
    let un = BSpline::from_periodic(p, cps.to_vec(), k.to_vec()).expect("unwraps");
    let e = exact_of(p, cps, k, &un);
    let (lo, hi) = (k[0], k[cps.len()]);
    let (mut h1, mut h2) = (64, 64);
    for i in 0..=m {
        let t = if i == m { hi } else { lo + (hi - lo) * (i as f64 / m as f64) };
        let tr = Rat::from_f64(t);
        let cc = deboor(p, &e.clamped.0, &e.clamped.1, &tr);
        let cw = deboor(p, &e.wrapped_f64.0, &e.wrapped_f64.1, &tr);
        let cp = deboor(p, &e.periodic.0, &e.periodic.1, &tr);
        h1 = h1.min(halvings(&cc, &cw, un.deviation_bound).unwrap_or_else(|| {
            panic!("p={p} n={} t={t}: deviation from the f64-formed wrapped curve exceeds {}", cps.len(), un.deviation_bound)
        }));
        h2 = h2.min(halvings(&cc, &cp, un.periodic_deviation_bound).unwrap_or_else(|| {
            panic!("p={p} n={} t={t}: deviation from the periodic curve exceeds {}", cps.len(), un.periodic_deviation_bound)
        }));
    }
    assert!(un.periodic_deviation_bound >= un.deviation_bound);
    (h1, h2)
}

/// The cyclic multiplicity of the seam knot: the first and the last knot are one knot.
fn seam_mult(k: &[f64]) -> usize {
    let n = k.len() - 1;
    k.iter().take_while(|&&v| v == k[0]).count() + k.iter().rev().take_while(|&&v| v == k[n]).count() - 1
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
fn the_probe_locus_is_inside_both_stated_bounds_in_exact_arithmetic_with_no_slack() {
    let (p, cps, k) = probe();
    let (h1, h2) = exact_check(p, &cps, &k, 24);
    println!("probe: bound 1 survives {h1} halvings, bound 2 survives {h2}");
    // the bounds are a few roundoffs of the largest coordinate, far inside the NP-04 tolerance
    let un = BSpline::from_periodic(p, cps.clone(), k).unwrap();
    assert!(un.deviation_bound > 0.0 && un.deviation_bound < 1000.0 * U * max_abs(&cps));
    assert!(un.periodic_deviation_bound < 0.000001 * max_abs(&cps), "a usable bound on a well conditioned probe");
}

#[test]
fn the_stated_bound_is_the_derived_formula_not_a_loose_number() {
    // 15 r u m, r = 2 (p + 1 - M) insertions for a seam of multiplicity M, m the largest coordinate;
    // the point count of the result follows from M as well: n + p + 1 - M
    let (p, cps, k) = probe();
    let un = BSpline::from_periodic(p, cps.clone(), k.clone()).unwrap();
    let (m, r) = (max_abs(&cps), 2 * (p + 1 - seam_mult(&k)));
    assert_eq!(un.deviation_bound, 15.0 * r as f64 * U * m);
    assert_eq!(un.curve.control_points().len(), cps.len() + p + 1 - seam_mult(&k));
    // and with a seam of multiplicity 3 on a cubic (the seam test's knots) fewer insertions are needed
    let k2 = vec![0., 0., 0.2, 0.4, 0.5, 0.6, 0.8, 0.9, 1., 1.];
    let cps2: Vec<Point2> = (0..9).map(|i| pt(i as f64, (i * i % 7) as f64)).collect();
    let un2 = BSpline::from_periodic(3, cps2.clone(), k2.clone()).unwrap();
    assert_eq!(seam_mult(&k2), 3);
    assert_eq!(un2.deviation_bound, 15.0 * 2.0 * U * max_abs(&cps2));
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
    exact_check(3, &cps, &k, 40);
    // a break is a corner: the curve passes through a control point at 0.4 and at the seam
    let un = BSpline::from_periodic(3, cps, k).unwrap();
    assert!(un.curve.derivative(0.4, 1, Side::Both).is_err(), "no two-sided derivative at a break");
    assert!(un.curve.derivative(0.4, 1, Side::Left).is_ok());
}

/// A random periodic spline, interior knots of multiplicity up to two (never at the seam), coordinates
/// and knot offsets spread over many orders of magnitude when `big` is set.
fn random_case(r: &mut Rng, big: bool) -> (usize, Vec<Point2>, Vec<f64>) {
    let p = 1 + r.below(6);
    let n = p + 1 + r.below(8);
    let scale = if big { 10f64.powi(r.below(13) as i32) } else { 50.0 };
    let cps: Vec<Point2> = (0..n).map(|_| pt(r.range(-1., 1.) * scale, r.range(-1., 1.) * scale)).collect();
    let base = if big { r.range(-1000000.0, 1000000.0) } else { r.range(-5., 5.) };
    let mut k = vec![base];
    for i in 0..n {
        let last = *k.last().unwrap();
        let fresh = k.len() < 2 || k[k.len() - 1] > k[k.len() - 2]; // multiplicity stays at two
        let repeat = p >= 2 && i > 0 && i + 1 < n && fresh && r.below(5) == 0;
        k.push(last + if repeat { 0.0 } else { r.range(0.01, 1.0) });
    }
    (p, cps, k)
}

#[test]
fn many_random_periodic_splines_agree_with_the_wrapped_form() {
    let mut r = Rng(0x0093_5eed);
    for case in 0..300 {
        let (p, cps, k) = random_case(&mut r, false);
        let un = BSpline::from_periodic(p, cps.clone(), k.clone())
            .unwrap_or_else(|e| panic!("case {case} p={p} k={k:?}: {e}"));
        let w = wrapped(p, &cps, &k);
        let d = un.curve.domain();
        for i in 0..=200 {
            let t = d.lo() + (d.hi() - d.lo()) * (i as f64 / 200.0);
            let e = dist(un.curve.eval(t).unwrap(), w.eval(t).unwrap());
            // the evaluator's own rounding is in this float comparison, so it only has to land inside the
            // NP-04 relative tolerance; the exact tests below hold the stated bounds to account
            assert!(e <= 5000.0 * U * max_abs(&cps), "case {case} p={p} t={t} e={e}");
        }
    }
}

#[test]
fn the_exact_oracle_holds_both_bounds_up_to_the_coordinate_limit_and_large_knot_offsets() {
    let mut r = Rng(0x0093_b16);
    let (mut h1, mut h2) = (64u32, 64u32);
    for case in 0..120 {
        let (p, cps, k) = random_case(&mut r, case % 4 != 0);
        let (a, b) = exact_check(p, &cps, &k, 9);
        h1 = h1.min(a);
        h2 = h2.min(b);
    }
    println!("closest approach to bound 1: {h1} halvings, to bound 2: {h2} halvings");
    // not vacuous: somewhere in the suite the exact deviation is within 2^7 of bound 1, so a bound divided
    // by 128 would have failed (the stated bound is not a number picked to be safe)
    assert!(h1 <= 6, "bound 1 is never within 2^7 of the measured deviation: {h1}");
}

#[test]
fn a_seam_knot_repeated_with_an_inexact_period_is_matched_bit_for_bit() {
    // 0.7 - (0.7 - 0.1) is not 0.1 in f64, so the knot a period before 0.7 would be one ulp off the seam
    // knot 0.1 and the pair would be refused as ambiguous, were it not carried over bit for bit
    let k = vec![0.1, 0.1, 0.3, 0.45, 0.6, 0.7, 0.7];
    assert_ne!(k[6] - (k[6] - k[0]), k[0], "the period is inexact for these knots");
    let (_, cps, _) = probe();
    let un = BSpline::from_periodic(3, cps.clone(), k.clone()).expect("a seam of multiplicity 3 on a cubic is legal");
    assert_eq!(un.curve.knots()[..4], [0.1; 4]);
    assert_eq!(un.curve.knots()[un.curve.knots().len() - 4..], [0.7; 4]);
    exact_check(3, &cps, &k, 30);
}

#[test]
fn an_extension_the_f64_knots_cannot_resolve_is_refused_not_given_a_meaningless_bound() {
    // knots near 1000000000.0 spaced by 0.000003, a few ulps of their own magnitude: adding the period rounds each
    // extended knot by more than a quarter of the gap, so the f64 curve is not the periodic one
    let k: Vec<f64> = (0..=6).map(|i| 1000000000.0 + i as f64 * 0.000003).collect();
    let (_, cps, _) = probe();
    assert_eq!(rule_of(BSpline::from_periodic(3, cps.clone(), k)), "NP-14");
    // the same spacing at a modest offset is fine
    let k: Vec<f64> = (0..=6).map(|i| 1.0 + i as f64 * 0.000003).collect();
    assert!(BSpline::from_periodic(3, cps, k).is_ok());
}

#[test]
fn the_point_count_limit_is_checked_on_the_wrapped_count_before_any_work() {
    let k: Vec<f64> = (0..=MAX_CONTROL_POINTS).map(|i| i as f64).collect();
    let cps = vec![pt(1., 1.); MAX_CONTROL_POINTS];
    // n is at the limit, n + p is over it: refused as NP-18 up front
    assert_eq!(rule_of(BSpline::from_periodic(3, cps, k)), "NP-18");
    // n + p exactly at the limit is accepted (the result has n + p + 1 - M points)
    let n = MAX_CONTROL_POINTS - 3;
    let cps: Vec<Point2> = (0..n).map(|i| pt((i % 17) as f64, (i % 13) as f64)).collect();
    let k: Vec<f64> = (0..=n).map(|i| i as f64).collect();
    assert!(BSpline::from_periodic(3, cps, k).is_ok());
}

#[test]
fn unique_points_with_n_plus_p_plus_1_knots_and_a_periodic_flag_are_not_an_unwrap_input() {
    // the count is that of a plain B-spline, so it is one, and an open one: the flag is the source's claim
    // and nothing in PVec can read it. Only a closed claim is checked (NP-20), and it fails here.
    let (p, cps, _) = probe();
    let uniform: Vec<f64> = (0..10).map(|i| i as f64).collect();
    let open = BSpline::new(p, cps, uniform).expect("legal as an open curve");
    assert!(!open.is_declared_closed());
    assert_eq!(rule_of(open.declare_closed()), "NP-20");
}

#[test]
fn the_wrapped_unclamped_layout_is_already_a_legal_bspline() {
    // A producer that stores the wrapped control points and the extended knots (n + p points,
    // n + 2p + 1 knots) needs nothing new: BSpline::new takes it and the ends meet.
    let (p, cps, k) = probe();
    let w = wrapped(p, &cps, &k).declare_closed().unwrap();
    assert!(w.is_declared_closed());
    exact_check(p, &cps, &k, 12);
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
    amb[3] = amb[2] + 100.0 * U;
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
