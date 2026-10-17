//! Shared fixtures and a tiny deterministic generator. No external dependency, so the property
//! tests are reproducible from a seed and a registry outage cannot break the gate.
#![allow(dead_code, missing_docs)]

use core::f64::consts::FRAC_PI_2;
use pvec_geometry::*;
use pvec_numeric::Policy;

pub const P: Policy = Policy::V1;

pub fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

pub fn v(x: f64, y: f64) -> Vector2 {
    Vector2::new(x, y)
}

pub fn near(a: Point2, b: Point2, scale: f64) -> bool {
    P.near_point([a.x, a.y], [b.x, b.y], scale)
}

pub fn place(x: f64, y: f64, dx: f64, dy: f64) -> Placement2 {
    Placement2::new(pt(x, y), v(dx, dy)).unwrap()
}

/// SplitMix64: small, fast, and good enough to drive property tests from a printed seed.
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    /// Any f64 bit pattern: NaN, infinities, subnormals and huge values all occur.
    pub fn any_f64(&mut self) -> f64 {
        f64::from_bits(self.next_u64())
    }
}

/// One valid sample of each of the nine kinds, plus the variants that exercise trims.
pub fn samples() -> Vec<(&'static str, Curve)> {
    let circle = Circle::new(place(1.0, -2.0, 1.0, 1.0), 3.0).unwrap();
    let ellipse = Ellipse::new(place(0.5, 0.5, 2.0, 1.0), 4.0, 1.5).unwrap();
    let arc = Trimmed::new(Curve::Circle(circle.clone()), 0.3, 2.9, true).unwrap();
    let arc_rev = Trimmed::new(Curve::Ellipse(ellipse.clone()), 4.0, 1.0, false).unwrap();
    let seg = Trimmed::new(Curve::Line(Line::new(pt(0.0, 0.0), v(3.0, 4.0)).unwrap()), 0.0, 5.0, true).unwrap();
    let bez = Bezier::new(vec![pt(0., 0.), pt(1., 3.), pt(4., 3.), pt(5., 0.)]).unwrap();
    let bsp = BSpline::new(
        3,
        vec![pt(0., 0.), pt(1., 2.), pt(2., 3.), pt(3., 0.), pt(4., -1.), pt(5., 2.)],
        vec![0., 0., 0., 0., 0.4, 0.7, 1., 1., 1., 1.],
    )
    .unwrap();
    let rat = RationalBSpline::new(
        2,
        vec![pt(0., 0.), pt(1., 2.), pt(3., 2.), pt(4., 0.)],
        vec![0., 0., 0., 0.5, 1., 1., 1.],
        vec![1., 0.5, 2., 1.],
    )
    .unwrap();
    let poly = Polyline::new(vec![pt(0., 0.), pt(2., 1.), pt(3., 3.), pt(1., 4.)]).unwrap();
    let a = Trimmed::new(Curve::Line(Line::new(pt(0.0, 0.0), v(1.0, 0.0)).unwrap()), 0.0, 2.0, true).unwrap();
    let c = Circle::new(place(2.0, 1.0, 1.0, 0.0), 1.0).unwrap();
    let b = Trimmed::new(Curve::Circle(c), -FRAC_PI_2, FRAC_PI_2, true).unwrap();
    let z = Bezier::new(vec![pt(2.0, 2.0), pt(1.0, 3.0), pt(0.0, 3.0), pt(-1.0, 2.0)]).unwrap();
    let comp = Composite::new(vec![Curve::Trimmed(a), Curve::Trimmed(b), Curve::Bezier(z)]).unwrap();
    vec![
        ("line", Curve::Line(Line::new(pt(1.0, 2.0), v(1.0, -1.0)).unwrap())),
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

/// `n` parameters across the domain, inclusive of both ends. Unbounded carriers get a short window.
pub fn params(c: &Curve, n: usize) -> Vec<f64> {
    let d = c.domain();
    let (lo, hi) = if c.kind() == CurveKind::Line { (-3.0, 3.0) } else { (d.lo(), d.hi()) };
    (0..=n).map(|i| lo + (hi - lo) * i as f64 / n as f64).collect()
}

/// Six transforms the profile cares about: translation, rotation with scale, anisotropic scale,
/// shear, reflection, reflection with rotation.
pub fn transforms() -> Vec<(&'static str, Transform2)> {
    let (s, c) = 0.6f64.sin_cos();
    vec![
        ("translate", Transform2::new([1., 0., 0., 1., 3.5, -2.0]).unwrap()),
        ("rotate-scale", Transform2::new([2.0 * c, -2.0 * s, 2.0 * s, 2.0 * c, 1.0, 1.0]).unwrap()),
        ("anisotropic", Transform2::new([3.0, 0., 0., 0.5, 0.0, 0.0]).unwrap()),
        ("shear", Transform2::new([1.0, 0.75, 0.0, 1.0, 0.0, 0.0]).unwrap()),
        ("mirror", Transform2::new([-1., 0., 0., 1., 2.0, 0.0]).unwrap()),
        ("mirror-rotate", Transform2::new([c, s, s, -c, -1.0, 4.0]).unwrap()),
    ]
}
