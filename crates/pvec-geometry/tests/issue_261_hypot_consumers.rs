//! Issue 261: the lengths that end up in a stored artifact come from `detmath::hypot`, not the C
//! library: `Vector2::length`, `Point2::distance`, `Bounds::diagonal` and the direction normalised
//! by `Policy::normalise_direction`.

use pvec_geometry::{Bounds, Point2, Vector2};
use pvec_numeric::detmath::hypot;

const PAIRS: [(f64, f64); 6] = [
    (0.1, 0.2),
    (123.456, -789.012),
    (0.001, 0.00777),
    (33000.0, 44000.1),
    (-0.57735, 0.81650),
    (25000000.0, -1100000.0),
];

#[test]
fn vector_length_is_the_deterministic_hypot() {
    for (x, y) in PAIRS {
        assert_eq!(Vector2::new(x, y).length().to_bits(), hypot(x, y).to_bits(), "({x}, {y})");
    }
}

#[test]
fn point_distance_is_the_deterministic_hypot() {
    for (x, y) in PAIRS {
        let d = Point2::new(1.5, -2.5).distance(Point2::new(1.5 + x, -2.5 + y));
        let (dx, dy) = ((1.5 + x) - 1.5, (-2.5 + y) - -2.5);
        assert_eq!(d.to_bits(), hypot(dx, dy).to_bits(), "({x}, {y})");
    }
}

#[test]
fn bounds_diagonal_is_the_deterministic_hypot() {
    for (x, y) in PAIRS {
        let b = Bounds::new(Point2::new(0.0, 0.0), Point2::new(x.abs(), y.abs())).unwrap();
        assert_eq!(b.diagonal().to_bits(), hypot(x.abs(), y.abs()).to_bits(), "({x}, {y})");
    }
}
