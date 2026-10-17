//! Exact evaluation, derivatives, analytic bounds and exact transforms for all nine kinds (P3.1).
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::TAU;
use pvec_geometry::*;

// How closely a central finite difference is expected to agree with the analytic derivative. These
// bound the truncation error of the difference quotient itself, they are not geometry tolerances.
const FD_LOOSE: f64 = 1.0 / 10_000.0;
const FD2_LOOSE: f64 = 1.0 / 1_000.0;

#[test]
fn all_nine_kinds_evaluate_inside_their_bounds() {
    let mut kinds = std::collections::HashSet::new();
    for (name, c) in samples() {
        kinds.insert(c.kind());
        let Ok(b) = c.bounds() else {
            assert_eq!(c.kind(), CurveKind::Line, "{name}: only the unbounded line has no bounds");
            continue;
        };
        let slack = P.tol(b.diagonal(), b.magnitude());
        for t in params(&c, 200) {
            let q = c.eval(t).unwrap_or_else(|e| panic!("{name}: eval({t}) -> {e:?}"));
            assert!(
                q.x >= b.min().x - slack
                    && q.x <= b.max().x + slack
                    && q.y >= b.min().y - slack
                    && q.y <= b.max().y + slack,
                "{name}: {q:?} outside {b:?}"
            );
        }
    }
    assert_eq!(kinds.len(), CurveKind::ALL.len());
    assert_eq!(CurveKind::ALL.len(), 9);
}

#[test]
fn the_domain_of_each_kind_follows_the_profile() {
    let by_name: std::collections::HashMap<_, _> = samples().into_iter().collect();
    let d = |n: &str| by_name[n].domain();
    assert_eq!((d("circle").lo(), d("circle").hi()), (0.0, TAU));
    assert_eq!((d("ellipse").lo(), d("ellipse").hi()), (0.0, TAU));
    assert_eq!((d("polyline").lo(), d("polyline").hi()), (0.0, 3.0));
    assert_eq!((d("composite").lo(), d("composite").hi()), (0.0, 3.0));
    assert_eq!((d("bezier").lo(), d("bezier").hi()), (0.0, 1.0));
    assert_eq!((d("bspline").lo(), d("bspline").hi()), (0.0, 1.0));
    assert_eq!(d("trimmed-line").lo(), 0.0);
    assert!((d("trimmed-line").hi() - 5.0).abs() <= P.tol(5.0, 5.0));
}

#[test]
fn a_parameter_outside_the_domain_is_refused_and_a_bad_parameter_is_np01() {
    let by_name: std::collections::HashMap<_, _> = samples().into_iter().collect();
    for n in ["polyline", "composite", "bezier", "bspline", "rational-bspline", "trimmed-arc", "trimmed-line"] {
        let c = &by_name[n];
        let d = c.domain();
        assert_eq!(c.eval(d.hi() + d.width()).unwrap_err().rule(), "NP-13", "{n}");
        assert_eq!(c.eval(d.lo() - d.width()).unwrap_err().rule(), "NP-13", "{n}");
        assert_eq!(c.eval(f64::NAN).unwrap_err().rule(), "NP-01", "{n}");
        assert_eq!(c.eval(f64::INFINITY).unwrap_err().rule(), "NP-01", "{n}");
        assert!(c.derivative(f64::NAN, 1, Side::Right).is_err(), "{n}");
    }
}

#[test]
fn derivatives_match_central_differences() {
    for (name, c) in samples() {
        let d = c.domain();
        let ts: Vec<f64> = if c.kind() == CurveKind::Line {
            vec![-2.0, 0.0, 1.5]
        } else {
            (1..23).map(|i| d.lo() + d.width() * i as f64 / 23.0).collect()
        };
        for t in ts {
            let h = d.width().min(100.0) / 1_000_000.0; // finite-difference step, not a tolerance
            let (Ok(a), Ok(b)) = (c.eval(t - h), c.eval(t + h)) else { continue };
            let d1 = c.derivative(t, 1, Side::Right).unwrap();
            let num = (b.x - a.x, b.y - a.y);
            let num = (num.0 / (2.0 * h), num.1 / (2.0 * h));
            let scale = d1.x.abs().max(d1.y.abs()).max(1.0);
            let agrees = (d1.x - num.0).abs() <= FD_LOOSE * scale && (d1.y - num.1).abs() <= FD_LOOSE * scale;
            if !agrees {
                // A break inside the stencil is legitimate for polylines and composites only.
                assert!(matches!(name, "polyline" | "composite"), "{name} t={t}: analytic {d1:?} vs {num:?}");
            }
        }
    }
}

#[test]
fn second_derivative_of_the_rational_spline_matches_differences() {
    let (_, c) = samples().into_iter().find(|(n, _)| *n == "rational-bspline").unwrap();
    for i in 1..17 {
        let t = i as f64 / 17.0;
        let h = 1.0 / 10_000.0; // finite-difference step, not a tolerance
        let (a, m, b) = (c.eval(t - h).unwrap(), c.eval(t).unwrap(), c.eval(t + h).unwrap());
        let num = ((a.x - 2.0 * m.x + b.x) / (h * h), (a.y - 2.0 * m.y + b.y) / (h * h));
        let d2 = c.derivative(t, 2, Side::Right).unwrap();
        assert!((d2.x - num.0).abs() < FD2_LOOSE * (1.0 + num.0.abs()), "t={t} {d2:?} {num:?}");
        assert!((d2.y - num.1).abs() < FD2_LOOSE * (1.0 + num.1.abs()), "t={t} {d2:?} {num:?}");
    }
}

#[test]
fn derivatives_of_any_order_never_panic_and_vanish_past_the_degree() {
    for (name, c) in samples() {
        let t = if c.kind() == CurveKind::Line { 0.5 } else { c.domain().lo() + 0.37 * c.domain().width() };
        for order in [0usize, 1, 2, 3, 4, 5, 17, 1000, usize::MAX] {
            let r = c.derivative(t, order, Side::Right);
            if let Ok(d) = r {
                assert!(d.x.is_finite() && d.y.is_finite(), "{name} order {order}");
            }
        }
    }
    let (_, bez) = samples().into_iter().find(|(n, _)| *n == "bezier").unwrap();
    assert_eq!(bez.derivative(0.4, 4, Side::Both).unwrap(), Vector2::new(0.0, 0.0));
    let (_, bsp) = samples().into_iter().find(|(n, _)| *n == "bspline").unwrap();
    assert_eq!(bsp.derivative(0.3, 4, Side::Both).unwrap(), Vector2::new(0.0, 0.0));
}

#[test]
fn a_conic_derivative_of_order_four_equals_the_curve_relative_to_the_centre() {
    let c = Circle::new(place(1.0, 2.0, 1.0, 0.0), 3.0).unwrap();
    let t = 0.7;
    let p = c.eval(t).unwrap();
    let d4 = c.derivative(t, 4, Side::Both).unwrap();
    assert!(near(pt(d4.x + 1.0, d4.y + 2.0), p, 3.0));
}

// ---------------------------------------------------------------- analytic bounds

#[test]
fn full_ellipse_bounds_match_the_closed_form() {
    let e = Ellipse::new(place(10.0, -4.0, 3.0, 1.0), 5.0, 2.0).unwrap();
    let phi = (1.0f64).atan2(3.0);
    let hx = ((5.0 * phi.cos()).powi(2) + (2.0 * phi.sin()).powi(2)).sqrt();
    let hy = ((5.0 * phi.sin()).powi(2) + (2.0 * phi.cos()).powi(2)).sqrt();
    let b = e.bounds().unwrap();
    assert!(near(b.min(), pt(10.0 - hx, -4.0 - hy), 10.0), "{b:?}");
    assert!(near(b.max(), pt(10.0 + hx, -4.0 + hy), 10.0), "{b:?}");
}

#[test]
fn trimmed_arc_bounds_match_the_closed_form() {
    // Circle r = 2 at (1, 1), arc from 30 to 200 degrees counter-clockwise. x is smallest at 180
    // degrees (inside the arc), largest at the 30 degree end; y is largest at 90 degrees, smallest
    // at the 200 degree end.
    let c = Circle::new(place(1.0, 1.0, 1.0, 0.0), 2.0).unwrap();
    let a = Trimmed::new(Curve::Circle(c), 30f64.to_radians(), 200f64.to_radians(), true).unwrap();
    let b = a.bounds().unwrap();
    let want_min = pt(1.0 - 2.0, 1.0 + 2.0 * 200f64.to_radians().sin());
    let want_max = pt(1.0 + 2.0 * 30f64.to_radians().cos(), 1.0 + 2.0);
    assert!(near(b.min(), want_min, 4.0) && near(b.max(), want_max, 4.0), "{b:?}");
}

#[test]
fn a_reversed_arc_has_the_same_bounds_as_the_forward_arc() {
    let c = || Curve::Circle(Circle::new(place(0.0, 0.0, 1.0, 1.0), 2.5).unwrap());
    let fwd = Trimmed::new(c(), 1.0, 4.0, true).unwrap().bounds().unwrap();
    let rev = Trimmed::new(c(), 4.0, 1.0, false).unwrap().bounds().unwrap();
    assert!(near(fwd.min(), rev.min(), 5.0) && near(fwd.max(), rev.max(), 5.0));
}

#[test]
fn sampled_bounds_would_fail_the_analytic_check() {
    // The guard for "analytic, not sampled": 64 samples of a rotated ellipse miss the extreme by far
    // more than the policy tolerance, so substituting sampling turns the closed-form test red.
    let e = Ellipse::new(place(10.0, -4.0, 3.0, 1.0), 5.0, 2.0).unwrap();
    let exact = e.bounds().unwrap();
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for i in 0..64 {
        let q = e.eval(TAU * i as f64 / 64.0).unwrap();
        max_x = max_x.max(q.x);
        max_y = max_y.max(q.y);
    }
    assert!(!near(pt(max_x, max_y), exact.max(), 10.0), "sampling happened to hit the extreme");
    assert!(max_x < exact.max().x, "a sample cannot exceed the analytic maximum");
}

#[test]
fn small_degree_bezier_bounds_are_tight_and_higher_degree_are_enclosures() {
    let cubic = Bezier::new(vec![pt(0., 0.), pt(1., 3.), pt(4., 3.), pt(5., 0.)]).unwrap();
    let b = cubic.bounds().unwrap();
    // y peaks at t = 1/2 with value 3 * 0.75 = 2.25 (symmetric hump).
    assert!(near(b.max(), pt(5.0, 2.25), 5.0), "{b:?}");
    let quartic = Bezier::new(vec![pt(0., 0.), pt(1., 3.), pt(4., 3.), pt(5., 0.), pt(6., 2.)]).unwrap();
    let hull = quartic.bounds().unwrap();
    for i in 0..=100 {
        let q = quartic.eval(i as f64 / 100.0).unwrap();
        assert!(q.y <= hull.max().y + P.abs && q.y >= hull.min().y - P.abs);
    }
}

#[test]
fn a_line_is_unbounded_and_a_trimmed_line_is_not() {
    let l = Line::new(pt(0.0, 0.0), v(1.0, 1.0)).unwrap();
    assert_eq!(l.bounds().unwrap_err().rule(), "PROFILE-4");
    let t = Trimmed::new(Curve::Line(l), 0.0, 2.0f64.sqrt(), true).unwrap();
    let b = t.bounds().unwrap();
    assert!(near(b.max(), pt(1.0, 1.0), 2.0));
}

// ---------------------------------------------------------------- exact transforms

#[test]
fn transform_is_exact_on_every_kind_under_every_map() {
    for (cn, curve) in samples() {
        for (mn, m) in transforms() {
            let (img, pm) = curve.transform(&m).unwrap_or_else(|e| panic!("{cn} under {mn}: {e:?}"));
            let nd = img.domain();
            for t in params(&curve, 40) {
                let p = curve.eval(t).unwrap();
                let want = m.apply_point(p);
                let t2 = pm.apply(t).clamp(nd.lo(), nd.hi());
                let got = img.eval(t2).unwrap_or_else(|e| panic!("{cn} under {mn}: eval({t2}) {e:?}"));
                let scale = want.x.abs().max(want.y.abs()).max(1.0);
                assert!(near(got, want, scale), "{cn} under {mn} at t={t}: {got:?} vs {want:?}");
            }
        }
    }
}

#[test]
fn circle_under_non_similarity_becomes_an_ellipse_and_under_similarity_stays_a_circle() {
    let c = Curve::Circle(Circle::new(place(1.0, 1.0, 1.0, 0.0), 2.0).unwrap());
    let (e, _) = c.transform(&Transform2::new([3., 0., 0., 0.5, 0., 0.]).unwrap()).unwrap();
    assert_eq!(e.kind(), CurveKind::Ellipse);
    let (r, _) = c.transform(&Transform2::new([0., -2., 2., 0., 0., 0.]).unwrap()).unwrap();
    assert_eq!(r.kind(), CurveKind::Circle);
    let (f, _) = c.transform(&Transform2::new([1., 0., 0., -1., 0., 0.]).unwrap()).unwrap();
    assert_eq!(f.kind(), CurveKind::Circle);
}

#[test]
fn transformed_bounds_enclose_the_transformed_curve() {
    for (cn, curve) in samples() {
        for (mn, m) in transforms() {
            let (img, _) = curve.transform(&m).unwrap();
            let Ok(b) = img.bounds() else { continue };
            let slack = P.tol(b.diagonal(), b.magnitude());
            for t in params(&img, 60) {
                let q = img.eval(t).unwrap();
                assert!(
                    q.x >= b.min().x - slack
                        && q.x <= b.max().x + slack
                        && q.y >= b.min().y - slack
                        && q.y <= b.max().y + slack,
                    "{cn} under {mn}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------- one-sided rule at joints

#[test]
fn two_sided_first_derivative_is_refused_at_a_composite_joint_and_polyline_vertex() {
    let by_name: std::collections::HashMap<_, _> = samples().into_iter().collect();
    let comp = &by_name["composite"];
    assert_eq!(comp.derivative(1.0, 1, Side::Both).unwrap_err().rule(), "NP-16");
    assert!(comp.derivative(1.0, 1, Side::Left).is_ok() && comp.derivative(1.0, 1, Side::Right).is_ok());
    let pl = &by_name["polyline"];
    assert_eq!(pl.derivative(1.0, 1, Side::Both).unwrap_err().rule(), "NP-16");
    let (l, r) = (pl.derivative(1.0, 1, Side::Left).unwrap(), pl.derivative(1.0, 1, Side::Right).unwrap());
    assert_eq!((l, r), (v(2.0, 1.0), v(1.0, 2.0)));
    // the ends of the domain have only one side
    assert!(pl.derivative(0.0, 1, Side::Left).is_err());
    assert!(pl.derivative(3.0, 1, Side::Right).is_err());
}

// ---------------------------------------------------------------- relationships between kinds

#[test]
fn a_bezier_equals_the_clamped_single_span_bspline() {
    let cps = vec![pt(0., 0.), pt(1., 3.), pt(4., 3.), pt(5., 0.)];
    let b = Bezier::new(cps.clone()).unwrap();
    let s = BSpline::new(3, cps, vec![0., 0., 0., 0., 1., 1., 1., 1.]).unwrap();
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        assert!(near(b.eval(t).unwrap(), s.eval(t).unwrap(), 5.0));
        let (db, ds) = (b.derivative(t, 1, Side::Both).unwrap(), s.derivative(t, 1, Side::Both).unwrap());
        assert!(near(pt(db.x, db.y), pt(ds.x, ds.y), 20.0));
    }
}

#[test]
fn a_rational_circle_matches_the_circle() {
    // The standard 9-point rational quadratic circle, radius 2.
    let w = 0.5f64.sqrt();
    let r = 2.0;
    let cps = vec![
        pt(r, 0.), pt(r, r), pt(0., r), pt(-r, r), pt(-r, 0.), pt(-r, -r), pt(0., -r), pt(r, -r), pt(r, 0.),
    ];
    let weights = vec![1., w, 1., w, 1., w, 1., w, 1.];
    let knots = vec![0., 0., 0., 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1., 1., 1.];
    let nurbs = RationalBSpline::new(2, cps, knots, weights).unwrap();
    for i in 0..=200 {
        let q = nurbs.eval(i as f64 / 200.0).unwrap();
        assert!(P.near(q.x.hypot(q.y), r, r), "radius {} at sample {i}", q.x.hypot(q.y));
    }
}

#[test]
fn a_rational_spline_with_equal_weights_is_the_same_curve_as_the_polynomial_one() {
    let cps = vec![pt(0., 0.), pt(1., 2.), pt(2., 3.), pt(3., 0.), pt(4., -1.)];
    let knots = vec![0., 0., 0., 0.3, 0.6, 1., 1., 1.];
    let poly = BSpline::new(2, cps.clone(), knots.clone()).unwrap();
    let rat = RationalBSpline::new(2, cps, knots, vec![2.5; 5]).unwrap();
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        assert!(near(poly.eval(t).unwrap(), rat.eval(t).unwrap(), 4.0));
    }
}

#[test]
fn full_multiplicity_knots_give_distinct_one_sided_derivatives() {
    let s = BSpline::new(
        3,
        vec![pt(0., 0.), pt(1., 2.), pt(2., 3.), pt(3., 0.), pt(4., -1.), pt(5., -2.), pt(6., 0.)],
        vec![0., 0., 0., 0., 0.5, 0.5, 0.5, 1., 1., 1., 1.],
    )
    .unwrap();
    let (l, r) = (s.derivative(0.5, 1, Side::Left).unwrap(), s.derivative(0.5, 1, Side::Right).unwrap());
    assert!(near(pt(l.x, l.y), pt(6.0, -18.0), 18.0) && near(pt(r.x, r.y), pt(6.0, -6.0), 18.0));
    assert_eq!(s.derivative(0.5, 1, Side::Both).unwrap_err().rule(), "NP-16");
    // order zero is the value, and exists two-sided
    assert!(s.derivative(0.5, 0, Side::Both).is_ok());
}

#[test]
fn trimmed_ends_are_the_basis_points_at_the_trim_parameters() {
    let c = Circle::new(place(1.0, -2.0, 1.0, 1.0), 3.0).unwrap();
    let basis = Curve::Circle(c);
    for (t1, t2, sense) in [(0.3, 2.9, true), (4.0, 1.0, false), (5.5, 1.0, true), (-1.0, 1.0, true), (1.0, -3.0, false)] {
        let tc = Trimmed::new(basis.clone(), t1, t2, sense).unwrap();
        let d = tc.domain();
        assert!(near(tc.eval(d.lo()).unwrap(), basis.eval(t1).unwrap(), 3.0), "start {t1} {t2} {sense}");
        assert!(near(tc.eval(d.hi()).unwrap(), basis.eval(t2).unwrap(), 3.0), "end {t1} {t2} {sense}");
    }
}

#[test]
fn a_full_period_trim_is_a_closed_arc() {
    let basis = Curve::Circle(Circle::new(place(0.0, 0.0, 1.0, 0.0), 1.0).unwrap());
    let full = Trimmed::new(basis, 0.5, 0.5 + TAU, true).unwrap();
    assert_eq!(full.trim().span, TAU);
    let d = full.domain();
    assert!(near(full.eval(d.lo()).unwrap(), full.eval(d.hi()).unwrap(), 1.0));
}

#[test]
fn composite_segments_map_linearly_onto_unit_intervals() {
    let (_, comp) = samples().into_iter().find(|(n, _)| *n == "composite").unwrap();
    let Curve::Composite(c) = &comp else { unreachable!() };
    for (k, seg) in c.segments().iter().enumerate() {
        let d = seg.domain();
        let mid = comp.eval(k as f64 + 0.5).unwrap();
        let want = seg.eval(d.lo() + 0.5 * d.width()).unwrap();
        assert!(near(mid, want, 4.0), "segment {k}");
    }
}

#[test]
fn values_are_deterministic() {
    for (name, c) in samples() {
        let d = c.domain();
        let t = if c.kind() == CurveKind::Line { 0.25 } else { d.lo() + 0.31 * d.width() };
        let (a, b) = (c.eval(t).unwrap(), c.eval(t).unwrap());
        assert_eq!((a.x.to_bits(), a.y.to_bits()), (b.x.to_bits(), b.y.to_bits()), "{name}");
    }
}
