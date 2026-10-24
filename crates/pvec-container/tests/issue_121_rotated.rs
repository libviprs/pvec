//! Issue 121: the production writer stores a rotated circle or ellipse frame and says which numbers it
//! derived. The angle comes from the deterministic arctangent, so the bytes are the same on every
//! machine. The expected files are written through the hot-block calls with the angles spelled out.

mod support;

use pvec_container::Bounds;
use pvec_container::writer::{ContainerWriter, EntityMeta};
use pvec_geometry::{Circle, Curve, Ellipse, Placement2, Point2, Trimmed, Vector2};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, TAU};
use support::*;

fn writer() -> ContainerWriter {
    ContainerWriter::new(minimal().config.clone())
}

fn frame(x: f64, y: f64) -> Placement2 {
    Placement2::new(Point2::new(2.0, 3.0), Vector2::new(x, y)).unwrap()
}

fn square(c: [f64; 2], r: f64) -> Bounds {
    Bounds { min: [c[0] - r, c[1] - r], max: [c[0] + r, c[1] + r] }
}

fn meta(b: Bounds) -> EntityMeta {
    EntityMeta { id: eid(0), style: 2, cell: 0, bounds: b }
}

fn via_curve(c: &Curve) -> Vec<u8> {
    let mut w = writer();
    w.push_curve(eid(0), 2, 0, c).expect("a rotated frame is stored");
    w.finish().unwrap()
}

#[test]
fn a_rotated_ellipse_is_stored_with_the_rotation_of_its_axis() {
    // (1, 1), (0, 1) and (-1, 0) have angles that are exact: pi/4, pi/2 and pi
    for (axis, rotation) in [((1.0, 1.0), FRAC_PI_4), ((0.0, 2.0), FRAC_PI_2), ((-1.0, 0.0), PI)] {
        let e = Curve::Ellipse(Ellipse::new(frame(axis.0, axis.1), 6.0, 2.0).unwrap());
        let mut w = writer();
        // a rotated ellipse lies inside the circle of its larger radius: the box is conservative
        w.push_ellipse_arc(meta(square([2.0, 3.0], 6.0)), [2.0, 3.0], [6.0, 2.0], [rotation, 0.0, TAU])
            .unwrap();
        assert_eq!(via_curve(&e), w.finish().unwrap(), "axis {axis:?}");
    }
}

#[test]
fn a_rotated_elliptical_arc_keeps_the_trim_and_the_reversed_sweep() {
    let basis = Curve::Ellipse(Ellipse::new(frame(0.0, 1.0), 3.0, 1.0).unwrap());
    let arc = Curve::Trimmed(Trimmed::new(basis, 0.5, 2.0, true).unwrap());
    let mut w = writer();
    w.push_ellipse_arc(meta(square([2.0, 3.0], 3.0)), [2.0, 3.0], [3.0, 1.0], [FRAC_PI_2, 0.5, 1.5]).unwrap();
    assert_eq!(via_curve(&arc), w.finish().unwrap());
}

#[test]
fn a_rotated_circle_is_stored_as_the_arc_starting_on_its_axis() {
    let c = Circle::new(frame(0.0, 1.0), 5.0).unwrap();
    let full = Curve::Circle(c);
    let mut w = writer();
    w.push_circle_arc(meta(square([2.0, 3.0], 5.0)), [2.0, 3.0], 5.0, [FRAC_PI_2, TAU]).unwrap();
    assert_eq!(via_curve(&full), w.finish().unwrap());

    let arc = Curve::Trimmed(Trimmed::new(Curve::Circle(c), 0.5, 2.0, true).unwrap());
    let mut w = writer();
    w.push_circle_arc(meta(square([2.0, 3.0], 5.0)), [2.0, 3.0], 5.0, [FRAC_PI_2 + 0.5, 1.5]).unwrap();
    assert_eq!(via_curve(&arc), w.finish().unwrap());
}

#[test]
fn what_was_derived_is_reported_and_what_was_authored_is_not() {
    let mut w = writer();
    w.push_curve(eid(0), 0, 0, &Curve::Ellipse(Ellipse::new(frame(1.0, 0.0), 6.0, 2.0).unwrap())).unwrap();
    assert!(w.derived().is_empty(), "an axis-aligned frame derives nothing");
    w.push_curve(eid(1), 0, 0, &Curve::Ellipse(Ellipse::new(frame(1.0, 1.0), 6.0, 2.0).unwrap())).unwrap();
    let d = w.derived();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].entity, eid(1));
    assert!(d[0].bound >= f64::EPSILON / 2.0 * FRAC_PI_4, "the bound covers one ulp of the angle");
    assert!(d[0].what.contains("atan2"));
}
