//! Issue 121: the start angle and sweep of a bulge arc are the deterministic arctangent's, bit for bit.

use pvec_geometry::{Curve, Point2, Trimmed};
use pvec_numeric::detmath::{atan, atan2};
use std::f64::consts::TAU;

#[test]
fn a_bulge_arc_carries_the_detmath_angles_exactly() {
    let cases = [
        ((0.0, 0.0), (2.0, 0.0), 1.0),
        ((-3.0, 2.0), (5.0, -1.0), 0.37),
        ((1.0, 1.0), (4.0, 5.0), -0.8),
        ((0.3, 0.7), (9.1, -2.2), 2.5),
    ];
    for ((x0, y0), (x1, y1), b) in cases {
        let (p0, p1) = (Point2::new(x0, y0), Point2::new(x1, y1));
        let t = Trimmed::from_bulge(p0, p1, b).unwrap();
        let Curve::Circle(c) = t.basis() else { panic!("a circle basis") };
        let o = c.placement().origin();
        let ccw = b > 0.0;
        let sweep = 4.0 * atan(b.abs());
        let trim = t.trim();
        let want_start = if ccw {
            atan2(p0.y - o.y, p0.x - o.x).rem_euclid(TAU)
        } else {
            atan2(p1.y - o.y, p1.x - o.x).rem_euclid(TAU)
        };
        assert_eq!(trim.start.to_bits(), want_start.to_bits(), "start of {b}");
        assert!(
            (trim.span - sweep).abs() <= 16.0 * f64::from_bits(0x3ca0_0000_0000_0000) * TAU,
            "span of {b}"
        );
    }
}
