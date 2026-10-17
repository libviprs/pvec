//! Line, circle and ellipse: the closed-form carriers.

use crate::{Affine, Bounds, Curve, Curve2d, Error, POLICY, ParamMap, Pt, Side};
use core::f64::consts::{FRAC_PI_2, TAU};
use pvec_numeric::{MAX_ABS_COORD, check_value};

/// A right-handed 2D frame: an origin and a unit first axis. The second axis is the first rotated
/// by +90 degrees, so there is no left-handed frame (see `ISO_PROFILE.md`, axis placement 2D).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Location.
    pub origin: Pt,
    /// Unit first axis.
    pub x: Pt,
}

impl Frame {
    /// Builds a frame, normalising the reference direction (NP-10) and checking the origin.
    pub fn new(origin: Pt, ref_dir: Pt) -> Result<Frame, Error> {
        Ok(Frame {
            origin: [check_value(origin[0])?, check_value(origin[1])?],
            x: POLICY.normalise_direction(ref_dir[0], ref_dir[1])?,
        })
    }
    /// The second axis.
    pub fn y(&self) -> Pt {
        [-self.x[1], self.x[0]]
    }
    fn mag(&self) -> f64 {
        self.origin[0].abs().max(self.origin[1].abs())
    }
}

/// An unbounded line with a unit direction, so the parameter is arc length from the origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    /// Point at `t = 0`.
    pub origin: Pt,
    /// Unit direction.
    pub dir: Pt,
}

impl Line {
    /// Builds a line, normalising the direction.
    pub fn new(origin: Pt, dir: Pt) -> Result<Line, Error> {
        Ok(Line {
            origin: [check_value(origin[0])?, check_value(origin[1])?],
            dir: POLICY.normalise_direction(dir[0], dir[1])?,
        })
    }
}

impl Curve2d for Line {
    fn domain(&self) -> (f64, f64) {
        (-MAX_ABS_COORD, MAX_ABS_COORD)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        let t = check_value(t)?;
        Ok([self.origin[0] + t * self.dir[0], self.origin[1] + t * self.dir[1]])
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Pt, Error> {
        check_value(t)?;
        Ok(if order == 1 { self.dir } else { [0.0, 0.0] })
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        Err(Error::Unbounded)
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        let d = a.vector(self.dir);
        let s = d[0].hypot(d[1]);
        let line = Line::new(a.point(self.origin), d)?;
        Ok((Curve::Line(line), ParamMap { offset: 0.0, scale: s }))
    }
}

/// `P(t) = c + a cos t x + b sin t y`, `t` in radians. A circle is the `a == b` case, kept as its own
/// type because it needs one radius and no axis ratio.
macro_rules! conic_eval {
    ($self:ident, $t:ident, $a:expr, $b:expr) => {{
        let t = check_value($t)?;
        let (s, c) = t.sin_cos();
        let (x, y) = ($self.frame.x, $self.frame.y());
        Ok([
            $self.frame.origin[0] + $a * c * x[0] + $b * s * y[0],
            $self.frame.origin[1] + $a * c * x[1] + $b * s * y[1],
        ])
    }};
}

macro_rules! conic_deriv {
    ($self:ident, $t:ident, $order:ident, $a:expr, $b:expr) => {{
        let t = check_value($t)?;
        let (s, c) = (t + $order as f64 * FRAC_PI_2).sin_cos();
        let (x, y) = ($self.frame.x, $self.frame.y());
        // d^k/dt^k cos t = cos(t + k pi/2), and likewise for sin.
        Ok([$a * c * x[0] + $b * s * y[0], $a * c * x[1] + $b * s * y[1]])
    }};
}

/// A circle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    /// Placement.
    pub frame: Frame,
    /// Radius, strictly positive and not degenerate (NP-07).
    pub radius: f64,
}

impl Circle {
    /// Builds a circle.
    pub fn new(center: Pt, ref_dir: Pt, radius: f64) -> Result<Circle, Error> {
        let frame = Frame::new(center, ref_dir)?;
        POLICY.check_radius(radius, frame.mag())?;
        Ok(Circle { frame, radius })
    }
}

impl Curve2d for Circle {
    fn domain(&self) -> (f64, f64) {
        (0.0, TAU)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        conic_eval!(self, t, self.radius, self.radius)
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Pt, Error> {
        conic_deriv!(self, t, order, self.radius, self.radius)
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        Ok(arc_bounds(&self.frame, self.radius, self.radius, 0.0, TAU))
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        image_of_conic(&self.frame, self.radius, self.radius, a)
    }
}

/// An ellipse. `semi_axis_1` runs along the frame's first axis and is not reordered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    /// Placement.
    pub frame: Frame,
    /// Semi-axis along the first axis.
    pub a: f64,
    /// Semi-axis along the second axis.
    pub b: f64,
}

impl Ellipse {
    /// Builds an ellipse.
    pub fn new(center: Pt, ref_dir: Pt, a: f64, b: f64) -> Result<Ellipse, Error> {
        let frame = Frame::new(center, ref_dir)?;
        POLICY.check_semi_axes(a, b, frame.mag())?;
        Ok(Ellipse { frame, a, b })
    }
}

impl Curve2d for Ellipse {
    fn domain(&self) -> (f64, f64) {
        (0.0, TAU)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        conic_eval!(self, t, self.a, self.b)
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Pt, Error> {
        conic_deriv!(self, t, order, self.a, self.b)
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        Ok(arc_bounds(&self.frame, self.a, self.b, 0.0, TAU))
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        image_of_conic(&self.frame, self.a, self.b, a)
    }
}

fn dot(a: Pt, b: Pt) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The image of the conic `c + a cos t x + b sin t y` under `m`, as a canonical (orthogonal axes,
/// right-handed, major axis first) circle or ellipse, with the parameter map.
///
/// The image is `c' + u cos t + v sin t` with `u = M (a x)` and `v = M (b y)`, conjugate
/// semi-diameters that are generally not orthogonal. The principal axes sit at the stationary
/// points of `|E(t)|^2`, at `t0 = atan2(2 u.v, u.u - v.v) / 2`, and `s = t - t0` re-parameterises
/// onto them. If the image is left-handed (a reflection) the parameter also reverses.
pub(crate) fn image_of_conic(f: &Frame, a: f64, b: f64, m: &Affine) -> Result<(Curve, ParamMap), Error> {
    let u = m.vector([a * f.x[0], a * f.x[1]]);
    let v = m.vector([b * f.y()[0], b * f.y()[1]]);
    let t0 = 0.5 * (2.0 * dot(u, v)).atan2(dot(u, u) - dot(v, v));
    let (s0, c0) = t0.sin_cos();
    let p = [u[0] * c0 + v[0] * s0, u[1] * c0 + v[1] * s0];
    let q = [-u[0] * s0 + v[0] * c0, -u[1] * s0 + v[1] * c0];
    let (la, lb) = (p[0].hypot(p[1]), q[0].hypot(q[1]));
    let frame = Frame::new(m.point(f.origin), p)?;
    let flipped = dot(q, frame.y()) < 0.0;
    let map =
        if flipped { ParamMap { offset: t0, scale: -1.0 } } else { ParamMap { offset: -t0, scale: 1.0 } };
    let mag = frame.mag();
    POLICY.check_semi_axes(la, lb, mag)?;
    // Similar image: keep a circle a circle.
    if POLICY.near(la, lb, la) {
        Ok((Curve::Circle(Circle { frame, radius: la }), map))
    } else {
        Ok((Curve::Ellipse(Ellipse { frame, a: la, b: lb }), map))
    }
}

/// Closed-form bounds of the arc `t in [start, start + span]` of `c + a cos t x + b sin t y`.
///
/// Along each world axis the coordinate is `c_k + a x_k cos t + b y_k sin t`, whose derivative
/// vanishes where `tan t = b y_k / (a x_k)`, that is at `t* = atan2(b y_k, a x_k)` and `t* + pi`.
/// The bounds are the extremes of the end points and of those stationary points that fall in the
/// arc. Nothing is sampled.
pub fn arc_bounds(f: &Frame, a: f64, b: f64, start: f64, span: f64) -> Bounds {
    let (x, y) = (f.x, f.y());
    let at = |t: f64| -> Pt {
        let (s, c) = t.sin_cos();
        [f.origin[0] + a * c * x[0] + b * s * y[0], f.origin[1] + a * c * x[1] + b * s * y[1]]
    };
    let mut out = Bounds::of(at(start));
    out.add(at(start + span));
    for k in 0..2 {
        let t_star = (b * y[k]).atan2(a * x[k]);
        for t in [t_star, t_star + core::f64::consts::PI] {
            // Distance travelled from `start` to the stationary point, wrapped into [0, 2 pi).
            let along = (t - start).rem_euclid(TAU);
            if along <= span {
                out.add(at(start + along));
            }
        }
    }
    out
}
