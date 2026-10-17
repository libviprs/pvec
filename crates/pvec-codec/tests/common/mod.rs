//! Shared fixtures for the codec tests: sample entities of every basic kind, a field-by-field
//! structural view of a curve (so a failure names the field that moved), and a seeded generator.
#![allow(dead_code, missing_docs)]

use core::f64::consts::{FRAC_PI_2, TAU};
use pvec_entity::EntityId;
use pvec_geometry::*;

pub fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

pub fn v(x: f64, y: f64) -> Vector2 {
    Vector2::new(x, y)
}

pub fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

pub fn place(x: f64, y: f64, dx: f64, dy: f64) -> Placement2 {
    Placement2::new(pt(x, y), v(dx, dy)).unwrap()
}

/// SplitMix64, so every property test prints a seed that reproduces it.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// The named fields of a curve as raw bits, in a fixed order. Comparing these is what "bit exact"
/// means in the round-trip tests, and the name of the first differing field goes into the failure.
pub fn fields(c: &Curve) -> Vec<(String, Vec<u64>)> {
    let mut out = Vec::new();
    collect("", c, &mut out);
    out
}

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn collect(prefix: &str, c: &Curve, out: &mut Vec<(String, Vec<u64>)>) {
    let mut put = |name: &str, v: Vec<u64>| out.push((format!("{prefix}{name}"), v));
    match c {
        Curve::Line(l) => {
            put("kind", vec![0]);
            put("origin", bits(&[l.origin().x, l.origin().y]));
            put("direction", bits(&[l.direction().x, l.direction().y]));
        }
        Curve::Circle(c) => {
            put("kind", vec![1]);
            put("center", bits(&[c.placement().origin().x, c.placement().origin().y]));
            put("x_axis", bits(&[c.placement().x_axis().x, c.placement().x_axis().y]));
            put("radius", bits(&[c.radius()]));
        }
        Curve::Ellipse(e) => {
            put("kind", vec![2]);
            put("center", bits(&[e.placement().origin().x, e.placement().origin().y]));
            put("x_axis", bits(&[e.placement().x_axis().x, e.placement().x_axis().y]));
            put("semi_axis_1", bits(&[e.semi_axis_1()]));
            put("semi_axis_2", bits(&[e.semi_axis_2()]));
        }
        Curve::Polyline(p) => {
            put("kind", vec![3]);
            put("vertices", p.vertices().iter().flat_map(|q| [q.x.to_bits(), q.y.to_bits()]).collect());
        }
        Curve::Trimmed(t) => {
            put("kind", vec![4]);
            put("trim.start", bits(&[t.trim().start]));
            put("trim.span", bits(&[t.trim().span]));
            put("trim.reversed", vec![u64::from(t.trim().reversed)]);
            collect(&format!("{prefix}basis."), t.basis(), out);
        }
        Curve::Composite(c) => {
            put("kind", vec![5]);
            put("closed", vec![u64::from(c.is_declared_closed())]);
            put("segments", vec![c.segments().len() as u64]);
            for (i, s) in c.segments().iter().enumerate() {
                collect(&format!("{prefix}segment[{i}]."), s, out);
            }
        }
        Curve::Bezier(b) => {
            put("kind", vec![6]);
            put("closed", vec![u64::from(b.is_declared_closed())]);
            put(
                "control_points",
                b.control_points().iter().flat_map(|q| [q.x.to_bits(), q.y.to_bits()]).collect(),
            );
        }
        Curve::BSpline(s) => {
            put("kind", vec![7]);
            put("degree", vec![s.degree() as u64]);
            put("closed", vec![u64::from(s.is_declared_closed())]);
            put(
                "control_points",
                s.control_points().iter().flat_map(|q| [q.x.to_bits(), q.y.to_bits()]).collect(),
            );
            put("knots", bits(s.knots()));
        }
        Curve::RationalBSpline(s) => {
            put("kind", vec![8]);
            put("degree", vec![s.degree() as u64]);
            put("closed", vec![u64::from(s.is_declared_closed())]);
            put(
                "control_points",
                s.control_points().iter().flat_map(|q| [q.x.to_bits(), q.y.to_bits()]).collect(),
            );
            put("knots", bits(s.knots()));
            put("weights", bits(s.weights()));
        }
        other => panic!("fields() does not know {other:?}"),
    }
}

/// Panics naming the first field that differs, or the field list that is missing.
pub fn assert_bit_equal(what: &str, want: &Curve, got: &Curve) {
    let (a, b) = (fields(want), fields(got));
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(x, y, "{what}: field {} differs", x.0);
    }
    assert_eq!(a.len(), b.len(), "{what}: the decoded curve has a different number of fields");
}

pub fn line(ox: f64, oy: f64, dx: f64, dy: f64) -> Line {
    Line::new(pt(ox, oy), v(dx, dy)).unwrap()
}

/// A segment of a line, in canonical form: the trim runs from `a` to `b` along the carrier.
pub fn segment(ox: f64, oy: f64, dx: f64, dy: f64, a: f64, b: f64) -> Curve {
    Curve::Trimmed(Trimmed::new(Curve::Line(line(ox, oy, dx, dy)), a, b, a < b).unwrap())
}

/// One sample per basic kind and per awkward trim: the cases the issue names.
pub fn basic_samples() -> Vec<(&'static str, Curve)> {
    let third = 1.0 / 3.0;
    let circle = Circle::new(place(1.0 / 7.0, -2.0 * third, 1.0, 1.0), 3.0 * third).unwrap();
    let ellipse = Ellipse::new(place(0.1, 0.2, 2.0, 1.0), 4.0 * third, 1.5).unwrap();
    let tc =
        |t1: f64, t2: f64, s: bool| Curve::Trimmed(Trimmed::new(Curve::Circle(circle), t1, t2, s).unwrap());
    let te =
        |t1: f64, t2: f64, s: bool| Curve::Trimmed(Trimmed::new(Curve::Ellipse(ellipse), t1, t2, s).unwrap());
    let a = segment(0.0, 0.0, 1.0, 0.0, 0.0, 2.0);
    let b = Curve::Trimmed(
        Trimmed::new(
            Curve::Circle(Circle::new(place(2.0, 1.0, 1.0, 0.0), 1.0).unwrap()),
            -FRAC_PI_2,
            FRAC_PI_2,
            true,
        )
        .unwrap(),
    );
    let c = Curve::Polyline(Polyline::new(vec![pt(2.0, 2.0), pt(0.0, 2.0), pt(0.0, 0.0)]).unwrap());
    vec![
        ("line", Curve::Line(line(0.1, 0.2, 3.0, 4.0))),
        ("circle", Curve::Circle(circle)),
        ("ellipse", Curve::Ellipse(ellipse)),
        ("segment", segment(0.1, 0.7, 3.0, -4.0, 0.0, 12.0 * third)),
        ("segment-not-from-zero", segment(1.0, 1.0, 1.0, 1.0, 2.5, 9.0)),
        ("segment-inverted", segment(1.0, 1.0, 1.0, 1.0, 9.0, 2.5)),
        ("arc", tc(0.3, 2.9, true)),
        ("arc-inverted", tc(4.0, 1.0, false)),
        ("arc-wrapping-the-period", tc(5.5, 1.0, true)),
        ("arc-negative-parameters", tc(-1.0, 1.0, true)),
        ("arc-full-circle", tc(0.5, 0.5 + TAU, true)),
        ("elliptical-arc", te(0.3, 2.9, true)),
        ("elliptical-arc-inverted", te(4.0, 1.0, false)),
        ("polyline-2", Curve::Polyline(Polyline::new(vec![pt(0.1, 0.2), pt(1.0 / 3.0, 7.0)]).unwrap())),
        (
            "polyline-100",
            Curve::Polyline(
                Polyline::new((0..100).map(|i| pt(i as f64 * 0.37, ((i * i) % 17) as f64 / 3.0)).collect())
                    .unwrap(),
            ),
        ),
        ("composite", Curve::Composite(Composite::new(vec![a, b, c]).unwrap())),
    ]
}

/// A closed square as four polyline segments, `rot` positions rotated.
pub fn square(rot: usize) -> Composite {
    let corners = [pt(0., 0.), pt(4., 0.), pt(4., 4.), pt(0., 4.)];
    let segs: Vec<Curve> = (0..4)
        .map(|k| {
            let i = (k + rot) % 4;
            Curve::Polyline(Polyline::new(vec![corners[i], corners[(i + 1) % 4]]).unwrap())
        })
        .collect();
    Composite::new(segs).unwrap().declare_closed().unwrap()
}

pub fn entities(samples: &[(&'static str, Curve)]) -> Vec<(EntityId, Curve)> {
    samples.iter().enumerate().map(|(i, (_, c))| (id(i as u32), c.clone())).collect()
}
