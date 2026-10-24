//! Bezier, B-spline and rational B-spline (profile sections 10, 11 and 12).
//!
//! There is exactly one B-spline evaluator in PVec, the de Boor implementation in this file, and the
//! codecs, the transform code and the other repositories all reach it through [`Curve2d`]. Derivatives
//! are evaluated in place on the `degree + 1` control points around the span, so a derivative costs
//! no allocation and does not depend on the number of control points.

use crate::curve::{Curve, Curve2d};
use crate::error::{GeometryError, Result};
use crate::primitives::*;
use crate::transform::{ParamMap, Transform2};
use pvec_numeric::{
    MAX_CONTROL_POINTS, MAX_DEGREE, NumericError, Policy, Side, check_count, check_derivative_request,
    check_value,
};

const POLICY: Policy = Policy::V1;
const WIN: usize = MAX_DEGREE + 1;

/// Index of the knot span containing `t`, chosen from the side requested at a knot.
fn find_span(knots: &[f64], p: usize, n_cp: usize, t: f64, side: Side) -> usize {
    match side {
        Side::Left => {
            let idx = knots[p + 1..=n_cp].partition_point(|&k| k < t);
            let mut i = (p + idx).min(n_cp - 1);
            while i < n_cp - 1 && knots[i] == knots[i + 1] {
                i += 1;
            }
            i
        }
        _ => {
            let idx = knots[p..=n_cp].partition_point(|&k| k <= t);
            let mut i = (p + idx).saturating_sub(1).min(n_cp - 1);
            while i > p && knots[i] == knots[i + 1] {
                i -= 1;
            }
            i
        }
    }
}

/// The `k`-th derivative at `t` of the degree-`p` spline with control values `get`, evaluated on span
/// `i` (so `knots[i] < knots[i + 1]`). `N` is 2 for a plain curve and 3 for homogeneous coordinates.
///
/// Step one differentiates the control values in place (`Q_g^r = (p - r + 1) (Q_{g+1}^{r-1} -
/// Q_g^{r-1}) / (u_{g+p+1} - u_{g+r})`, a zero denominator giving zero), step two is de Boor on the
/// resulting degree `p - k` curve, whose knot vector is the original shifted by `k`. A derivative
/// above the degree is the zero vector.
fn local_eval<const N: usize>(
    get: impl Fn(usize) -> [f64; N],
    knots: &[f64],
    p: usize,
    i: usize,
    t: f64,
    k: usize,
) -> [f64; N] {
    if k > p {
        return [0.0; N];
    }
    let mut w = [[0.0; N]; WIN];
    for (j, slot) in w.iter_mut().enumerate().take(p + 1) {
        *slot = get(i - p + j);
    }
    for r in 1..=k {
        let f = (p - r + 1) as f64;
        for j in 0..=(p - r) {
            let g = i - p + j;
            let den = knots[g + p + 1] - knots[g + r];
            for c in 0..N {
                w[j][c] = if den != 0.0 { f * (w[j + 1][c] - w[j][c]) / den } else { 0.0 };
            }
        }
    }
    let q = p - k;
    for r in 1..=q {
        for j in (r..=q).rev() {
            let lo = knots[i - p + j + k];
            let alpha = (t - lo) / (knots[i + 1 + j - r] - lo);
            for c in 0..N {
                w[j][c] = (1.0 - alpha) * w[j - 1][c] + alpha * w[j][c];
            }
        }
    }
    w[q]
}

fn binom(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

/// The implementation shared by [`BSpline`] and [`RationalBSpline`].
#[derive(Clone, Debug, PartialEq)]
struct Core {
    degree: usize,
    cps: Vec<Point2>,
    knots: Vec<f64>,
    weights: Option<Vec<f64>>,
    lo: f64,
    hi: f64,
    closed: bool,
}

impl Core {
    fn new(degree: usize, cps: Vec<Point2>, knots: Vec<f64>, weights: Option<Vec<f64>>) -> Result<Core> {
        // Counts first: nothing below may be sized by an unchecked number.
        check_count(cps.len(), MAX_CONTROL_POINTS)?;
        let (lo, hi) = POLICY.validate_knots(degree, cps.len(), &knots)?;
        let mut pts = Vec::with_capacity(cps.len());
        let mut checked = Vec::with_capacity(cps.len());
        for q in &cps {
            let q = Point2::checked(q.x, q.y)?;
            pts.push(q.arr());
            checked.push(q);
        }
        POLICY.check_hull(&pts)?;
        let weights = match weights {
            Some(w) => {
                if w.len() != checked.len() {
                    return Err(GeometryError::WeightCount);
                }
                POLICY.validate_weights(&w)?;
                // Negative zero cannot occur (weights are strictly positive), so nothing to normalise.
                Some(w)
            }
            None => None,
        };
        let knots = knots.into_iter().map(|k| if k == 0.0 { 0.0 } else { k }).collect();
        Ok(Core { degree, cps: checked, knots, weights, lo, hi, closed: false })
    }

    fn param(&self, t: f64) -> Result<f64> {
        let t = check_value(t)?;
        if t >= self.lo && t <= self.hi {
            return Ok(t);
        }
        let tp = POLICY.tol(self.hi - self.lo, self.lo.abs().max(self.hi.abs()).max(t.abs()));
        if t >= self.lo - tp && t <= self.hi + tp {
            Ok(t.clamp(self.lo, self.hi))
        } else {
            Err(GeometryError::OutsideDomain)
        }
    }

    fn hom(&self, j: usize) -> [f64; 3] {
        let p = self.cps[j];
        let w = self.weights.as_ref().map_or(1.0, |w| w[j]);
        [w * p.x, w * p.y, w]
    }

    fn eval(&self, t: f64) -> Result<Point2> {
        let t = self.param(t)?;
        let i = find_span(&self.knots, self.degree, self.cps.len(), t, Side::Right);
        if self.weights.is_none() {
            let v = local_eval::<2>(|j| self.cps[j].arr(), &self.knots, self.degree, i, t, 0);
            return finite_point(Point2::new(v[0], v[1]));
        }
        let h = local_eval::<3>(|j| self.hom(j), &self.knots, self.degree, i, t, 0);
        finite_point(Point2::new(h[0] / h[2], h[1] / h[2]))
    }

    fn mult_at(&self, t: f64) -> usize {
        self.knots.iter().filter(|&&k| k == t).count()
    }

    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        let t = self.param(t)?;
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        let side = if t == self.lo {
            if side == Side::Left {
                return Err(GeometryError::OutsideDomain);
            }
            Side::Right
        } else if t == self.hi {
            if side == Side::Right {
                return Err(GeometryError::OutsideDomain);
            }
            Side::Left
        } else {
            let m = self.mult_at(t);
            if m > 0 {
                check_derivative_request(order, self.degree, m, side)?;
            }
            if side == Side::Both { Side::Right } else { side }
        };
        if order > self.degree {
            return Ok(Vector2::new(0.0, 0.0));
        }
        let i = find_span(&self.knots, self.degree, self.cps.len(), t, side);
        match &self.weights {
            None => {
                let v = local_eval::<2>(|j| self.cps[j].arr(), &self.knots, self.degree, i, t, order);
                finite_vector(Vector2::new(v[0], v[1]))
            }
            Some(_) => {
                // A^(j)(t) for j = 0..=order, in homogeneous coordinates.
                let mut a = [[0.0; 3]; WIN];
                for (j, slot) in a.iter_mut().enumerate().take(order + 1) {
                    *slot = local_eval::<3>(|q| self.hom(q), &self.knots, self.degree, i, t, j);
                }
                // C^(k) = (A^(k) - sum_{i=1..k} binom(k,i) w^(i) C^(k-i)) / w
                let mut cd = [[0.0; 2]; WIN];
                for k in 0..=order {
                    let mut v = [a[k][0], a[k][1]];
                    for q in 1..=k {
                        let b = binom(k, q) * a[q][2];
                        v[0] -= b * cd[k - q][0];
                        v[1] -= b * cd[k - q][1];
                    }
                    cd[k] = [v[0] / a[0][2], v[1] / a[0][2]];
                }
                finite_vector(Vector2::new(cd[order][0], cd[order][1]))
            }
        }
    }

    /// The control hull: a guaranteed enclosure, not the tight extent.
    fn hull_bounds(&self) -> Bounds {
        let mut b = Bounds::of(self.cps[0]);
        for p in &self.cps[1..] {
            b.add(*p);
        }
        b
    }

    fn declare_closed(mut self) -> Result<Core> {
        let (start, end) = (self.eval(self.lo)?, self.eval(self.hi)?);
        POLICY.check_closed_flag(true, start.arr(), end.arr(), self.hull_bounds().diagonal())?;
        self.closed = true;
        Ok(self)
    }

    fn mapped(&self, m: &Transform2) -> Result<Core> {
        let pts = self.cps.iter().map(|&p| m.apply_point(p)).collect();
        let core = Core::new(self.degree, pts, self.knots.clone(), self.weights.clone())?;
        if self.closed { core.declare_closed() } else { Ok(core) }
    }
}

/// A non-rational B-spline with an explicit knot vector (profile section 11).
#[derive(Clone, Debug, PartialEq)]
pub struct BSpline(Core);

impl BSpline {
    /// Validates under NP-01, NP-02, NP-08, NP-14 and NP-18 and builds the curve.
    pub fn new(degree: usize, control_points: Vec<Point2>, knots: Vec<f64>) -> Result<BSpline> {
        Core::new(degree, control_points, knots, None).map(BSpline)
    }

    /// STUB (red): the exact unwrap of a periodic B-spline, see issue 93.
    pub fn from_periodic(degree: usize, control_points: Vec<Point2>, knots: Vec<f64>) -> Result<Unwrapped> {
        let _ = (degree, control_points, knots);
        Err(GeometryError::Numeric(NumericError::BadKnots))
    }

    /// The degree.
    pub fn degree(&self) -> usize {
        self.0.degree
    }

    /// The control points.
    pub fn control_points(&self) -> &[Point2] {
        &self.0.cps
    }

    /// The knot vector, with multiplicities.
    pub fn knots(&self) -> &[f64] {
        &self.0.knots
    }

    /// True when the curve was declared closed and the claim was checked (NP-20).
    pub fn is_declared_closed(&self) -> bool {
        self.0.closed
    }

    /// Declares the curve closed. The claim is checked against the geometry and refused if the ends
    /// do not meet (NP-20); a closed flag is never inferred.
    pub fn declare_closed(self) -> Result<BSpline> {
        self.0.declare_closed().map(BSpline)
    }
}

/// A periodic B-spline unwrapped into the clamped form (issue 93).
#[derive(Clone, Debug, PartialEq)]
pub struct Unwrapped {
    /// The clamped, closed curve.
    pub curve: BSpline,
    /// Bound on the distance between the control points of `curve` and the exact ones, which bounds
    /// the locus deviation from the periodic curve.
    pub deviation_bound: f64,
}

impl crate::Sealed for BSpline {}

impl Curve2d for BSpline {
    fn kind(&self) -> CurveKind {
        CurveKind::BSpline
    }
    fn domain(&self) -> Domain {
        Domain::raw(self.0.lo, self.0.hi)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        self.0.eval(t)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        self.0.derivative(t, order, side)
    }
    /// The control hull, which encloses the curve but is not its tight extent.
    fn bounds(&self) -> Result<Bounds> {
        Ok(self.0.hull_bounds())
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        Ok((Curve::BSpline(BSpline(self.0.mapped(m)?)), ParamMap::IDENTITY))
    }
}

/// A rational B-spline: a B-spline plus one strictly positive weight per control point, evaluated in
/// homogeneous coordinates (profile section 12). A set of equal weights is still rational; it is
/// never silently demoted.
#[derive(Clone, Debug, PartialEq)]
pub struct RationalBSpline(Core);

impl RationalBSpline {
    /// As [`BSpline::new`], plus NP-15 on the weights.
    pub fn new(
        degree: usize,
        control_points: Vec<Point2>,
        knots: Vec<f64>,
        weights: Vec<f64>,
    ) -> Result<RationalBSpline> {
        Core::new(degree, control_points, knots, Some(weights)).map(RationalBSpline)
    }

    /// The degree.
    pub fn degree(&self) -> usize {
        self.0.degree
    }

    /// The control points.
    pub fn control_points(&self) -> &[Point2] {
        &self.0.cps
    }

    /// The knot vector, with multiplicities.
    pub fn knots(&self) -> &[f64] {
        &self.0.knots
    }

    /// The weights, one per control point.
    pub fn weights(&self) -> &[f64] {
        self.0.weights.as_deref().unwrap_or(&[])
    }

    /// True when the curve was declared closed and the claim was checked (NP-20).
    pub fn is_declared_closed(&self) -> bool {
        self.0.closed
    }

    /// Declares the curve closed; refused if the ends do not meet (NP-20).
    pub fn declare_closed(self) -> Result<RationalBSpline> {
        self.0.declare_closed().map(RationalBSpline)
    }
}

impl crate::Sealed for RationalBSpline {}

impl Curve2d for RationalBSpline {
    fn kind(&self) -> CurveKind {
        CurveKind::RationalBSpline
    }
    fn domain(&self) -> Domain {
        Domain::raw(self.0.lo, self.0.hi)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        self.0.eval(t)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Vector2> {
        self.0.derivative(t, order, side)
    }
    /// The control hull, which encloses the curve but is not its tight extent.
    fn bounds(&self) -> Result<Bounds> {
        Ok(self.0.hull_bounds())
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        Ok((Curve::RationalBSpline(RationalBSpline(self.0.mapped(m)?)), ParamMap::IDENTITY))
    }
}

/// A Bezier curve: `degree + 1` control points, parameter `[0, 1]`, no knot array (profile section 10).
/// A rational Bezier is a [`RationalBSpline`].
#[derive(Clone, Debug, PartialEq)]
pub struct Bezier {
    cps: Vec<Point2>,
    closed: bool,
}

impl Bezier {
    /// Builds a Bezier of degree `control_points.len() - 1`.
    pub fn new(control_points: Vec<Point2>) -> Result<Bezier> {
        if control_points.len() < 2 || control_points.len() > MAX_DEGREE + 1 {
            return Err(NumericError::Limit.into());
        }
        let mut cps = Vec::with_capacity(control_points.len());
        for p in &control_points {
            cps.push(Point2::checked(p.x, p.y)?);
        }
        let arr: Vec<[f64; 2]> = cps.iter().map(|p| p.arr()).collect();
        POLICY.check_hull(&arr)?;
        Ok(Bezier { cps, closed: false })
    }

    /// The degree.
    pub fn degree(&self) -> usize {
        self.cps.len() - 1
    }

    /// The control points.
    pub fn control_points(&self) -> &[Point2] {
        &self.cps
    }

    /// True when the curve was declared closed and the claim was checked (NP-20).
    pub fn is_declared_closed(&self) -> bool {
        self.closed
    }

    /// Declares the curve closed; refused if the ends do not meet (NP-20).
    pub fn declare_closed(mut self) -> Result<Bezier> {
        let b = self.bounds()?;
        POLICY.check_closed_flag(
            true,
            self.cps[0].arr(),
            self.cps[self.cps.len() - 1].arr(),
            b.diagonal(),
        )?;
        self.closed = true;
        Ok(self)
    }

    fn casteljau(cps: &[[f64; 2]], t: f64) -> [f64; 2] {
        let mut d = [[0.0; 2]; WIN];
        d[..cps.len()].copy_from_slice(cps);
        for r in 1..cps.len() {
            for j in 0..cps.len() - r {
                d[j] = [(1.0 - t) * d[j][0] + t * d[j + 1][0], (1.0 - t) * d[j][1] + t * d[j + 1][1]];
            }
        }
        d[0]
    }

    fn param(t: f64) -> Result<f64> {
        let t = check_value(t)?;
        let tp = POLICY.tol(1.0, 1.0);
        if (-tp..=1.0 + tp).contains(&t) { Ok(t.clamp(0.0, 1.0)) } else { Err(GeometryError::OutsideDomain) }
    }

    fn array(&self) -> ([[f64; 2]; WIN], usize) {
        let mut a = [[0.0; 2]; WIN];
        for (slot, p) in a.iter_mut().zip(&self.cps) {
            *slot = p.arr();
        }
        (a, self.cps.len())
    }
}

impl crate::Sealed for Bezier {}

impl Curve2d for Bezier {
    fn kind(&self) -> CurveKind {
        CurveKind::Bezier
    }
    fn domain(&self) -> Domain {
        Domain::raw(0.0, 1.0)
    }
    fn eval(&self, t: f64) -> Result<Point2> {
        let (a, n) = self.array();
        let q = Self::casteljau(&a[..n], Self::param(t)?);
        finite_point(Point2::new(q[0], q[1]))
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Vector2> {
        let t = Self::param(t)?;
        if order == 0 {
            return self.eval(t).map(point_as_vector);
        }
        if order > self.degree() {
            return Ok(Vector2::new(0.0, 0.0));
        }
        let (mut c, mut len) = self.array();
        let mut n = self.degree();
        for _ in 0..order {
            for i in 0..len - 1 {
                c[i] = [n as f64 * (c[i + 1][0] - c[i][0]), n as f64 * (c[i + 1][1] - c[i][1])];
            }
            len -= 1;
            n -= 1;
        }
        let q = Self::casteljau(&c[..len], t);
        finite_vector(Vector2::new(q[0], q[1]))
    }
    /// Tight up to degree 3 (the extremes are roots of a polynomial of degree at most 2 per axis),
    /// the control hull above that.
    fn bounds(&self) -> Result<Bounds> {
        let mut b = Bounds::of(self.cps[0]);
        b.add(self.cps[self.cps.len() - 1]);
        let n = self.degree();
        let (a, len) = self.array();
        if n <= 3 {
            for k in 0..2 {
                let d: Vec<f64> = (0..n).map(|i| n as f64 * (a[i + 1][k] - a[i][k])).collect();
                for t in quadratic_roots_in_unit(&d) {
                    let q = Self::casteljau(&a[..len], t);
                    b.add(Point2::new(q[0], q[1]));
                }
            }
        } else {
            for p in &self.cps {
                b.add(*p);
            }
        }
        Ok(b)
    }
    fn transform(&self, m: &Transform2) -> Result<(Curve, ParamMap)> {
        let moved = Bezier::new(self.cps.iter().map(|&p| m.apply_point(p)).collect())?;
        let moved = if self.closed { moved.declare_closed()? } else { moved };
        Ok((Curve::Bezier(moved), ParamMap::IDENTITY))
    }
}

/// Roots in (0, 1) of the Bernstein-form polynomial with 1, 2 or 3 coefficients `d` (degree 0, 1 or
/// 2 in `t`).
fn quadratic_roots_in_unit(d: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    match d.len() {
        2 => {
            // d0 (1-t) + d1 t = 0
            let den = d[0] - d[1];
            if den != 0.0 {
                out.push(d[0] / den);
            }
        }
        3 => {
            // d0 (1-t)^2 + 2 d1 t (1-t) + d2 t^2 = a t^2 + b t + c
            let (a, b, c) = (d[0] - 2.0 * d[1] + d[2], 2.0 * (d[1] - d[0]), d[0]);
            if a == 0.0 {
                if b != 0.0 {
                    out.push(-c / b);
                }
            } else {
                let disc = b * b - 4.0 * a * c;
                if disc >= 0.0 {
                    let s = disc.sqrt();
                    // Numerically stable pairing of the two roots.
                    let q = -0.5 * (b + b.signum() * s);
                    out.push(q / a);
                    if q != 0.0 {
                        out.push(c / q);
                    }
                }
            }
        }
        _ => {}
    }
    out.retain(|t| *t > 0.0 && *t < 1.0);
    out
}
