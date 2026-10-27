//! Issue P8.6 (pvec#65): the rounding allowance of a Tier 0 bound is taken from the preview's own extent.
//!
//! P8.5 found `huge-coordinate-1e5` at 0.9419 coverage on every arm: 12,010 curves had no preview at all.
//! The stored bound adds an allowance for `f32` rounding, and it scaled that allowance by the absolute
//! coordinate (about 6.5 million there), which came to 0.65 world units, five times the whole budget of
//! 0.114. The vertices are stored as `f32` offsets from an `f64` origin, so the value that is rounded is the
//! offset and the allowance has to follow it. The bound is still measured after quantisation and is never
//! below what an independent sampling finds, which is what the second test holds it to.
#![allow(missing_docs)]

use pvec_entity::EntityId;
use pvec_geometry::{Circle, Curve, Curve2d, Ellipse, Placement2, Point2, Polyline, Trimmed, Vector2};
use pvec_stream::{InfeasibleReason, Tier0Builder, Tier0Params, build_preview, measure_deviation};

/// The budget `huge-coordinate-1e5` is written with: 0.125 px of a 1,920 px screen across its extent.
const BUDGET: f64 = 0.114;
/// UTM-scale offsets with millimetre detail, as the huge-coordinate class has.
const FAR: (f64, f64) = (6_479_916.464, 5_899_924.166);

fn place(x: f64, y: f64, angle: f64) -> Placement2 {
    Placement2::new(Point2::new(x, y), Vector2::new(angle.cos(), angle.sin())).unwrap()
}

fn ellipse_arc(x: f64, y: f64) -> Curve {
    let e = Ellipse::new(place(x, y, 0.4), 60.0, 25.0).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), 0.3, 4.6, true).unwrap())
}

fn circle_arc(x: f64, y: f64) -> Curve {
    let c = Circle::new(place(x, y, 0.0), 40.0).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Circle(c), 0.2, 2.9, true).unwrap())
}

fn wavy_polyline(x: f64, y: f64) -> Curve {
    let pts = (0..200)
        .map(|i| Point2::new(x + 0.5 * f64::from(i), y + 3.0 * (0.21 * f64::from(i)).sin()))
        .collect();
    Curve::Polyline(Polyline::new(pts).unwrap())
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

#[test]
fn a_curve_far_from_the_origin_gets_a_preview_when_its_own_extent_is_small() {
    for (name, curve) in [
        ("ellipse arc", ellipse_arc(FAR.0, FAR.1)),
        ("circle arc", circle_arc(FAR.0, FAR.1)),
        ("polyline", wavy_polyline(FAR.0, FAR.1)),
    ] {
        let made = build_preview(id(0), &curve, &Tier0Params { max_error: BUDGET }).unwrap();
        let preview = made.unwrap_or_else(|why| panic!("{name} at {FAR:?} has no preview: {why:?}"));
        assert!(
            f64::from(preview.error_bound()) <= BUDGET,
            "{name}: bound {} over the budget",
            preview.error_bound()
        );
    }
}

#[test]
fn the_stored_bound_is_still_never_below_an_independent_measurement() {
    for curve in [ellipse_arc(FAR.0, FAR.1), circle_arc(FAR.0, FAR.1), wavy_polyline(FAR.0, FAR.1)] {
        let preview = build_preview(id(1), &curve, &Tier0Params { max_error: BUDGET }).unwrap().unwrap();
        let measured = measure_deviation(&curve, &preview).unwrap();
        assert!(
            f64::from(preview.error_bound()) >= measured,
            "bound {} below the measured deviation {measured}",
            preview.error_bound()
        );
        // and the preview's own vertices are the ones the bound was measured on
        let dom = curve.domain();
        let end = curve.eval(dom.hi()).unwrap();
        let last = *preview.points().last().unwrap();
        assert!((last[0] - end.x).hypot(last[1] - end.y) <= f64::from(preview.error_bound()));
    }
}

#[test]
fn a_plan_of_far_curves_has_no_infeasible_entity() {
    let curves: Vec<Curve> = (0..40)
        .map(|i| {
            let (dx, dy) = (f64::from(i % 8) * 300.0, f64::from(i / 8) * 300.0);
            match i % 3 {
                0 => ellipse_arc(FAR.0 + dx, FAR.1 + dy),
                1 => circle_arc(FAR.0 + dx, FAR.1 + dy),
                _ => wavy_polyline(FAR.0 + dx, FAR.1 + dy),
            }
        })
        .collect();
    let mut plan = Tier0Builder::new(curves.len() as u32, Tier0Params { max_error: BUDGET }).unwrap();
    for (i, c) in curves.iter().enumerate() {
        plan.add_curve(id(i as u32), c).unwrap();
    }
    let plan = plan.finish().unwrap();
    let why: Vec<InfeasibleReason> = plan.infeasible().iter().map(|i| i.reason).collect();
    assert!(why.is_empty(), "{} entities have no preview: {why:?}", why.len());
    assert_eq!(plan.exact_count() + plan.previews().len(), curves.len());
}
