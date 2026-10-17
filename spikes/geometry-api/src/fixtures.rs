//! One sample of every curve kind, shared by the tests and the benchmarks.

use crate::*;
use core::f64::consts::FRAC_PI_2;

/// A named sample of each of the nine kinds (two extra trimmed variants, a reversed ellipse arc
/// and a trimmed line segment).
pub fn samples() -> Vec<(&'static str, Curve)> {
    let circle = Circle::new([1.0, -2.0], [1.0, 1.0], 3.0).unwrap();
    let ellipse = Ellipse::new([0.5, 0.5], [2.0, 1.0], 4.0, 1.5).unwrap();
    let arc = Trimmed::new(Curve::Circle(circle), 0.3, 2.9, true).unwrap();
    let arc_rev = Trimmed::new(Curve::Ellipse(ellipse), 4.0, 1.0, false).unwrap();
    let seg = Trimmed::new(Curve::Line(Line::new([0.0, 0.0], [3.0, 4.0]).unwrap()), 0.0, 5.0, true).unwrap();
    let bez = Bezier::new(vec![[0., 0.], [1., 3.], [4., 3.], [5., 0.]]).unwrap();
    let bsp = BSpline::new(
        3,
        vec![[0., 0.], [1., 2.], [2., 3.], [3., 0.], [4., -1.], [5., 2.]],
        vec![0., 0., 0., 0., 0.4, 0.7, 1., 1., 1., 1.],
    )
    .unwrap();
    let rat = RationalBSpline::new(
        2,
        vec![[0., 0.], [1., 2.], [3., 2.], [4., 0.]],
        vec![0., 0., 0., 0.5, 1., 1., 1.],
        vec![1., 0.5, 2., 1.],
    )
    .unwrap();
    let poly = Polyline::new(vec![[0., 0.], [2., 1.], [3., 3.], [1., 4.]]).unwrap();
    // A composite: line segment, arc, Bezier, joined end to start.
    let a = Trimmed::new(Curve::Line(Line::new([0.0, 0.0], [1.0, 0.0]).unwrap()), 0.0, 2.0, true).unwrap();
    let c = Circle::new([2.0, 1.0], [1.0, 0.0], 1.0).unwrap();
    let b = Trimmed::new(Curve::Circle(c), -FRAC_PI_2, FRAC_PI_2, true).unwrap();
    let z = Bezier::new(vec![[2.0, 2.0], [1.0, 3.0], [0.0, 3.0], [-1.0, 2.0]]).unwrap();
    let comp = Composite::new(vec![Curve::Trimmed(a), Curve::Trimmed(b), Curve::Bezier(z)]).unwrap();
    vec![
        ("line", Curve::Line(Line::new([1.0, 2.0], [1.0, -1.0]).unwrap())),
        ("circle", Curve::Circle(circle)),
        ("ellipse", Curve::Ellipse(ellipse)),
        ("polyline", Curve::Polyline(poly)),
        ("trimmed-arc", Curve::Trimmed(arc)),
        ("trimmed-ellipse-reversed", Curve::Trimmed(arc_rev)),
        ("trimmed-line", Curve::Trimmed(seg)),
        ("composite", Curve::Composite(comp)),
        ("bezier", Curve::Bezier(bez)),
        ("bspline", Curve::BSpline(bsp)),
        ("rational-bspline", Curve::RationalBSpline(rat)),
    ]
}
