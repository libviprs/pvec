//! Fixtures and helpers shared by the GML export tests (pvec#21).
#![allow(dead_code)]

use core::f64::consts::{FRAC_PI_2, TAU};
use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Curve2d, Ellipse, Line, Placement2, Point2, Polyline,
    RationalBSpline, Transform2, Trimmed, Vector2,
};
use pvec_interchange::gml::*;
use pvec_interchange::{FidelityKind, FidelityRecord};

pub const SRS: &str = "urn:ogc:def:crs:EPSG::25832";

pub fn eid(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

pub fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

pub fn crs(t: Transform2) -> CrsTransform {
    CrsTransform::new(SRS, t).unwrap()
}

pub fn budget(v: f64) -> Budget {
    Budget::new(v).unwrap()
}

pub fn circle(cx: f64, cy: f64, r: f64) -> Curve {
    Curve::Circle(Circle::new(Placement2::new(pt(cx, cy), Vector2::new(1.0, 0.0)).unwrap(), r).unwrap())
}

pub fn trimmed(basis: Curve, t1: f64, t2: f64) -> Curve {
    Curve::Trimmed(Trimmed::new(basis, t1, t2, true).unwrap())
}

pub fn ellipse() -> Curve {
    Curve::Ellipse(Ellipse::from_major_axis(pt(0.0, 0.0), Vector2::new(30.0, 0.0), 0.5).unwrap())
}

pub fn cubic_bezier() -> Curve {
    Curve::Bezier(Bezier::new(vec![pt(0.0, 0.0), pt(2.0, 8.0), pt(6.0, 8.0), pt(9.0, 1.0)]).unwrap())
}

pub fn spline() -> Curve {
    let cps = vec![pt(0.0, 0.0), pt(1.0, 3.0), pt(3.0, 4.0), pt(5.0, 1.0), pt(8.0, 2.0), pt(9.0, 6.0)];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.25, 0.75, 1.0, 1.0, 1.0, 1.0];
    Curve::BSpline(BSpline::new(3, cps, knots).unwrap())
}

pub fn rational(weights: Vec<f64>) -> Curve {
    let cps = vec![pt(0.0, 0.0), pt(2.0, 5.0), pt(5.0, 5.0), pt(7.0, 0.0), pt(9.0, 3.0)];
    let knots = vec![0.0, 0.0, 0.0, 0.4, 0.7, 1.0, 1.0, 1.0];
    Curve::RationalBSpline(RationalBSpline::new(2, cps, knots, weights).unwrap())
}

pub fn composite() -> Curve {
    let arc = trimmed(circle(10.0, 5.0, 5.0), -FRAC_PI_2, 0.0);
    let end = arc.eval(arc.domain().hi()).unwrap();
    let tail = Curve::Bezier(
        Bezier::new(vec![
            end,
            pt(end.x + 2.0, end.y + 3.0),
            pt(end.x + 5.0, end.y + 3.0),
            pt(end.x + 6.0, end.y),
        ])
        .unwrap(),
    );
    Curve::Composite(
        Composite::new(vec![
            Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(10.0, 0.0)]).unwrap()),
            arc,
            tail,
        ])
        .unwrap(),
    )
}

/// Entity ids, in the order the fixtures are exported.
pub const POLYLINE: u32 = 1;
pub const TRIMMED_LINE: u32 = 2;
pub const LINE: u32 = 3;
pub const CIRCLE: u32 = 4;
pub const ARC: u32 = 5;
pub const FULL_CIRCLE: u32 = 6;
pub const ELLIPSE: u32 = 7;
pub const ELLIPSE_ARC: u32 = 8;
pub const BEZIER: u32 = 9;
pub const BSPLINE: u32 = 10;
pub const RATIONAL: u32 = 11;
pub const RATIONAL_EQUAL: u32 = 12;
pub const COMPOSITE: u32 = 13;

pub fn fixtures() -> Vec<(EntityId, Curve)> {
    let line = Line::new(pt(1.0, 1.0), Vector2::new(1.0, 0.0)).unwrap();
    vec![
        (
            eid(POLYLINE),
            Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 5.0)]).unwrap()),
        ),
        (eid(TRIMMED_LINE), trimmed(Curve::Line(line), 0.0, 5.0)),
        (eid(LINE), Curve::Line(line)),
        (eid(CIRCLE), circle(100.0, 200.0, 25.0)),
        (eid(ARC), trimmed(circle(40.0, 40.0, 12.0), 0.3, 2.0)),
        (eid(FULL_CIRCLE), trimmed(circle(5.0, 6.0, 7.0), 0.0, TAU)),
        (eid(ELLIPSE), ellipse()),
        (eid(ELLIPSE_ARC), trimmed(ellipse(), 0.2, 2.5)),
        (eid(BEZIER), cubic_bezier()),
        (eid(BSPLINE), spline()),
        (eid(RATIONAL), rational(vec![1.0, 0.5, 2.0, 1.0, 1.5])),
        (eid(RATIONAL_EQUAL), rational(vec![2.0; 5])),
        (eid(COMPOSITE), composite()),
    ]
}

pub fn run(entities: &[(EntityId, Curve)], t: Transform2, b: f64) -> GmlExport {
    export(entities, &crs(t), budget(b)).unwrap()
}

pub fn records_of(x: &GmlExport, id: u32) -> Vec<&FidelityRecord> {
    x.ledger.records().iter().filter(|r| r.entity_id().index() == id).collect()
}

pub fn kinds_of(x: &GmlExport, id: u32) -> Vec<FidelityKind> {
    records_of(x, id).iter().map(|r| r.fidelity().kind()).collect()
}

pub fn constructs_of(x: &GmlExport, id: u32) -> Vec<String> {
    records_of(x, id).iter().map(|r| r.target_construct().name().to_string()).collect()
}
