//! Cutting a sub-range out of a spline by knot insertion (pvec#134).

use pvec_geometry::{BSpline, Bezier, Curve, Curve2d, Point2, Trimmed};

fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn spline() -> BSpline {
    let cps = vec![pt(0.0, 0.0), pt(1.0, 3.0), pt(3.0, 4.0), pt(5.0, 1.0), pt(8.0, 2.0), pt(9.0, 6.0)];
    BSpline::new(3, cps, vec![0.0, 0.0, 0.0, 0.0, 0.25, 0.75, 1.0, 1.0, 1.0, 1.0]).unwrap()
}

#[test]
fn a_sub_range_traces_the_same_points_over_the_same_parameters() {
    let s = spline();
    let r = s.sub_range(0.1, 0.6).unwrap();
    assert_eq!((r.domain().lo(), r.domain().hi()), (0.1, 0.6));
    for i in 0..=100 {
        let t = 0.1 + 0.5 * i as f64 / 100.0;
        assert!(s.eval(t).unwrap().distance(r.eval(t).unwrap()) <= 1e-12, "t = {t}");
    }
}

#[test]
fn a_sub_range_is_clamped_at_both_ends() {
    let r = spline().sub_range(0.3, 0.9).unwrap();
    let k = r.knots();
    assert_eq!(k.iter().filter(|&&v| v == 0.3).count(), 4);
    assert_eq!(k.iter().filter(|&&v| v == 0.9).count(), 4);
    assert_eq!(r.control_points().len(), k.len() - 4);
}

#[test]
fn the_whole_domain_is_the_spline_bit_for_bit() {
    let s = spline();
    assert_eq!(s.sub_range(0.0, 1.0).unwrap(), s);
}

#[test]
fn a_collapsed_or_outside_range_is_refused() {
    let s = spline();
    assert!(s.sub_range(0.5, 0.5).is_err());
    assert!(s.sub_range(0.6, 0.5).is_err());
    assert!(s.sub_range(-0.5, 0.5).is_err());
    assert!(s.sub_range(0.5, 1.5).is_err());
    assert!(s.sub_range(f64::NAN, 0.5).is_err());
}

#[test]
fn an_unclamped_spline_is_cut_to_a_clamped_one() {
    let cps = vec![pt(0.0, 0.0), pt(1.0, 2.0), pt(2.0, -1.0), pt(3.0, 3.0), pt(4.0, 0.0)];
    let s = BSpline::new(2, cps, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]).unwrap();
    let r = s.sub_range(2.5, 4.0).unwrap();
    for i in 0..=50 {
        let t = 2.5 + 1.5 * i as f64 / 50.0;
        assert!(s.eval(t).unwrap().distance(r.eval(t).unwrap()) <= 1e-12, "t = {t}");
    }
}

#[test]
fn a_bezier_as_a_bspline_is_the_same_curve() {
    let b = Bezier::new(vec![pt(0.0, 0.0), pt(2.0, 8.0), pt(6.0, 8.0), pt(9.0, 1.0)]).unwrap();
    let s = b.as_bspline().unwrap();
    for i in 0..=20 {
        let t = i as f64 / 20.0;
        assert!(b.eval(t).unwrap().distance(s.eval(t).unwrap()) <= 1e-12);
    }
}

#[test]
fn a_trimmed_spline_as_a_bspline_runs_over_its_own_parameter_both_ways() {
    for (t1, t2, sense) in [(0.1, 0.6, true), (0.6, 0.1, false)] {
        let t = Trimmed::new(Curve::BSpline(spline()), t1, t2, sense).unwrap();
        let s = t.as_bspline().unwrap();
        assert_eq!(s.domain().lo(), 0.0);
        assert_eq!(s.domain().hi(), t.domain().hi());
        for i in 0..=100 {
            let tau = t.domain().hi() * i as f64 / 100.0;
            assert!(t.eval(tau).unwrap().distance(s.eval(tau).unwrap()) <= 1e-12, "tau = {tau}");
        }
    }
}

#[test]
fn only_a_polynomial_spline_basis_has_a_bspline_form() {
    let line = pvec_geometry::Line::new(pt(0.0, 0.0), pvec_geometry::Vector2::new(1.0, 0.0)).unwrap();
    let t = Trimmed::new(Curve::Line(line), 0.0, 1.0, true).unwrap();
    assert!(t.as_bspline().is_err());
}
