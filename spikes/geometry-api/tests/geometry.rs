//! P1.3 acceptance checks: every curve kind evaluates, bounds are analytic, transforms are exact,
//! the numeric-policy probes pass, and no serialization type leaks into the API.
#![allow(clippy::needless_range_loop)]

use core::f64::consts::{PI, TAU};
use geometry_api_spike::fixtures::samples;
use geometry_api_spike::*;

#[path = "../../../tests/pathological/probes.rs"]
mod probes;
use probes::{Built, Evaluator, ProbeCurve, probes, run_probe};

const P: Policy = Policy::V1;

// How closely a central finite difference is expected to agree with the analytic derivative. These
// bound the truncation error of the difference quotient itself, they are not geometry tolerances.
const FD_LOOSE: f64 = 1.0 / 10_000.0;
const FD2_LOOSE: f64 = 1.0 / 1_000.0;

fn pt_near(a: Pt, b: Pt, scale: f64) -> bool {
    P.near_point(a, b, scale)
}

// ---------------------------------------------------------------- the probes, against a real evaluator

struct Spike;
struct Wrapped(Curve);

fn lower(e: Error) -> NumericError {
    match e {
        Error::Numeric(n) => n,
        Error::Domain => NumericError::TrimOutsideDomain,
        Error::Unbounded => NumericError::EmptyDomain,
    }
}

fn build(c: &ProbeCurve) -> Result<Curve, Error> {
    Ok(match c {
        ProbeCurve::Line { origin, dir } => Curve::Line(Line::new(*origin, *dir)?),
        ProbeCurve::Circle { center, radius } => Curve::Circle(Circle::new(*center, [1.0, 0.0], *radius)?),
        ProbeCurve::Trimmed { basis, t1, t2, sense } => {
            Curve::Trimmed(Trimmed::new(build(basis)?, *t1, *t2, *sense)?)
        }
        ProbeCurve::BSpline { degree, cps, weights: None, knots } => {
            Curve::BSpline(BSpline::new(*degree, cps.clone(), knots.clone())?)
        }
        ProbeCurve::BSpline { degree, cps, weights: Some(w), knots } => {
            Curve::RationalBSpline(RationalBSpline::new(*degree, cps.clone(), knots.clone(), w.clone())?)
        }
    })
}

impl Evaluator for Spike {
    fn build(&self, c: &ProbeCurve) -> Result<Box<dyn Built>, NumericError> {
        build(c).map(|c| Box::new(Wrapped(c)) as Box<dyn Built>).map_err(lower)
    }
}

impl Built for Wrapped {
    fn domain(&self) -> (f64, f64) {
        self.0.domain()
    }
    fn eval(&self, t: f64) -> Result<[f64; 2], NumericError> {
        self.0.eval(t).map_err(lower)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<[f64; 2], NumericError> {
        self.0.derivative(t, order, side).map_err(lower)
    }
}

#[test]
fn every_pathological_probe_passes_against_the_prototype() {
    let failures: Vec<String> = probes().iter().filter_map(|p| run_probe(&Spike, p).err()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// ---------------------------------------------------------------- sample curves, all nine kinds

fn ts(c: &Curve, n: usize) -> Vec<f64> {
    let (lo, hi) = c.domain();
    // Stay clear of the very ends so central differences do not leave the domain.
    (1..n).map(|i| lo + (hi - lo) * (i as f64) / (n as f64)).collect()
}

#[test]
fn all_nine_kinds_evaluate_inside_their_bounds() {
    let mut kinds = std::collections::HashSet::new();
    for (name, c) in samples() {
        kinds.insert(std::mem::discriminant(&c));
        let (lo, hi) = c.domain();
        let Ok(b) = c.bounds() else {
            assert_eq!(name, "line", "{name}: only the unbounded line has no bounds");
            continue;
        };
        for i in 0..=200 {
            let t = lo + (hi - lo) * i as f64 / 200.0;
            let q = c.eval(t).unwrap_or_else(|e| panic!("{name}: eval({t}) -> {e:?}"));
            let slack = P.tol(b.diagonal(), b.magnitude());
            assert!(
                q[0] >= b.min[0] - slack
                    && q[0] <= b.max[0] + slack
                    && q[1] >= b.min[1] - slack
                    && q[1] <= b.max[1] + slack,
                "{name}: {q:?} outside {b:?}"
            );
        }
    }
    assert_eq!(kinds.len(), 9, "all nine curve kinds are sampled");
}

#[test]
fn derivatives_match_central_differences() {
    for (name, c) in samples() {
        for &t in &ts(&c, 23) {
            let (lo, hi) = c.domain();
            let h = (hi - lo) / 1_000_000.0; // finite-difference step, not a tolerance
            let (a, b) = (c.eval(t - h), c.eval(t + h));
            let (Ok(a), Ok(b)) = (a, b) else { continue };
            // Skip points a finite difference straddles a break across (composite joints, knots).
            let d1 = c.derivative(t, 1, Side::Right).unwrap();
            let num = [(b[0] - a[0]) / (2.0 * h), (b[1] - a[1]) / (2.0 * h)];
            let scale = d1[0].abs().max(d1[1].abs()).max(1.0);
            if (d1[0] - num[0]).abs() > FD_LOOSE * scale || (d1[1] - num[1]).abs() > FD_LOOSE * scale {
                // A break inside the stencil is legitimate for polylines and composites.
                let l = c.derivative(t, 1, Side::Left).unwrap();
                assert!(
                    (l[0] - d1[0]).abs() > P.rel
                        || (l[1] - d1[1]).abs() > P.rel
                        || matches!(name, "polyline" | "composite"),
                    "{name} t={t}: analytic {d1:?} vs numeric {num:?}"
                );
            }
        }
    }
}

#[test]
fn second_derivative_of_rational_spline_matches_differences() {
    let (_, c) = samples().into_iter().find(|(n, _)| *n == "rational-bspline").unwrap();
    for &t in &ts(&c, 17) {
        let h = 1.0 / 10_000.0; // finite-difference step, not a tolerance
        let (a, m, b) = (c.eval(t - h).unwrap(), c.eval(t).unwrap(), c.eval(t + h).unwrap());
        let num = [(a[0] - 2.0 * m[0] + b[0]) / (h * h), (a[1] - 2.0 * m[1] + b[1]) / (h * h)];
        let d2 = c.derivative(t, 2, Side::Right).unwrap();
        assert!((d2[0] - num[0]).abs() < FD2_LOOSE * (1.0 + num[0].abs()), "t={t} {d2:?} {num:?}");
        assert!((d2[1] - num[1]).abs() < FD2_LOOSE * (1.0 + num[1].abs()), "t={t} {d2:?} {num:?}");
    }
}

// ---------------------------------------------------------------- analytic bounds

#[test]
fn full_ellipse_bounds_match_the_closed_form() {
    let e = Ellipse::new([10.0, -4.0], [3.0, 1.0], 5.0, 2.0).unwrap();
    let phi = (1.0f64).atan2(3.0);
    let hx = ((5.0 * phi.cos()).powi(2) + (2.0 * phi.sin()).powi(2)).sqrt();
    let hy = ((5.0 * phi.sin()).powi(2) + (2.0 * phi.cos()).powi(2)).sqrt();
    let b = e.bounds().unwrap();
    assert!(pt_near(b.min, [10.0 - hx, -4.0 - hy], 10.0), "{b:?}");
    assert!(pt_near(b.max, [10.0 + hx, -4.0 + hy], 10.0), "{b:?}");
}

#[test]
fn trimmed_arc_bounds_match_the_closed_form() {
    // Circle r = 2 at (1, 1), arc from 30 to 200 degrees counter-clockwise. x is smallest at 180
    // degrees (inside the arc), largest at the 30 degree end; y is largest at 90 degrees, smallest
    // at the 200 degree end.
    let c = Circle::new([1.0, 1.0], [1.0, 0.0], 2.0).unwrap();
    let a = Trimmed::new(Curve::Circle(c), 30f64.to_radians(), 200f64.to_radians(), true).unwrap();
    let b = a.bounds().unwrap();
    let want_min = [1.0 - 2.0, 1.0 + 2.0 * 200f64.to_radians().sin()];
    let want_max = [1.0 + 2.0 * 30f64.to_radians().cos(), 1.0 + 2.0];
    assert!(pt_near(b.min, want_min, 4.0) && pt_near(b.max, want_max, 4.0), "{b:?}");
}

#[test]
fn sampled_bounds_would_fail_the_analytic_check() {
    // The check that guards "analytic, not sampled": 64 samples of a rotated ellipse miss the extreme
    // by far more than the policy tolerance, so substituting sampling turns the closed-form test red.
    let e = Ellipse::new([10.0, -4.0], [3.0, 1.0], 5.0, 2.0).unwrap();
    let exact = e.bounds().unwrap();
    let mut sampled = Bounds { min: [f64::INFINITY; 2], max: [f64::NEG_INFINITY; 2] };
    for i in 0..64 {
        let q = e.eval(TAU * i as f64 / 64.0).unwrap();
        for k in 0..2 {
            sampled.min[k] = sampled.min[k].min(q[k]);
            sampled.max[k] = sampled.max[k].max(q[k]);
        }
    }
    assert!(!pt_near(sampled.max, exact.max, 10.0), "sampling happened to hit the extreme");
}

#[test]
fn small_degree_bezier_bounds_are_tight_and_higher_degree_are_enclosures() {
    let cubic = Bezier::new(vec![[0., 0.], [1., 3.], [4., 3.], [5., 0.]]).unwrap();
    let b = cubic.bounds().unwrap();
    // y peaks at t = 1/2 with value 3 * 0.75 = 2.25 (symmetric hump).
    assert!(pt_near(b.max, [5.0, 2.25], 5.0), "{b:?}");
    let quartic = Bezier::new(vec![[0., 0.], [1., 3.], [4., 3.], [5., 0.], [6., 2.]]).unwrap();
    let hull = quartic.bounds().unwrap();
    for i in 0..=100 {
        let q = quartic.eval(i as f64 / 100.0).unwrap();
        assert!(q[1] <= hull.max[1] + P.abs && q[1] >= hull.min[1] - P.abs);
    }
}

// ---------------------------------------------------------------- exact transforms

fn maps() -> Vec<(&'static str, Affine)> {
    let (s, c) = 0.6f64.sin_cos();
    vec![
        ("translate", Affine::new([1., 0., 0., 1., 5., -3.]).unwrap()),
        ("rotate+scale", Affine::new([2.0 * c, -2.0 * s, 2.0 * s, 2.0 * c, 1., 2.]).unwrap()),
        ("anisotropic", Affine::new([3., 0., 0., 0.5, 0., 0.]).unwrap()),
        ("shear", Affine::new([1., 0.7, 0., 1., 2., 1.]).unwrap()),
        ("reflect", Affine::new([1., 0., 0., -1., 0., 4.]).unwrap()),
        ("reflect+rotate", Affine::new([c, s, s, -c, -3., 1.]).unwrap()),
    ]
}

#[test]
fn transform_is_exact_on_every_kind_under_every_map() {
    for (cn, curve) in samples() {
        for (mn, m) in maps() {
            let (img, pm) = curve.transform(&m).unwrap_or_else(|e| panic!("{cn} under {mn}: {e:?}"));
            // An unbounded line is sampled near its origin, not across +-2^40.
            let (lo, hi) = if matches!(curve, Curve::Line(_)) { (-10.0, 10.0) } else { curve.domain() };
            for i in 0..=40 {
                let t = lo + (hi - lo) * i as f64 / 40.0;
                let want = m.point(curve.eval(t).unwrap());
                let (nlo, nhi) = img.domain();
                // Conics, lines and trimmed curves carry a parameter map; the rest keep their parameter.
                let t2 = match &curve {
                    Curve::Circle(_) | Curve::Ellipse(_) | Curve::Line(_) => pm.apply(t),
                    Curve::Trimmed(_) => pm.apply(t).clamp(nlo, nhi),
                    _ => t.clamp(nlo, nhi),
                };
                let got = img.eval(t2).unwrap_or_else(|e| panic!("{cn} under {mn}: eval({t2}) {e:?}"));
                let scale = want[0].abs().max(want[1].abs()).max(1.0);
                assert!(pt_near(got, want, scale), "{cn} under {mn} at t={t}: {got:?} vs {want:?}");
            }
        }
    }
}

#[test]
fn circle_under_non_similarity_becomes_an_ellipse_and_under_similarity_stays_a_circle() {
    let c = Curve::Circle(Circle::new([1.0, 1.0], [1.0, 0.0], 2.0).unwrap());
    let (e, _) = c.transform(&Affine::new([3., 0., 0., 0.5, 0., 0.]).unwrap()).unwrap();
    assert!(matches!(e, Curve::Ellipse(_)));
    let (r, _) = c.transform(&Affine::new([0., -2., 2., 0., 0., 0.]).unwrap()).unwrap();
    assert!(matches!(r, Curve::Circle(_)));
    let (f, _) = c.transform(&Affine::new([1., 0., 0., -1., 0., 0.]).unwrap()).unwrap();
    assert!(matches!(f, Curve::Circle(_)));
}

#[test]
fn singular_transform_is_refused() {
    assert!(matches!(
        Affine::new([1., 2., 2., 4., 0., 0.]),
        Err(Error::Numeric(NumericError::SingularTransform))
    ));
}

// ---------------------------------------------------------------- one-sided rule at joints

#[test]
fn two_sided_first_derivative_is_refused_at_a_composite_joint_and_polyline_vertex() {
    let (_, comp) = samples().into_iter().find(|(n, _)| *n == "composite").unwrap();
    assert_eq!(comp.derivative(1.0, 1, Side::Both), Err(Error::Numeric(NumericError::TwoSidedAtBreak)));
    assert!(comp.derivative(1.0, 1, Side::Left).is_ok() && comp.derivative(1.0, 1, Side::Right).is_ok());
    let (_, pl) = samples().into_iter().find(|(n, _)| *n == "polyline").unwrap();
    assert_eq!(pl.derivative(1.0, 1, Side::Both), Err(Error::Numeric(NumericError::TwoSidedAtBreak)));
}

// ---------------------------------------------------------------- storage shapes agree exactly

#[test]
fn owned_and_borrowed_storage_give_bit_identical_results() {
    let cps = vec![[0., 0.], [1., 2.], [2., 3.], [3., 0.], [4., -1.], [5., 2.]];
    let flat: Vec<f64> = cps.iter().flatten().copied().collect();
    let knots = vec![0., 0., 0., 0., 0.4, 0.7, 1., 1., 1., 1.];
    let w = vec![1., 2., 0.5, 1., 3., 1.];
    let owned = BSpline::new(3, cps.clone(), knots.clone()).unwrap();
    let borrowed = BSpline::new(3, Flat(&flat), &knots[..]).unwrap();
    let rown = RationalBSpline::new(3, cps, knots.clone(), w.clone()).unwrap();
    let rbor = RationalBSpline::new(3, Flat(&flat), &knots[..], &w[..]).unwrap();
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        assert_eq!(owned.eval(t).unwrap(), borrowed.eval(t).unwrap());
        assert_eq!(owned.derivative(t, 1, Side::Both), borrowed.derivative(t, 1, Side::Both));
        assert_eq!(rown.eval(t).unwrap(), rbor.eval(t).unwrap());
        assert_eq!(rown.derivative(t, 1, Side::Both).unwrap(), rbor.derivative(t, 1, Side::Both).unwrap());
    }
}

#[test]
fn a_bezier_equals_the_clamped_single_span_bspline() {
    let cps = vec![[0., 0.], [1., 3.], [4., 3.], [5., 0.]];
    let b = Bezier::new(cps.clone()).unwrap();
    let s = BSpline::new(3, cps, vec![0., 0., 0., 0., 1., 1., 1., 1.]).unwrap();
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        assert!(pt_near(b.eval(t).unwrap(), s.eval(t).unwrap(), 5.0));
        assert!(pt_near(
            b.derivative(t, 1, Side::Both).unwrap(),
            s.derivative(t, 1, Side::Both).unwrap(),
            20.0
        ));
    }
}

// ---------------------------------------------------------------- no serialization type in the API

const BANNED: [&str; 8] = ["serde", "flatbuffers", "prost", "hdf5", "bincode", "rkyv", "capnp", "bytemuck"];

#[test]
fn manifest_has_no_serialization_dependency() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let deps = manifest.split("[dependencies]").nth(1).unwrap().split("\n[").next().unwrap();
    for b in BANNED {
        assert!(!deps.contains(b), "dependency on {b}");
    }
}

#[test]
fn no_source_file_names_a_serialization_crate() {
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for b in BANNED {
                assert!(
                    !text.contains(&format!("{b}::")) && !text.contains(&format!("use {b}")),
                    "{path:?} names {b}"
                );
            }
        }
    }
}

#[test]
fn consts_used_by_the_tests_are_what_they_claim() {
    assert!((PI * 2.0 - TAU).abs() == 0.0);
}
