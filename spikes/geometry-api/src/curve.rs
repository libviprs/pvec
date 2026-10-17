//! The closed set of v1 curve kinds, and the three that are built from others.

use crate::conic::{Circle, Ellipse, Line, arc_bounds};
use crate::spline::{BSpline, Bezier, RationalBSpline};
use crate::{Affine, Bounds, Curve2d, Error, POLICY, ParamMap, Pt, Side};
use core::f64::consts::TAU;
use pvec_numeric::{
    MAX_COMPOSITE_SEGMENTS, MAX_POLYLINE_VERTICES, NumericError, check_count, check_derivative_request,
    check_value,
};

/// Every v1 curve kind. A closed enum, because the set is fixed by the profile and a decoder must
/// reject anything else; extensions arrive through the P3.5 rules, not through a trait object.
#[derive(Clone, Debug)]
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

impl Curve2d for Curve {
    fn domain(&self) -> (f64, f64) {
        dispatch!(self, c => c.domain())
    }
    #[inline]
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        dispatch!(self, c => c.eval(t))
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error> {
        dispatch!(self, c => c.derivative(t, order, side))
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        dispatch!(self, c => c.bounds())
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        dispatch!(self, c => c.transform(a))
    }
}

fn dist(a: Pt, b: Pt) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// A polyline: `n` vertices, vertex `i` at `t = i` (a PVec definition, see `ISO_PROFILE.md`).
#[derive(Clone, Debug)]
pub struct Polyline {
    pts: Vec<Pt>,
}

impl Polyline {
    /// Validates under NP-06 (no zero-length segment) and NP-18 (counts).
    pub fn new(pts: Vec<Pt>) -> Result<Polyline, Error> {
        if pts.len() < 2 {
            return Err(NumericError::Limit.into());
        }
        check_count(pts.len(), MAX_POLYLINE_VERTICES)?;
        for p in &pts {
            check_value(p[0])?;
            check_value(p[1])?;
        }
        for w in pts.windows(2) {
            let mag = w.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
            if POLICY.degenerate_length(dist(w[0], w[1]), mag) {
                return Err(NumericError::DegenerateLength.into());
            }
        }
        Ok(Polyline { pts })
    }
    fn seg(&self, i: usize) -> (Pt, Pt) {
        (self.pts[i], self.pts[i + 1])
    }
}

impl Curve2d for Polyline {
    fn domain(&self) -> (f64, f64) {
        (0.0, (self.pts.len() - 1) as f64)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        let t = check_value(t)?;
        let n = self.pts.len() - 1;
        if t < 0.0 || t > n as f64 {
            return Err(Error::Domain);
        }
        let i = (t as usize).min(n - 1);
        let u = t - i as f64;
        let (a, b) = self.seg(i);
        Ok([a[0] + u * (b[0] - a[0]), a[1] + u * (b[1] - a[1])])
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error> {
        let t = check_value(t)?;
        let n = self.pts.len() - 1;
        if t < 0.0 || t > n as f64 {
            return Err(Error::Domain);
        }
        if order == 0 {
            return self.eval(t);
        }
        if order >= 2 {
            return Ok([0.0, 0.0]);
        }
        let vertex = t.fract() == 0.0;
        let i = if !vertex {
            t as usize
        } else {
            let k = t as usize;
            if k == 0 {
                if side == Side::Left {
                    return Err(Error::Domain);
                }
                0
            } else if k == n {
                if side == Side::Right {
                    return Err(Error::Domain);
                }
                n - 1
            } else {
                // An interior vertex is a degree-1 knot of multiplicity 1 = degree: a C0 break.
                check_derivative_request(order, 1, 1, side)?;
                if side == Side::Left { k - 1 } else { k }
            }
        };
        let (a, b) = self.seg(i);
        Ok([b[0] - a[0], b[1] - a[1]])
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        let mut b = Bounds::of(self.pts[0]);
        for p in &self.pts[1..] {
            b.add(*p);
        }
        Ok(b)
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        Ok((
            Curve::Polyline(Polyline::new(self.pts.iter().map(|&p| a.point(p)).collect())?),
            ParamMap::IDENTITY,
        ))
    }
}

/// A trimmed curve. Its own parameter is `tau` in `[0, span]`: `tau = 0` is where the curve starts,
/// whichever way round the basis parameter runs.
#[derive(Clone, Debug)]
pub struct Trimmed {
    basis: Box<Curve>,
    start: f64,
    span: f64,
    reversed: bool,
}

impl Trimmed {
    /// Trims `basis` from `t1` to `t2`; `sense` is the ISO `sense_agreement`.
    pub fn new(basis: Curve, t1: f64, t2: f64, sense: bool) -> Result<Trimmed, Error> {
        let trim = match &basis {
            Curve::Circle(_) | Curve::Ellipse(_) => POLICY.normalise_periodic_trim(t1, t2, sense, TAU)?,
            Curve::Line(l) => {
                POLICY.normalise_unbounded_trim(t1, t2, sense, l.origin[0].abs().max(l.origin[1].abs()))?
            }
            Curve::Bezier(_) | Curve::BSpline(_) | Curve::RationalBSpline(_) => {
                let (lo, hi) = basis.domain();
                POLICY.normalise_trim(t1, t2, sense, lo, hi)?
            }
            _ => return Err(NumericError::Limit.into()),
        };
        Ok(Trimmed { basis: Box::new(basis), start: trim.start, span: trim.span, reversed: trim.reversed })
    }
    /// Basis parameter for trimmed parameter `tau`.
    fn basis_param(&self, tau: f64) -> Result<f64, Error> {
        let tau = check_value(tau)?;
        if !(0.0..=self.span).contains(&tau) {
            let tp = POLICY.tol(self.span, self.span.abs());
            if !(-tp..=self.span + tp).contains(&tau) {
                return Err(Error::Domain);
            }
        }
        let tau = tau.clamp(0.0, self.span);
        Ok(if self.reversed { self.start + self.span - tau } else { self.start + tau })
    }
}

impl Curve2d for Trimmed {
    fn domain(&self) -> (f64, f64) {
        (0.0, self.span)
    }
    fn eval(&self, tau: f64) -> Result<Pt, Error> {
        self.basis.eval(self.basis_param(tau)?)
    }
    fn derivative(&self, tau: f64, order: usize, side: Side) -> Result<Pt, Error> {
        let s = self.basis_param(tau)?;
        let side = match (self.reversed, side) {
            (true, Side::Left) => Side::Right,
            (true, Side::Right) => Side::Left,
            (_, s) => s,
        };
        let d = self.basis.derivative(s, order, side)?;
        let sign = if self.reversed && order % 2 == 1 { -1.0 } else { 1.0 };
        Ok([sign * d[0], sign * d[1]])
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        match &*self.basis {
            Curve::Circle(c) => Ok(arc_bounds(&c.frame, c.radius, c.radius, self.start, self.span)),
            Curve::Ellipse(e) => Ok(arc_bounds(&e.frame, e.a, e.b, self.start, self.span)),
            Curve::Line(_) => {
                let mut b = Bounds::of(self.eval(0.0)?);
                b.add(self.eval(self.span)?);
                Ok(b)
            }
            // Splines: the hull of the whole basis encloses the trimmed part but is not tight.
            other => other.bounds(),
        }
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        let (nb, map) = self.basis.transform(a)?;
        let (t1, t2) = if self.reversed {
            (map.apply(self.start + self.span), map.apply(self.start))
        } else {
            (map.apply(self.start), map.apply(self.start + self.span))
        };
        let sense = !self.reversed ^ (map.scale < 0.0);
        // tau is the distance travelled along the basis parameter, so it scales by |map.scale|
        // (a line stretches; conics only shift and flip).
        let tau_map = ParamMap { offset: 0.0, scale: map.scale.abs() };
        Ok((Curve::Trimmed(Trimmed::new(nb, t1, t2, sense)?), tau_map))
    }
}

/// A composite curve. Segment `k` occupies `[k, k + 1]` of the composite parameter, mapped
/// linearly from that segment's own domain (a PVec definition, see `ISO_PROFILE.md`).
#[derive(Clone, Debug)]
pub struct Composite {
    segs: Vec<Curve>,
}

impl Composite {
    /// Validates the segment kinds, the count (NP-18) and the joints (NP-17).
    pub fn new(segs: Vec<Curve>) -> Result<Composite, Error> {
        if segs.is_empty() {
            return Err(NumericError::Limit.into());
        }
        check_count(segs.len(), MAX_COMPOSITE_SEGMENTS)?;
        for s in &segs {
            if !matches!(
                s,
                Curve::Trimmed(_)
                    | Curve::Polyline(_)
                    | Curve::Bezier(_)
                    | Curve::BSpline(_)
                    | Curve::RationalBSpline(_)
            ) {
                return Err(NumericError::Limit.into());
            }
        }
        for w in segs.windows(2) {
            let end = w[0].eval(w[0].domain().1)?;
            let start = w[1].eval(w[1].domain().0)?;
            let scale = w[0].bounds()?.diagonal().max(w[1].bounds()?.diagonal());
            POLICY.check_join(end, start, scale)?;
        }
        Ok(Composite { segs })
    }
    fn locate(&self, t: f64) -> Result<(usize, f64), Error> {
        let t = check_value(t)?;
        let n = self.segs.len();
        if t < 0.0 || t > n as f64 {
            return Err(Error::Domain);
        }
        let k = (t as usize).min(n - 1);
        let (lo, hi) = self.segs[k].domain();
        Ok((k, lo + (t - k as f64) * (hi - lo)))
    }
}

impl Curve2d for Composite {
    fn domain(&self) -> (f64, f64) {
        (0.0, self.segs.len() as f64)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        let (k, u) = self.locate(t)?;
        self.segs[k].eval(u)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error> {
        let n = self.segs.len();
        let t = check_value(t)?;
        if order == 0 {
            return self.eval(t);
        }
        let joint = t.fract() == 0.0 && t > 0.0 && t < n as f64;
        let (k, u, side) = if joint {
            // Joint continuity is position only, so only one-sided limits exist (NP-16).
            check_derivative_request(order, 1, 1, side)?;
            let k = t as usize;
            if side == Side::Left {
                (k - 1, self.segs[k - 1].domain().1, Side::Left)
            } else {
                (k, self.segs[k].domain().0, Side::Right)
            }
        } else {
            let (k, u) = self.locate(t)?;
            (k, u, side)
        };
        let (lo, hi) = self.segs[k].domain();
        let d = self.segs[k].derivative(u, order, side)?;
        let f = (hi - lo).powi(order as i32);
        Ok([d[0] * f, d[1] * f])
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        let mut it = self.segs.iter();
        let mut b = it.next().unwrap().bounds()?;
        for s in it {
            b = b.union(s.bounds()?);
        }
        Ok(b)
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        let segs = self.segs.iter().map(|s| s.transform(a).map(|(c, _)| c)).collect::<Result<Vec<_>, _>>()?;
        Ok((Curve::Composite(Composite::new(segs)?), ParamMap::IDENTITY))
    }
}
