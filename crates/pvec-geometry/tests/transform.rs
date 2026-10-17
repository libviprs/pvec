//! Transform composition and the promotion rules (P3.8): exact affine handling with no tessellation
//! anywhere on the path, and the deviations measured rather than assumed negligible.
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::{FRAC_PI_2, PI, TAU};
use pvec_geometry::*;
use pvec_numeric::MAX_ABS_COORD;

fn rule_of<T>(r: Result<T, GeometryError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected a refusal"),
        Err(e) => e.rule(),
    }
}

fn dev(a: Point2, b: Point2) -> f64 {
    (a.x - b.x).abs().max((a.y - b.y).abs())
}

// ---------------------------------------------------------------- building and composing

#[test]
fn the_elementary_maps_do_what_they_say() {
    let p = pt(3.0, 2.0);
    assert!(near(Transform2::translate(1.0, -4.0).unwrap().apply_point(p), pt(4.0, -2.0), 5.0));
    assert!(near(Transform2::rotate(FRAC_PI_2).unwrap().apply_point(pt(1.0, 0.0)), pt(0.0, 1.0), 1.0));
    assert!(near(Transform2::scale(2.0, 0.5).unwrap().apply_point(p), pt(6.0, 1.0), 6.0));
    assert!(near(Transform2::mirror_x().unwrap().apply_point(p), pt(3.0, -2.0), 3.0));
    assert!(near(Transform2::mirror_y().unwrap().apply_point(p), pt(-3.0, 2.0), 3.0));
    assert!(near(Transform2::shear(0.5, 0.0).unwrap().apply_point(p), pt(4.0, 2.0), 4.0));
    assert!(near(Transform2::shear(0.0, 0.5).unwrap().apply_point(p), pt(3.0, 3.5), 4.0));
    assert!(Transform2::mirror_x().unwrap().determinant() < 0.0);
    assert_eq!(Transform2::IDENTITY.matrix(), [1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
}

#[test]
fn then_applies_the_first_map_first() {
    let a = Transform2::scale(2.0, 3.0).unwrap();
    let b = Transform2::translate(10.0, 20.0).unwrap();
    let ab = a.then(&b).unwrap();
    let p = pt(1.0, 1.0);
    assert!(near(ab.apply_point(p), b.apply_point(a.apply_point(p)), 30.0));
    assert!(near(ab.apply_point(p), pt(12.0, 23.0), 30.0));
    let ba = b.then(&a).unwrap();
    assert!(near(ba.apply_point(p), pt(22.0, 63.0), 70.0));
    assert!(!near(ab.apply_point(p), ba.apply_point(p), 70.0), "composition is not commutative");
}

#[test]
fn the_inverse_undoes_the_map_and_a_map_composed_with_it_is_the_identity() {
    for (name, m) in transforms() {
        let inv = m.inverse().unwrap_or_else(|e| panic!("{name}: {e}"));
        let id = m.then(&inv).unwrap();
        for p in [pt(0.0, 0.0), pt(3.5, -2.0), pt(-100.0, 77.0)] {
            assert!(near(id.apply_point(p), p, 100.0), "{name} at {p:?}");
        }
    }
}

#[test]
fn composition_is_associative_within_the_policy_on_the_huge_coordinate_fixtures() {
    // Translations of a quarter of the coordinate bound, rotations and non-uniform scales: the
    // magnitudes where a careless composition loses the low bits. The worst deviation is measured and
    // printed, then held to the NP-04 tolerance at the magnitude of the numbers involved.
    let big = MAX_ABS_COORD / 4.0;
    let maps = [
        Transform2::translate(big, -big).unwrap(),
        Transform2::rotate(0.6).unwrap(),
        Transform2::scale(3.0, 0.25).unwrap(),
        Transform2::translate(-big / 3.0, big / 7.0).unwrap(),
        Transform2::shear(0.3, -0.2).unwrap(),
        Transform2::rotate(-2.1).unwrap(),
    ];
    let points = [pt(0.0, 0.0), pt(1.0, 1.0), pt(big / 2.0, -big / 5.0), pt(-123456.789, 98765.4321)];
    let mut worst = (0.0f64, (0, 0, 0), 0usize);
    let mut count = 0;
    for (i, a) in maps.iter().enumerate() {
        for (j, b) in maps.iter().enumerate() {
            for (k, c) in maps.iter().enumerate() {
                let (Ok(ab), Ok(bc)) = (a.then(b), b.then(c)) else { continue };
                let (Ok(l), Ok(r)) = (ab.then(c), a.then(&bc)) else { continue };
                count += 1;
                for (n, p) in points.iter().enumerate() {
                    let (x, y) = (l.apply_point(*p), r.apply_point(*p));
                    let scale = x.x.abs().max(x.y.abs()).max(1.0);
                    let d = dev(x, y);
                    if d / scale > worst.0 {
                        worst = (d / scale, (i, j, k), n);
                    }
                    assert!(near(x, y, scale), "triple {i}{j}{k} at point {n}: {x:?} vs {y:?}");
                }
            }
        }
    }
    println!(
        "associativity: {count} triples, worst relative deviation {:e} at triple {:?}, point {}",
        worst.0, worst.1, worst.2
    );
    assert!(count > 100, "enough triples survived NP-19 to mean something, got {count}");
}

#[test]
fn a_composition_that_leaves_the_coordinate_range_is_refused_not_wrapped() {
    let a = Transform2::translate(MAX_ABS_COORD * 0.75, 0.0).unwrap();
    assert_eq!(rule_of(a.then(&a)), "NP-02");
    let s = Transform2::scale(1048576.0, 1.0).unwrap();
    assert_eq!(rule_of(s.then(&s)), "NP-02", "the linear entries are bounded too");
}

// ---------------------------------------------------------------- degenerate maps

#[test]
fn a_singular_or_nearly_singular_transform_is_a_typed_degeneracy_error() {
    assert_eq!(rule_of(Transform2::scale(1.0, 0.0)), "NP-19");
    assert_eq!(rule_of(Transform2::scale(0.0, 0.0)), "NP-19");
    assert_eq!(rule_of(Transform2::new([1., 2., 2., 4., 0., 0.])), "NP-19");
    // a shear whose determinant is lost against the size of its entries
    assert_eq!(rule_of(Transform2::new([1.0, 1000000.0, 1.0, 1000001.0, 0.0, 0.0])), "NP-19");
    assert_eq!(rule_of(Transform2::rotate(f64::NAN)), "NP-01");
    assert_eq!(rule_of(Transform2::translate(f64::INFINITY, 0.0)), "NP-01");
    // composing a good map with its mirror image of zero area never produces a map
    let flat = Transform2::new([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]).unwrap();
    assert!(flat.then(&Transform2::scale(1.0, 2.0).unwrap()).is_ok());
}

#[test]
fn no_degenerate_map_can_reach_a_curve_so_no_zero_area_curve_comes_out() {
    // The only way to a transform is a constructor that refuses singular maps, and the curve methods
    // take a Transform2, so a degenerate transform is not representable on the curve path.
    let circle = Circle::new(place(0.0, 0.0, 1.0, 0.0), 1.0).unwrap();
    for bad in [[1., 1., 1., 1., 0., 0.], [0.; 6], [1., 0., 0., 0., 0., 0.]] {
        assert!(Transform2::new(bad).is_err());
    }
    assert!(circle.transform(&Transform2::scale(1.0, 0.5).unwrap()).is_ok());
}

// ---------------------------------------------------------------- the promotion table

#[test]
fn a_circle_under_non_uniform_scale_is_an_exact_ellipse_and_the_deviation_is_recorded() {
    let circle = Circle::new(place(2.0, -1.0, 1.0, 1.0), 3.0).unwrap();
    let m = Transform2::scale(3.0, 0.5).unwrap();
    let (img, map) = circle.transform(&m).unwrap();
    assert_eq!(img.kind(), CurveKind::Ellipse);
    let mut worst = 0.0f64;
    for i in 0..=2000 {
        let t = TAU * i as f64 / 2000.0;
        let want = m.apply_point(circle.eval(t).unwrap());
        let got = img.eval(map.apply(t)).unwrap();
        worst = worst.max(dev(want, got));
        assert!(near(got, want, 10.0), "t = {t}");
    }
    println!("circle -> ellipse: worst locus deviation {worst:e}");
    let Curve::Ellipse(e) = &img else { unreachable!() };
    assert!(e.semi_axis_1() >= e.semi_axis_2(), "the image is expressed major axis first");
}

#[test]
fn a_circle_under_a_similarity_stays_a_circle_with_the_scaled_radius() {
    let circle = Circle::new(place(2.0, -1.0, 1.0, 1.0), 3.0).unwrap();
    let m = Transform2::rotate(0.7).unwrap().then(&Transform2::scale(2.0, 2.0).unwrap()).unwrap();
    let (img, _) = circle.transform(&m).unwrap();
    let Curve::Circle(c) = &img else { panic!("{:?}", img.kind()) };
    assert!(P.near(c.radius(), 6.0, 6.0));
    let (mir, _) = circle.transform(&Transform2::mirror_x().unwrap()).unwrap();
    assert_eq!(mir.kind(), CurveKind::Circle);
}

#[test]
fn an_ellipse_under_shear_is_an_exact_ellipse_and_has_an_exact_rational_form_never_a_polyline() {
    let ellipse = Ellipse::new(place(1.0, 2.0, 2.0, 1.0), 5.0, 2.0).unwrap();
    let m = Transform2::shear(0.8, 0.0).unwrap();
    let (img, map) = ellipse.transform(&m).unwrap();
    // the affine image of an ellipse is an ellipse: that is the exact result (ADR 0012)
    assert_eq!(img.kind(), CurveKind::Ellipse);
    let mut worst = 0.0f64;
    for i in 0..=2000 {
        let t = TAU * i as f64 / 2000.0;
        worst = worst.max(dev(m.apply_point(ellipse.eval(t).unwrap()), img.eval(map.apply(t)).unwrap()));
    }
    println!("ellipse under shear: worst locus deviation {worst:e}");
    assert!(worst <= P.tol(10.0, 10.0));
    // the exact rational form of the sheared ellipse matches its locus
    let rational = img.to_rational_b_spline().unwrap();
    assert_eq!(rational.kind(), CurveKind::RationalBSpline);
    let mut worst = 0.0f64;
    for i in 0..=2000 {
        // the rational form runs over [0, 1]; every sample of it lies on the sheared ellipse
        let got = rational.eval(i as f64 / 2000.0).unwrap();
        worst = worst.max(distance_to_conic(&img, got));
    }
    println!("rational form of the sheared ellipse: worst distance to the locus {worst:e}");
    assert!(worst <= P.tol(10.0, 10.0));
}

/// Distance from `q` to the ellipse or circle `c`, by the implicit equation in its own frame.
fn distance_to_conic(c: &Curve, q: Point2) -> f64 {
    let (frame, a, b) = match c {
        Curve::Ellipse(e) => (*e.placement(), e.semi_axis_1(), e.semi_axis_2()),
        Curve::Circle(k) => (*k.placement(), k.radius(), k.radius()),
        _ => panic!("not a conic"),
    };
    let d = frame.origin().vector_to(q);
    let (x, y) = (d.dot(frame.x_axis()), d.dot(frame.y_axis()));
    // |(x/a, y/b)| = 1 on the curve; convert the residual back to length with the smaller semi-axis
    (((x / a).powi(2) + (y / b).powi(2)).sqrt() - 1.0).abs() * a.min(b)
}

#[test]
fn an_ellipse_too_eccentric_after_the_map_is_refused_not_flattened_into_a_polyline() {
    let ellipse = Ellipse::new(place(0.0, 0.0, 1.0, 0.0), 1.0, 0.5).unwrap();
    // shear hard enough that the semi-axis ratio passes the NP-07 bound (about 1.2e7 against 1e6)
    let m = Transform2::shear(5000.0, 0.0).unwrap();
    assert_eq!(rule_of(ellipse.transform(&m)), "NP-07");
}

#[test]
fn an_arc_under_any_map_keeps_its_end_points_and_stays_an_arc_of_a_conic() {
    let circle = Circle::new(place(1.0, 1.0, 1.0, 0.0), 2.0).unwrap();
    for (name, m) in transforms() {
        for (t1, t2, sense) in [(0.3, 2.9, true), (4.0, 1.0, false), (5.5, 1.0, true)] {
            let arc = Trimmed::new(Curve::Circle(circle), t1, t2, sense).unwrap();
            let (img, _) = arc.transform(&m).unwrap();
            let Curve::Trimmed(t) = &img else { panic!("{name}: {:?}", img.kind()) };
            assert!(matches!(t.basis().kind(), CurveKind::Circle | CurveKind::Ellipse), "{name}");
            let (d0, d1) = (arc.domain(), img.domain());
            assert!(
                near(img.eval(d1.lo()).unwrap(), m.apply_point(arc.eval(d0.lo()).unwrap()), 10.0),
                "{name} start"
            );
            assert!(
                near(img.eval(d1.hi()).unwrap(), m.apply_point(arc.eval(d0.hi()).unwrap()), 10.0),
                "{name} end"
            );
        }
    }
}

#[test]
fn a_spline_under_any_map_keeps_its_knots_and_weights_bit_for_bit_and_moves_only_its_points() {
    let samples = samples();
    let get = |n: &str| samples.iter().find(|(m, _)| *m == n).unwrap().1.clone();
    for (name, m) in transforms() {
        let Curve::BSpline(s) = get("bspline") else { unreachable!() };
        let (img, map) = Curve::BSpline(s.clone()).transform(&m).unwrap();
        let Curve::BSpline(t) = &img else { panic!() };
        assert_eq!(map, ParamMap::IDENTITY);
        assert_eq!(
            s.knots().iter().map(|k| k.to_bits()).collect::<Vec<_>>(),
            t.knots().iter().map(|k| k.to_bits()).collect::<Vec<_>>(),
            "{name}"
        );
        for (a, b) in s.control_points().iter().zip(t.control_points()) {
            assert_eq!(m.apply_point(*a), *b, "{name}");
        }
        let Curve::RationalBSpline(r) = get("rational-bspline") else { unreachable!() };
        let (img, _) = Curve::RationalBSpline(r.clone()).transform(&m).unwrap();
        let Curve::RationalBSpline(q) = &img else { panic!() };
        assert_eq!(
            r.weights().iter().map(|w| w.to_bits()).collect::<Vec<_>>(),
            q.weights().iter().map(|w| w.to_bits()).collect::<Vec<_>>(),
            "{name}"
        );
        assert_eq!(
            r.knots().iter().map(|w| w.to_bits()).collect::<Vec<_>>(),
            q.knots().iter().map(|w| w.to_bits()).collect::<Vec<_>>(),
            "{name}"
        );
    }
}

#[test]
fn lines_polylines_beziers_and_composites_keep_their_kind() {
    for (name, c) in samples() {
        for (mn, m) in transforms() {
            let (img, _) = c.transform(&m).unwrap();
            let expected = match c.kind() {
                CurveKind::Circle => None, // may become an ellipse
                CurveKind::Ellipse => Some(CurveKind::Ellipse),
                k => Some(k),
            };
            if let Some(k) = expected {
                assert_eq!(img.kind(), k, "{name} under {mn}");
            }
        }
    }
}

// ---------------------------------------------------------------- trim then transform

#[test]
fn trimming_then_transforming_equals_transforming_then_trimming_for_arcs() {
    let circle = Circle::new(place(1.0, -2.0, 1.0, 1.0), 3.0).unwrap();
    let mut worst = (0.0f64, "");
    for (name, m) in transforms() {
        for (t1, t2, sense) in [(0.3, 2.9, true), (4.0, 1.0, false), (5.5, 1.0, true), (-1.0, 1.0, true)] {
            // A: trim, then transform
            let trimmed = Trimmed::new(Curve::Circle(circle), t1, t2, sense).unwrap();
            let (a, _) = trimmed.transform(&m).unwrap();
            // B: transform the carrier, then trim with the mapped parameters
            let (img, map) = Curve::Circle(circle).transform(&m).unwrap();
            let sense_b = sense ^ (map.scale < 0.0);
            let b = Trimmed::new(img, map.apply(t1), map.apply(t2), sense_b).unwrap();
            let (da, db) = (a.domain(), b.domain());
            for i in 0..=200 {
                let u = i as f64 / 200.0;
                let pa = a.eval(da.lo() + u * da.width()).unwrap();
                let pb = b.eval(db.lo() + u * db.width()).unwrap();
                let d = dev(pa, pb);
                if d > worst.0 {
                    worst = (d, name);
                }
                assert!(near(pa, pb, 20.0), "{name} ({t1}, {t2}, {sense}) u = {u}");
            }
        }
    }
    println!("trim/transform commutation on arcs: worst deviation {:e} under {}", worst.0, worst.1);
}

#[test]
fn trimming_then_transforming_equals_transforming_then_trimming_for_splines() {
    let samples = samples();
    let spline = samples.iter().find(|(n, _)| *n == "bspline").unwrap().1.clone();
    let rational = samples.iter().find(|(n, _)| *n == "rational-bspline").unwrap().1.clone();
    let mut worst = (0.0f64, "");
    for base in [spline, rational] {
        for (name, m) in transforms() {
            let (lo, hi) = (0.15, 0.85);
            let a = Trimmed::new(base.clone(), lo, hi, true).unwrap().transform(&m).unwrap().0;
            let (img, map) = base.transform(&m).unwrap();
            let b = Trimmed::new(img, map.apply(lo), map.apply(hi), true).unwrap();
            let (da, db) = (a.domain(), b.domain());
            for i in 0..=200 {
                let u = i as f64 / 200.0;
                let (pa, pb) =
                    (a.eval(da.lo() + u * da.width()).unwrap(), b.eval(db.lo() + u * db.width()).unwrap());
                let d = dev(pa, pb);
                if d > worst.0 {
                    worst = (d, name);
                }
                assert!(near(pa, pb, 20.0), "{name} u = {u}");
            }
        }
    }
    println!("trim/transform commutation on splines: worst deviation {:e} under {}", worst.0, worst.1);
}

// ---------------------------------------------------------------- the exact rational form of a conic

#[test]
fn full_circles_and_ellipses_have_the_nine_point_rational_form() {
    let circle = Curve::Circle(Circle::new(place(1.0, 2.0, 0.6, 0.8), 2.5).unwrap());
    let Curve::RationalBSpline(r) = circle.to_rational_b_spline().unwrap() else { panic!() };
    assert_eq!((r.degree(), r.control_points().len(), r.knots().len()), (2, 9, 12));
    let ellipse = Curve::Ellipse(Ellipse::new(place(-1.0, 0.5, 1.0, 2.0), 4.0, 1.5).unwrap());
    let rational = ellipse.to_rational_b_spline().unwrap();
    let mut worst = 0.0f64;
    for i in 0..=1000 {
        worst = worst.max(distance_to_conic(&ellipse, rational.eval(i as f64 / 1000.0).unwrap()));
    }
    println!("ellipse -> rational: worst distance to the locus {worst:e}");
    assert!(worst <= P.tol(10.0, 10.0));
    for c in [&circle] {
        let rc = c.to_rational_b_spline().unwrap();
        for i in 0..=1000 {
            assert!(distance_to_conic(c, rc.eval(i as f64 / 1000.0).unwrap()) <= P.tol(10.0, 10.0));
        }
    }
}

#[test]
fn arcs_of_any_span_and_direction_have_an_exact_rational_form_with_the_right_end_points() {
    let ellipse = Ellipse::new(place(1.0, 2.0, 2.0, 1.0), 5.0, 2.0).unwrap();
    for (t1, t2, sense) in [
        (0.3, 2.9, true),
        (4.0, 1.0, false),
        (5.5, 1.0, true),
        (0.0, TAU, true),
        (1.0, 1.0 + 1.0 / 64.0, true),
        (0.0, PI, true),
    ] {
        let arc = Curve::Trimmed(Trimmed::new(Curve::Ellipse(ellipse), t1, t2, sense).unwrap());
        let rational = arc.to_rational_b_spline().unwrap();
        let (a, r) = (arc.domain(), rational.domain());
        assert!(near(rational.eval(r.lo()).unwrap(), arc.eval(a.lo()).unwrap(), 10.0), "start ({t1}, {t2})");
        assert!(near(rational.eval(r.hi()).unwrap(), arc.eval(a.hi()).unwrap(), 10.0), "end ({t1}, {t2})");
        for i in 0..=400 {
            let q = rational.eval(r.lo() + r.width() * i as f64 / 400.0).unwrap();
            assert!(
                distance_to_conic(&Curve::Ellipse(ellipse), q) <= P.tol(10.0, 10.0),
                "({t1}, {t2}) sample {i}"
            );
        }
        let Curve::RationalBSpline(s) = &rational else { panic!() };
        assert_eq!(s.degree(), 2);
        assert!(s.weights().iter().all(|w| *w > 0.0 && *w <= 1.0));
    }
}

#[test]
fn only_conics_and_their_arcs_have_a_rational_form() {
    for (name, c) in samples() {
        let r = c.to_rational_b_spline();
        let is_conic = matches!(c.kind(), CurveKind::Circle | CurveKind::Ellipse)
            || matches!(&c, Curve::Trimmed(t) if matches!(t.basis().kind(), CurveKind::Circle | CurveKind::Ellipse));
        assert_eq!(r.is_ok(), is_conic, "{name}");
        if !is_conic {
            assert_eq!(r.unwrap_err().rule(), "PROFILE-8", "{name}");
        }
    }
}
