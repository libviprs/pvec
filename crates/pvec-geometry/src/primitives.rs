//! Points, vectors, parameter domains, bounds and the closed list of curve kinds.

use crate::error::Result;
use pvec_numeric::check_value;

/// A point in the plane. Plain data: the curve constructors validate what they are given.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Point2 {
    /// The first coordinate.
    pub x: f64,
    /// The second coordinate.
    pub y: f64,
}

impl Point2 {
    /// A point with no validation, for values that are computed rather than received.
    pub const fn new(x: f64, y: f64) -> Point2 {
        Point2 { x, y }
    }

    /// A point that passed NP-01, NP-02 and NP-03: finite, normal, inside the coordinate bound,
    /// with negative zero stored as positive zero.
    pub fn checked(x: f64, y: f64) -> Result<Point2> {
        Ok(Point2 { x: check_value(x)?, y: check_value(y)? })
    }

    /// The vector from this point to `other`.
    pub fn vector_to(self, other: Point2) -> Vector2 {
        Vector2 { x: other.x - self.x, y: other.y - self.y }
    }

    /// Euclidean distance to `other`.
    pub fn distance(self, other: Point2) -> f64 {
        (other.x - self.x).hypot(other.y - self.y)
    }

    pub(crate) fn mag(self) -> f64 {
        self.x.abs().max(self.y.abs())
    }

    pub(crate) fn arr(self) -> [f64; 2] {
        [self.x, self.y]
    }
}

/// A free vector (a direction with a length, or a derivative).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vector2 {
    /// The first component.
    pub x: f64,
    /// The second component.
    pub y: f64,
}

impl Vector2 {
    /// A vector with no validation.
    pub const fn new(x: f64, y: f64) -> Vector2 {
        Vector2 { x, y }
    }

    /// Euclidean length.
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// Dot product.
    pub fn dot(self, other: Vector2) -> f64 {
        self.x * other.x + self.y * other.y
    }
}

/// The parameter interval `[lo, hi]` of a curve, with `lo < hi`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Domain {
    lo: f64,
    hi: f64,
}

impl Domain {
    /// A domain that passed NP-01 and has a width (NP-13).
    pub fn new(lo: f64, hi: f64) -> Result<Domain> {
        let (lo, hi) = (check_value(lo)?, check_value(hi)?);
        if hi <= lo {
            return Err(pvec_numeric::NumericError::EmptyDomain.into());
        }
        Ok(Domain { lo, hi })
    }

    pub(crate) fn raw(lo: f64, hi: f64) -> Domain {
        Domain { lo, hi }
    }

    /// The lower end.
    pub fn lo(&self) -> f64 {
        self.lo
    }

    /// The upper end.
    pub fn hi(&self) -> f64 {
        self.hi
    }

    /// `hi - lo`.
    pub fn width(&self) -> f64 {
        self.hi - self.lo
    }

    /// True when `t` is inside the closed interval (no tolerance).
    pub fn contains(&self, t: f64) -> bool {
        t >= self.lo && t <= self.hi
    }
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    min: Point2,
    max: Point2,
}

impl Bounds {
    pub(crate) fn of(p: Point2) -> Bounds {
        Bounds { min: p, max: p }
    }

    pub(crate) fn add(&mut self, p: Point2) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub(crate) fn union(mut self, o: Bounds) -> Bounds {
        self.add(o.min);
        self.add(o.max);
        self
    }

    /// The lower corner.
    pub fn min(&self) -> Point2 {
        self.min
    }

    /// The upper corner.
    pub fn max(&self) -> Point2 {
        self.max
    }

    /// Length of the diagonal.
    pub fn diagonal(&self) -> f64 {
        (self.max.x - self.min.x).hypot(self.max.y - self.min.y)
    }

    /// Largest coordinate magnitude, the `mag` argument of the tolerance formula (NP-04).
    pub fn magnitude(&self) -> f64 {
        self.min.mag().max(self.max.mag())
    }
}

/// The closed list of curve kinds in the v1 profile (`docs/ISO_PROFILE.md`, sections 4 to 12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum CurveKind {
    /// Section 4.
    Line,
    /// Section 6.
    Circle,
    /// Section 7.
    Ellipse,
    /// Section 5.
    Polyline,
    /// Section 8.
    TrimmedCurve,
    /// Section 9.
    CompositeCurve,
    /// Section 10.
    Bezier,
    /// Section 11.
    BSpline,
    /// Section 12.
    RationalBSpline,
}

impl CurveKind {
    /// Every kind, in profile order of the sections.
    pub const ALL: [CurveKind; 9] = [
        CurveKind::Line,
        CurveKind::Polyline,
        CurveKind::Circle,
        CurveKind::Ellipse,
        CurveKind::TrimmedCurve,
        CurveKind::CompositeCurve,
        CurveKind::Bezier,
        CurveKind::BSpline,
        CurveKind::RationalBSpline,
    ];
}

/// Refuses a result that is not finite (an overflow in an intermediate), so no consumer ever sees
/// a NaN or an infinity come out of a successful call.
pub(crate) fn finite_point(p: Point2) -> Result<Point2> {
    if p.x.is_finite() && p.y.is_finite() {
        Ok(p)
    } else {
        Err(pvec_numeric::NumericError::OutOfRange.into())
    }
}

/// As [`finite_point`], for vectors.
pub(crate) fn finite_vector(v: Vector2) -> Result<Vector2> {
    if v.x.is_finite() && v.y.is_finite() {
        Ok(v)
    } else {
        Err(pvec_numeric::NumericError::OutOfRange.into())
    }
}

pub(crate) fn point_as_vector(p: Point2) -> Vector2 {
    Vector2::new(p.x, p.y)
}

/// Accepts a parameter inside `[lo, hi]`, or within the NP-04 tolerance of it (clamped onto the end),
/// so a parameter that arrived through arithmetic one rounding step outside is not refused.
pub(crate) fn param_in(t: f64, lo: f64, hi: f64) -> Result<f64> {
    let t = check_value(t)?;
    if t >= lo && t <= hi {
        return Ok(t);
    }
    let tp = pvec_numeric::Policy::V1.tol(hi - lo, lo.abs().max(hi.abs()).max(t.abs()));
    if t >= lo - tp && t <= hi + tp { Ok(t.clamp(lo, hi)) } else { Err(crate::GeometryError::OutsideDomain) }
}
