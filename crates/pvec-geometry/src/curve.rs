//! The [`Curve2d`] trait, the closed [`Curve`] enum over the nine v1 kinds, and the three kinds that
//! are built from others: polyline, trimmed curve and composite curve.

use crate::conic::{Circle, Ellipse, Line, arc_bounds};
use crate::error::{GeometryError, Result};
use crate::primitives::*;
use crate::spline::{BSpline, Bezier, RationalBSpline};
use crate::transform::{ParamMap, Transform2};
use core::f64::consts::TAU;
use pvec_numeric::{
    MAX_COMPOSITE_SEGMENTS, MAX_POLYLINE_VERTICES, NumericError, Policy, Side, Trim, check_count,
    check_derivative_request, check_value,
};

const POLICY: Policy = Policy::V1;

/// What every curve of the profile can do.
///
/// The trait is sealed: the set of kinds is fixed by `docs/ISO_PROFILE.md`, and a new kind arrives
/// through the extension rules (P3.5), not through an implementation outside this crate.
///
/// A derivative of order `0` is the point itself, returned as a vector. Where only one-sided limits
/// exist (a knot, a polyline vertex, a composite joint) a [`Side::Both`] request for an order the
/// curve is not smooth enough for is refused under NP-16 rather than answered from a chosen side.
pub trait Curve2d: crate::Sealed {
    /// Which of the nine kinds this is.
    fn kind(&self) -> CurveKind;
    /// The parameter domain.
    fn domain(&self) -> Domain;
    /// The point at parameter `t`.
    fn eval(&self, t: f64) -> Result<Point2>;
    /// The derivative of `order` at `t`, taken from `side`.
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2>;
    /// A box that contains the curve. Closed form for lines (once trimmed), arcs, ellipses,
    /// polylines and Bezier curves up to degree 3; the control hull, which encloses but is not
    /// tight, for splines and higher-degree Bezier curves. Nothing is sampled.
    fn bounds(&self) -> Result<Bounds>;
    /// The exact image under an affine map, together with the map from old parameters to new ones.
    /// No path through this function approximates: a kind that cannot represent its image exactly
    /// is refused.
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)>;
}

/// Every v1 curve kind. A closed enum, because the set is fixed by the profile and a decoder must
/// reject anything else.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Curve {
    /// Unbounded line.
    Line(Line),
    /// Circle.
    Circle(Circle),
    /// Ellipse.
    Ellipse(Ellipse),
    /// Polyline.
    Polyline(Polyline),
    /// Trimmed curve.
    Trimmed(Trimmed),
    /// Composite curve.
    Composite(Composite),
    /// Bezier curve.
    Bezier(Bezier),
    /// Non-rational B-spline.
    BSpline(BSpline),
    /// Rational B-spline.
    RationalBSpline(RationalBSpline),
}

macro_rules! dispatch {
    ($self:ident, $c:ident => $e:expr) => {
        match $self {
            Curve::Line($c) => $e,
            Curve::Circle($c) => $e,
            Curve::Ellipse($c) => $e,
            Curve::Polyline($c) => $e,
            Curve::Trimmed($c) => $e,
            Curve::Composite($c) => $e,
            Curve::Bezier($c) => $e,
            Curve::BSpline($c) => $e,
            Curve::RationalBSpline($c) => $e,
        }
    };
}

impl Curve {
    /// How deeply curves are nested, counting the outermost as 1 (NP-18 allows at most 4). The
    /// structure of the profile keeps this at 3 or less.
    pub fn nesting_depth(&self) -> usize {
        match self {
            Curve::Trimmed(t) => 1 + t.basis.nesting_depth(),
            Curve::Composite(c) => 1 + c.segs.iter().map(Curve::nesting_depth).max().unwrap_or(0),
            _ => 1,
        }
    }
}

impl crate::Sealed for Curve {}

impl Curve2d for Curve {
    fn kind(&self) -> CurveKind {
        dispatch!(self, c => c.kind())
    }
    fn domain(&self) -> Domain {
        dispatch!(self, c => c.domain())
    }
    #[inline]
    fn eval(&self, t: f64) -> Result<Point2> {
        dispatch!(self, c => c.eval(t))
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        dispatch!(self, c => c.derivative(t, order, side))
    }
    fn bounds(&self) -> Result<Bounds> {
        dispatch!(self, c => c.bounds())
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        dispatch!(self, c => c.transform(m))
    }
}

/// A polyline: `n` vertices, vertex `i` at `t = i` (a PVec definition, see `docs/ISO_PROFILE.md`).
#[derive(Clone, Debug, PartialEq)]
pub struct Polyline {
    pts: Vec<Point2>,
}

impl Polyline {
    /// Validates under NP-18 (between 2 and `MAX_POLYLINE_VERTICES` vertices), NP-01 to NP-03 and
    /// NP-06 (no zero-length segment).
    pub fn new(vertices: Vec<Point2>) -> Result<Polyline> {
        if vertices.len() < 2 {
            return Err(NumericError::Limit.into());
        }
        check_count(vertices.len(), MAX_POLYLINE_VERTICES)?;
        let mut pts = Vec::with_capacity(vertices.len());
        for p in &vertices {
            pts.push(Point2::checked(p.x, p.y)?);
        }
        for w in pts.windows(2) {
            let mag = w[0].mag().max(w[1].mag());
            if POLICY.degenerate_length(w[0].distance(w[1]), mag) {
                return Err(NumericError::DegenerateLength.into());
            }
        }
        Ok(Polyline { pts })
    }

    /// The vertices.
    pub fn vertices(&self) -> &[Point2] {
        &self.pts
    }

    fn seg(&self, i: usize) -> (Point2, Point2) {
        (self.pts[i], self.pts[i + 1])
    }
}

impl crate::Sealed for Polyline {}

impl Curve2d for Polyline {
    fn kind(&self) -> CurveKind {
        CurveKind::Polyline
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, (self.pts.len() - 1) as f64)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        let n = self.pts.len() - 1;
        let t = param_in(t, 0.0, n as f64)?;
        let i = (t as usize).min(n - 1);
        let u = t - i as f64;
        let (a, b) = self.seg(i);
        finite_point(Point2::new(a.x + u * (b.x - a.x), a.y + u * (b.y - a.y)))
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        let n = self.pts.len() - 1;
        let t = param_in(t, 0.0, n as f64)?;
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        if order >= 2 {
            return Ok(Vector2::new(0.0, 0.0));
        }
        let i = if t.fract() != 0.0 {
            t as usize
        } else {
            let k = t as usize;
            if k == 0 {
                if side == Side::Left {
                    return Err(GeometryError::OutsideDomain);
                }
                0
            } else if k == n {
                if side == Side::Right {
                    return Err(GeometryError::OutsideDomain);
                }
                n - 1
            } else {
                // An interior vertex is a degree-1 knot of multiplicity 1 = degree: a C0 break.
                check_derivative_request(order, 1, 1, side)?;
                if side == Side::Left { k - 1 } else { k }
            }
        };
        let (a, b) = self.seg(i);
        finite_vector(a.vector_to(b))
    }
    fn bounds(&self) -> Result<Bounds> {
        let mut b = Bounds::of(self.pts[0]);
        for p in &self.pts[1..] {
            b.add(*p);
        }
        Ok(b)
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        let moved = self.pts.iter().map(|&p| m.apply_point(p)).collect();
        Ok((Curve::Polyline(Polyline::new(moved)?), ParamMap::IDENTITY))
    }
}

/// A trimmed curve. Its own parameter is `tau` in `[0, span]`: `tau = 0` is where the curve starts,
/// whichever way round the basis parameter runs. The basis is a line, circle, ellipse, Bezier or
/// B-spline (profile section 8).
#[derive(Clone, Debug, PartialEq)]
pub struct Trimmed {
    basis: Box<Curve>,
    trim: Trim,
}

impl Trimmed {
    /// Trims `basis` from `t1` to `t2`; `sense` is the ISO `sense_agreement`. The trim is normalised
    /// under NP-11 and NP-12.
    pub fn new(basis: Curve, t1: f64, t2: f64, sense: bool) -> Result<Trimmed> {
        let trim = match &basis {
            Curve::Circle(_) | Curve::Ellipse(_) => POLICY.normalise_periodic_trim(t1, t2, sense, TAU)?,
            Curve::Line(l) => POLICY.normalise_unbounded_trim(t1, t2, sense, l.origin().mag())?,
            Curve::Bezier(_) | Curve::BSpline(_) | Curve::RationalBSpline(_) => {
                let d = basis.domain();
                POLICY.normalise_trim(t1, t2, sense, d.lo(), d.hi())?
            }
            _ => return Err(GeometryError::BadBasis),
        };
        Ok(Trimmed { basis: Box::new(basis), trim })
    }

    /// Rebuilds a trimmed curve from an already normalised trim, keeping its numbers bit for bit.
    /// This is what a decoder uses; the trim is validated against the same rules as [`Trimmed::new`]
    /// but is not re-normalised, so a round trip cannot move a parameter.
    pub fn from_normalised(basis: Curve, trim: Trim) -> Result<Trimmed> {
        let (start, span) = (check_value(trim.start)?, check_value(trim.span)?);
        match &basis {
            Curve::Circle(_) | Curve::Ellipse(_) => {
                let tp = POLICY.tol(TAU, TAU);
                if !(0.0..TAU).contains(&start) {
                    return Err(NumericError::TrimOutsideDomain.into());
                }
                if span <= tp {
                    return Err(NumericError::CollapsedTrim.into());
                }
                if span > TAU {
                    return Err(NumericError::PeriodicSpan.into());
                }
            }
            Curve::Line(l) => {
                if POLICY.degenerate_length(span, l.origin().mag().max(start.abs()).max((start + span).abs()))
                {
                    return Err(NumericError::CollapsedTrim.into());
                }
            }
            Curve::Bezier(_) | Curve::BSpline(_) | Curve::RationalBSpline(_) => {
                let d = basis.domain();
                let tp = POLICY.tol(d.width(), d.lo().abs().max(d.hi().abs()));
                if span <= tp {
                    return Err(NumericError::CollapsedTrim.into());
                }
                if start < d.lo() - tp || start + span > d.hi() + tp {
                    return Err(NumericError::TrimOutsideDomain.into());
                }
            }
            _ => return Err(GeometryError::BadBasis),
        }
        Ok(Trimmed { basis: Box::new(basis), trim: Trim { start, span, reversed: trim.reversed } })
    }

    /// The curve being trimmed.
    pub fn basis(&self) -> &Curve {
        &self.basis
    }

    /// The normalised trim: an increasing interval of the basis parameter plus a direction flag.
    pub fn trim(&self) -> Trim {
        self.trim
    }

    /// Basis parameter for trimmed parameter `tau`.
    fn basis_param(&self, tau: f64) -> Result<f64> {
        let tau = param_in(tau, 0.0, self.trim.span)?;
        let Trim { start, span, reversed } = self.trim;
        Ok(if reversed { start + span - tau } else { start + tau })
    }
}

impl crate::Sealed for Trimmed {}

impl Curve2d for Trimmed {
    fn kind(&self) -> CurveKind {
        CurveKind::TrimmedCurve
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, self.trim.span)
    }
    fn eval(&self, tau: f64) -> Result<Point2> {
        self.basis.eval(self.basis_param(tau)?)
    }
    fn derivative(&self, tau: f64, order: usize, side: Side) -> Result<Vector2> {
        let s = self.basis_param(tau)?;
        if order == 0 {
            return self.basis.eval(s).map(point_as_vector);
        }
        let side = match (self.trim.reversed, side) {
            (true, Side::Left) => Side::Right,
            (true, Side::Right) => Side::Left,
            (_, s) => s,
        };
        let d = self.basis.derivative(s, order, side)?;
        let sign = if self.trim.reversed && order % 2 == 1 { -1.0 } else { 1.0 };
        Ok(Vector2::new(sign * d.x, sign * d.y))
    }
    fn bounds(&self) -> Result<Bounds> {
        let Trim { start, span, .. } = self.trim;
        match &*self.basis {
            Curve::Circle(c) => Ok(arc_bounds(c.placement(), c.radius(), c.radius(), start, span)),
            Curve::Ellipse(e) => Ok(arc_bounds(e.placement(), e.semi_axis_1(), e.semi_axis_2(), start, span)),
            Curve::Line(_) => {
                let mut b = Bounds::of(self.eval(0.0)?);
                b.add(self.eval(span)?);
                Ok(b)
            }
            // Splines: the hull of the whole basis encloses the trimmed part but is not tight.
            other => other.bounds(),
        }
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        let Trim { start, span, reversed } = self.trim;
        let (nb, map) = self.basis.transform(m)?;
        // A line stretches and a spline keeps its parameter: the trim scales, and nothing wraps, so
        // the numbers are carried over directly. A conic shifts or flips its parameter and has to be
        // re-normalised around the period.
        let tau_map = ParamMap { offset: 0.0, scale: map.scale.abs() };
        let moved = if matches!(
            *self.basis,
            Curve::Line(_) | Curve::Bezier(_) | Curve::BSpline(_) | Curve::RationalBSpline(_)
        ) {
            Trimmed::from_normalised(nb, Trim { start: map.scale * start, span: map.scale * span, reversed })?
        } else {
            let (t1, t2) = if reversed {
                (map.apply(start + span), map.apply(start))
            } else {
                (map.apply(start), map.apply(start + span))
            };
            let sense = !reversed ^ (map.scale < 0.0);
            Trimmed::new(nb, t1, t2, sense)?
        };
        Ok((Curve::Trimmed(moved), tau_map))
    }
}

/// A composite curve. Segment `k` occupies `[k, k + 1]` of the composite parameter, mapped linearly
/// from that segment's own domain (a PVec definition, see `docs/ISO_PROFILE.md`). Segments are
/// trimmed curves, polylines, Bezier curves or B-splines, and must join end to start (NP-17).
#[derive(Clone, Debug, PartialEq)]
pub struct Composite {
    segs: Vec<Curve>,
    closed: bool,
}

impl Composite {
    /// Validates the count (NP-18), the segment kinds (profile section 9) and the joints (NP-17).
    pub fn new(segments: Vec<Curve>) -> Result<Composite> {
        if segments.is_empty() {
            return Err(NumericError::Limit.into());
        }
        check_count(segments.len(), MAX_COMPOSITE_SEGMENTS)?;
        for s in &segments {
            if !matches!(
                s,
                Curve::Trimmed(_)
                    | Curve::Polyline(_)
                    | Curve::Bezier(_)
                    | Curve::BSpline(_)
                    | Curve::RationalBSpline(_)
            ) {
                return Err(GeometryError::BadSegment);
            }
        }
        for w in segments.windows(2) {
            let end = w[0].eval(w[0].domain().hi())?;
            let start = w[1].eval(w[1].domain().lo())?;
            let scale = w[0].bounds()?.diagonal().max(w[1].bounds()?.diagonal());
            POLICY.check_join(end.arr(), start.arr(), scale)?;
        }
        Ok(Composite { segs: segments, closed: false })
    }

    /// The segments, in order. Reordering them changes the curve.
    pub fn segments(&self) -> &[Curve] {
        &self.segs
    }

    /// True when the composite was declared closed and the claim was checked (NP-20).
    pub fn is_declared_closed(&self) -> bool {
        self.closed
    }

    /// Declares the composite closed. The last end must meet the first start (NP-20).
    pub fn declare_closed(mut self) -> Result<Composite> {
        let first = self.segs[0].eval(self.segs[0].domain().lo())?;
        let last_seg = &self.segs[self.segs.len() - 1];
        let last = last_seg.eval(last_seg.domain().hi())?;
        let scale = Curve::Composite(self.clone()).bounds()?.diagonal();
        POLICY.check_closed_flag(true, first.arr(), last.arr(), scale)?;
        self.closed = true;
        Ok(self)
    }

    fn locate(&self, t: f64) -> Result<(usize, f64)> {
        let n = self.segs.len();
        let t = param_in(t, 0.0, n as f64)?;
        let k = (t as usize).min(n - 1);
        let d = self.segs[k].domain();
        Ok((k, d.lo() + (t - k as f64) * d.width()))
    }
}

impl crate::Sealed for Composite {}

impl Curve2d for Composite {
    fn kind(&self) -> CurveKind {
        CurveKind::CompositeCurve
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, self.segs.len() as f64)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        let (k, u) = self.locate(t)?;
        self.segs[k].eval(u)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        let n = self.segs.len();
        let t = param_in(t, 0.0, n as f64)?;
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        let joint = t.fract() == 0.0 && t > 0.0 && t < n as f64;
        let (k, u, side) = if joint {
            // Joint continuity is position only, so only one-sided limits exist (NP-16).
            check_derivative_request(order, 1, 1, side)?;
            let k = t as usize;
            if side == Side::Left {
                (k - 1, self.segs[k - 1].domain().hi(), Side::Left)
            } else {
                (k, self.segs[k].domain().lo(), Side::Right)
            }
        } else {
            let (k, u) = self.locate(t)?;
            (k, u, side)
        };
        let d = self.segs[k].derivative(u, order, side)?;
        // Chain rule for the linear map from the composite parameter to the segment's own.
        let f = self.segs[k].domain().width().powf(order as f64);
        finite_vector(Vector2::new(d.x * f, d.y * f))
    }
    fn bounds(&self) -> Result<Bounds> {
        let mut it = self.segs.iter();
        let mut b = match it.next() {
            Some(s) => s.bounds()?,
            None => return Err(NumericError::Limit.into()),
        };
        for s in it {
            b = b.union(s.bounds()?);
        }
        Ok(b)
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        let segs = self.segs.iter().map(|s| s.transform(m).map(|(c, _)| c)).collect::<Result<Vec<_>>>()?;
        let moved = Composite::new(segs)?;
        let moved = if self.closed { moved.declare_closed()? } else { moved };
        Ok((Curve::Composite(moved), ParamMap::IDENTITY))
    }
}
