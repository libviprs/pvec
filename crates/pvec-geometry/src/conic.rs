//! Line, circle and ellipse: the closed-form carriers (profile sections 3, 4, 6 and 7).

use crate::curve::{Curve, Curve2d};
use crate::error::{GeometryError, Result};
use crate::primitives::*;
use crate::transform::{ParamMap, Transform2};
use core::f64::consts::{FRAC_PI_2, TAU};
use pvec_numeric::{MAX_ABS_COORD, NumericError, Policy, Side, check_value};

const POLICY: Policy = Policy::V1;

/// A right-handed 2D frame (`axis2_placement_2d`): a location and a unit first axis. The second axis
/// is the first rotated by +90 degrees, so there is no left-handed frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement2 {
    origin: Point2,
    x_axis: Vector2,
}

impl Placement2 {
    /// Builds a frame. The reference direction is normalised (NP-10) and the origin is checked
    /// (NP-01, NP-02, NP-03).
    pub fn new(origin: Point2, ref_direction: Vector2) -> Result<Placement2> {
        let origin = Point2::checked(origin.x, origin.y)?;
        let d = POLICY.normalise_direction(ref_direction.x, ref_direction.y)?;
        // NP-01 and NP-03 apply to the normalised axis too: a component that came out subnormal is
        // refused, and negative zero is stored as positive zero.
        let x_axis = Vector2::new(check_value(d[0])?, check_value(d[1])?);
        Ok(Placement2 { origin, x_axis })
    }

    /// Rebuilds a frame from an axis that was already normalised and stored, keeping its bits. A
    /// decoder uses this instead of [`Placement2::new`], because normalising a unit vector again can
    /// move its last bit. The axis must be finite and unit length within NP-05, else it is refused
    /// under NP-10 (a stored direction that is not unit length) or NP-01.
    pub fn from_stored(origin: Point2, x_axis: Vector2) -> Result<Placement2> {
        let origin = Point2::checked(origin.x, origin.y)?;
        let x_axis = stored_unit(x_axis)?;
        Ok(Placement2 { origin, x_axis })
    }

    /// The location.
    pub fn origin(&self) -> Point2 {
        self.origin
    }

    /// The unit first axis.
    pub fn x_axis(&self) -> Vector2 {
        self.x_axis
    }

    /// The unit second axis.
    pub fn y_axis(&self) -> Vector2 {
        Vector2::new(-self.x_axis.y, self.x_axis.x)
    }

    fn mag(&self) -> f64 {
        self.origin.mag()
    }
}

/// An unbounded line with a unit direction, so the parameter is arc length from the origin point.
/// A bounded segment is a [`crate::Trimmed`] over a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    origin: Point2,
    direction: Vector2,
}

impl Line {
    /// Builds a line through `origin` along `direction`, which is normalised (NP-10).
    pub fn new(origin: Point2, direction: Vector2) -> Result<Line> {
        let origin = Point2::checked(origin.x, origin.y)?;
        let d = POLICY.normalise_direction(direction.x, direction.y)?;
        let direction = Vector2::new(check_value(d[0])?, check_value(d[1])?);
        Ok(Line { origin, direction })
    }

    /// Rebuilds a line from a direction that was already normalised and stored, keeping its bits
    /// (see [`Placement2::from_stored`]).
    pub fn from_stored(origin: Point2, direction: Vector2) -> Result<Line> {
        let origin = Point2::checked(origin.x, origin.y)?;
        Ok(Line { origin, direction: stored_unit(direction)? })
    }

    /// The point at parameter zero.
    pub fn origin(&self) -> Point2 {
        self.origin
    }

    /// The unit direction.
    pub fn direction(&self) -> Vector2 {
        self.direction
    }
}

impl crate::Sealed for Line {}

impl Curve2d for Line {
    fn kind(&self) -> CurveKind {
        CurveKind::Line
    }
    fn domain(&self) -> Domain {
        Domain::raw(-MAX_ABS_COORD, MAX_ABS_COORD)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        let t = check_value(t)?;
        finite_point(Point2::new(self.origin.x + t * self.direction.x, self.origin.y + t * self.direction.y))
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Vector2> {
        match order {
            0 => self.eval(t).map(point_as_vector),
            1 => {
                check_value(t)?;
                Ok(self.direction)
            }
            _ => {
                check_value(t)?;
                Ok(Vector2::new(0.0, 0.0))
            }
        }
    }
    fn bounds(&self) -> Result<Bounds> {
        Err(GeometryError::Unbounded)
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        let d = m.apply_vector(self.direction);
        let line = Line::new(m.apply_point(self.origin), d)?;
        Ok((Curve::Line(line), ParamMap { offset: 0.0, scale: d.length() }))
    }
}

/// Evaluates `c + a cos t x + b sin t y`.
fn conic_point(f: &Placement2, a: f64, b: f64, t: f64) -> Result<Point2> {
    let t = check_value(t)?;
    let (s, c) = t.sin_cos();
    let (x, y) = (f.x_axis, f.y_axis());
    finite_point(Point2::new(f.origin.x + a * c * x.x + b * s * y.x, f.origin.y + a * c * x.y + b * s * y.y))
}

/// The derivative of order `k >= 1`: `d^k/dt^k cos t = cos(t + k pi/2)`, and likewise for sin. The
/// phase is taken from `k mod 4`, so a huge `k` costs nothing and loses no precision.
fn conic_derivative(f: &Placement2, a: f64, b: f64, t: f64, order: usize) -> Result<Vector2> {
    let t = check_value(t)?;
    let (s, c) = (t + (order % 4) as f64 * FRAC_PI_2).sin_cos();
    let (x, y) = (f.x_axis, f.y_axis());
    finite_vector(Vector2::new(a * c * x.x + b * s * y.x, a * c * x.y + b * s * y.y))
}

/// A circle: `P(t) = c + r (cos t x + sin t y)`, period `2 pi`, canonical domain `[0, 2 pi)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    placement: Placement2,
    radius: f64,
}

impl Circle {
    /// Builds a circle. The radius must be finite, positive and not degenerate (NP-07).
    pub fn new(placement: Placement2, radius: f64) -> Result<Circle> {
        POLICY.check_radius(radius, placement.mag())?;
        Ok(Circle { placement, radius })
    }

    /// The frame.
    pub fn placement(&self) -> &Placement2 {
        &self.placement
    }

    /// The radius.
    pub fn radius(&self) -> f64 {
        self.radius
    }
}

impl crate::Sealed for Circle {}

impl Curve2d for Circle {
    fn kind(&self) -> CurveKind {
        CurveKind::Circle
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, TAU)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        conic_point(&self.placement, self.radius, self.radius, t)
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Vector2> {
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        conic_derivative(&self.placement, self.radius, self.radius, t, order)
    }
    fn bounds(&self) -> Result<Bounds> {
        Ok(arc_bounds(&self.placement, self.radius, self.radius, 0.0, TAU))
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        image_of_conic(&self.placement, self.radius, self.radius, m)
    }
}

/// An ellipse: `P(t) = c + a cos t x + b sin t y`. The first semi-axis runs along the frame's first
/// axis and the two are never reordered, so the major axis may be either.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ellipse {
    placement: Placement2,
    a: f64,
    b: f64,
}

impl Ellipse {
    /// Builds an ellipse. Both semi-axes must be usable and their ratio inside the conditioning
    /// bound (NP-07).
    pub fn new(placement: Placement2, semi_axis_1: f64, semi_axis_2: f64) -> Result<Ellipse> {
        POLICY.check_semi_axes(semi_axis_1, semi_axis_2, placement.mag())?;
        Ok(Ellipse { placement, a: semi_axis_1, b: semi_axis_2 })
    }

    /// Builds an ellipse from a centre, a major-axis vector (the end of the major axis relative to
    /// the centre) and the ratio of the minor to the major semi-axis: the three numbers a DWG
    /// `ELLIPSE` stores.
    ///
    /// The frame is the vector normalised (NP-10), the first semi-axis is its length and the second
    /// is that length times `ratio`, so the major axis stays first. A reader hands over the source
    /// numbers as they are and does no arithmetic on them. Refused: a zero vector (NP-10), a
    /// non-finite or subnormal ratio (NP-01), a ratio outside `(0, 1]` or one whose axis ratio is
    /// past the conditioning bound (NP-07), and any value NP-01, NP-02 or NP-07 refuse in
    /// [`Ellipse::new`]. A ratio of one is still an ellipse; PVec does not convert it to a circle.
    pub fn from_major_axis(centre: Point2, major_axis: Vector2, ratio: f64) -> Result<Ellipse> {
        let placement = Placement2::new(centre, major_axis)?;
        check_value(ratio)?;
        if !(ratio > 0.0 && ratio <= 1.0) {
            return Err(NumericError::DegenerateRadius.into());
        }
        let major = major_axis.length();
        Ellipse::new(placement, major, ratio * major)
    }

    /// The frame.
    pub fn placement(&self) -> &Placement2 {
        &self.placement
    }

    /// The semi-axis along the first axis.
    pub fn semi_axis_1(&self) -> f64 {
        self.a
    }

    /// The semi-axis along the second axis.
    pub fn semi_axis_2(&self) -> f64 {
        self.b
    }
}

impl crate::Sealed for Ellipse {}

impl Curve2d for Ellipse {
    fn kind(&self) -> CurveKind {
        CurveKind::Ellipse
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, TAU)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        conic_point(&self.placement, self.a, self.b, t)
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Vector2> {
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        conic_derivative(&self.placement, self.a, self.b, t, order)
    }
    fn bounds(&self) -> Result<Bounds> {
        Ok(arc_bounds(&self.placement, self.a, self.b, 0.0, TAU))
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        image_of_conic(&self.placement, self.a, self.b, m)
    }
}

/// The image of the conic `c + a cos t x + b sin t y` under `m`, as a canonical (orthogonal axes,
/// right-handed, major axis first) circle or ellipse, with the parameter map.
///
/// The image is `c' + u cos t + v sin t` with `u = M (a x)` and `v = M (b y)`, conjugate
/// semi-diameters that are generally not orthogonal. The principal axes sit at the stationary
/// points of `|E(t)|^2`, at `t0 = atan2(2 u.v, u.u - v.v) / 2`, and `s = t - t0` re-parameterises
/// onto them. If the image is left-handed (a reflection) the parameter also reverses.
///
/// The image of an ellipse under an invertible affine map is always an ellipse, so this never
/// leaves the conic family; it fails only when the image is too eccentric for NP-07.
pub(crate) fn image_of_conic(f: &Placement2, a: f64, b: f64, m: &Transform2) -> Result<(Curve, ParamMap)> {
    let (x, y) = (f.x_axis, f.y_axis());
    let u = m.apply_vector(Vector2::new(a * x.x, a * x.y));
    let v = m.apply_vector(Vector2::new(b * y.x, b * y.y));
    let t0 = 0.5 * (2.0 * u.dot(v)).atan2(u.dot(u) - v.dot(v));
    let (s0, c0) = t0.sin_cos();
    let p = Vector2::new(u.x * c0 + v.x * s0, u.y * c0 + v.y * s0);
    let q = Vector2::new(-u.x * s0 + v.x * c0, -u.y * s0 + v.y * c0);
    let (la, lb) = (p.length(), q.length());
    let frame = Placement2::new(m.apply_point(f.origin), p)?;
    let flipped = q.dot(frame.y_axis()) < 0.0;
    let map =
        if flipped { ParamMap { offset: t0, scale: -1.0 } } else { ParamMap { offset: -t0, scale: 1.0 } };
    POLICY.check_semi_axes(la, lb, frame.mag())?;
    // A similar image keeps a circle a circle.
    if POLICY.near(la, lb, la) {
        Ok((Curve::Circle(Circle { placement: frame, radius: la }), map))
    } else {
        Ok((Curve::Ellipse(Ellipse { placement: frame, a: la, b: lb }), map))
    }
}

/// Closed-form bounds of the arc `t in [start, start + span]` of `c + a cos t x + b sin t y`.
///
/// Along each world axis the coordinate is `c_k + a x_k cos t + b y_k sin t`, whose derivative
/// vanishes where `tan t = b y_k / (a x_k)`, that is at `t* = atan2(b y_k, a x_k)` and `t* + pi`.
/// The bounds are the extremes of the end points and of those stationary points that fall in the
/// arc. Nothing is sampled.
pub(crate) fn arc_bounds(f: &Placement2, a: f64, b: f64, start: f64, span: f64) -> Bounds {
    let (x, y) = (f.x_axis, f.y_axis());
    let at = |t: f64| -> Point2 {
        let (s, c) = t.sin_cos();
        Point2::new(f.origin.x + a * c * x.x + b * s * y.x, f.origin.y + a * c * x.y + b * s * y.y)
    };
    let mut out = Bounds::of(at(start));
    out.add(at(start + span));
    for (xk, yk) in [(x.x, y.x), (x.y, y.y)] {
        let t_star = (b * yk).atan2(a * xk);
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

/// A stored direction: finite and normal (NP-01), and unit length within the NP-05 tolerance (NP-10).
fn stored_unit(d: Vector2) -> Result<Vector2> {
    let d = Vector2::new(check_value(d.x)?, check_value(d.y)?);
    if POLICY.is_unit([d.x, d.y]) { Ok(d) } else { Err(GeometryError::NotUnitDirection) }
}
