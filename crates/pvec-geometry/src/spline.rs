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

    /// Unwraps a periodic B-spline into the clamped form, exactly, and declares it closed (NP-20).
    ///
    /// The input is what a decoder reports for a periodic spline: the degree `p`, the `n` unique
    /// control points and the `n + 1` knots of one period, non-decreasing, where the first and the
    /// last knot name the same knot one period `T = knots[n] - knots[0]` apart. The curve it
    /// describes repeats the control points (`P[i + n] = P[i]`) and the knots (`k[j + n] = k[j] + T`),
    /// and a span `[k[j], k[j + 1]]` is shaped by `P[j] ..= P[j + p]` (indices modulo `n`). Its
    /// domain is `[knots[0], knots[n]]`, which the result keeps.
    ///
    /// The result is the same locus as the wrapped curve formed in f64, up to the rounding of the
    /// insertions: the wrapped, unclamped form is built (`n + p` points, `n + 2p + 1` knots, a legal
    /// [`BSpline`] by itself), then the knots at the two ends of the domain are inserted up to
    /// multiplicity `p + 1` (Boehm) and what lies outside the domain is dropped. Knot insertion changes
    /// the representation and not the curve. Every knot of the result is a value of the input, never
    /// recomputed.
    ///
    /// Two things are inexact and they are bounded separately. The insertions round their arithmetic,
    /// which [`Unwrapped::deviation_bound`] bounds against the f64-formed wrapped curve. And the
    /// extended knots `k[j] +- T` are rounded to f64 when they are formed, so the f64-formed wrapped
    /// curve is not quite the mathematically periodic one: [`Unwrapped::periodic_deviation_bound`]
    /// bounds the distance to that exact periodic curve, and refuses (NP-14) an input whose knots f64
    /// cannot keep apart after the shift, where no such bound exists.
    ///
    /// Refused: a degree outside `1..=16` or fewer than `degree + 1` points (NP-18), a knot count
    /// other than `n + 1`, a decreasing or ambiguous knot vector, or a knot whose multiplicity (first
    /// and last counted once) exceeds the degree (NP-14), any non-finite value (NP-01).
    pub fn from_periodic(degree: usize, control_points: Vec<Point2>, knots: Vec<f64>) -> Result<Unwrapped> {
        unwrap_periodic(degree, control_points, knots)
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
    /// Bound on the distance between the locus of `curve` and the locus of the f64-formed wrapped
    /// curve (the wrapped, unclamped form built with the extended knots `k[j] +- T` rounded to f64, the
    /// curve `BSpline::new` would evaluate on that input). It covers the rounding of the knot
    /// insertions only, and it is not a bound against the mathematically periodic curve: see
    /// [`Unwrapped::periodic_deviation_bound`].
    pub deviation_bound: f64,
    /// Bound on the distance between the locus of `curve` and the exact periodic curve, the one with
    /// the real-number knots `k[j] + m T`. It is `deviation_bound` plus the effect of rounding the
    /// extended knots: each is off by at most `delta = 2 u (2 T + S)` (`T` the period, `S` the larger
    /// of `|k[0]|` and `|k[n]|`), so a blending weight of the evaluation is off by at most
    /// `4 delta / (g - 2 delta)` (`g` the smallest gap between distinct knots), `p` weights are blended
    /// along any evaluation path and each moves a point by at most the control hull diameter, which is
    /// below `3 m`.
    pub periodic_deviation_bound: f64,
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

/// The unit roundoff of f64, 2^-53.
const UNIT_ROUNDOFF: f64 = 1.0 / 9007199254740992.0;

/// One Boehm insertion of `t` into the clamped-or-not B-spline `(pts, u)` of degree `p` (Piegl and
/// Tiller, A5.1 with r = 1). `t` is inside the domain and `u[i] <= t < u[i + 1]` for the returned
/// span, and every new point is a convex combination of two old ones.
fn insert_knot(p: usize, pts: &mut Vec<[f64; 2]>, u: &mut Vec<f64>, t: f64) {
    // the last index with u[i] <= t
    let i = u.partition_point(|&k| k <= t) - 1;
    let s = u.iter().filter(|&&k| k == t).count();
    let mut q = Vec::with_capacity(pts.len() + 1);
    q.extend_from_slice(&pts[..=(i - p)]);
    for j in (i - p + 1)..=(i - s) {
        let a = (t - u[j]) / (u[j + p] - u[j]);
        q.push([(1.0 - a) * pts[j - 1][0] + a * pts[j][0], (1.0 - a) * pts[j - 1][1] + a * pts[j][1]]);
    }
    q.extend_from_slice(&pts[(i - s)..]);
    u.insert(i + 1, t);
    *pts = q;
}

fn unwrap_periodic(p: usize, cps: Vec<Point2>, k: Vec<f64>) -> Result<Unwrapped> {
    let n = cps.len();
    // The wrapped form has n + p points: refuse a count over the limit before any work is done.
    check_count(n, MAX_CONTROL_POINTS)?;
    if !(1..=MAX_DEGREE).contains(&p) || n < p + 1 {
        return Err(NumericError::Limit.into());
    }
    check_count(n + p, MAX_CONTROL_POINTS)?;
    if k.len() != n + 1 {
        return Err(NumericError::BadKnots.into());
    }
    let mut pts = Vec::with_capacity(n + p);
    for q in &cps {
        pts.push(Point2::checked(q.x, q.y)?.arr());
    }
    let m = pts.iter().fold(0.0f64, |m, q| m.max(q[0].abs()).max(q[1].abs()));
    for &v in &k {
        check_value(v)?;
    }
    if k.windows(2).any(|w| w[1] < w[0]) {
        return Err(NumericError::BadKnots.into());
    }
    let period = k[n] - k[0];
    if period <= POLICY.tol(period, k[0].abs().max(k[n].abs())) {
        return Err(NumericError::EmptyDomain.into());
    }
    // The seam knot is one knot listed twice, so a run at the start and a run at the end add up.
    let lead = k.iter().take_while(|&&v| v == k[0]).count();
    let trail = k.iter().rev().take_while(|&&v| v == k[n]).count();
    if lead + trail - 1 > p {
        return Err(NumericError::BadKnots.into());
    }
    // The extended knots are rounded when they are formed. The bound against the periodic curve needs the
    // rounding to stay well below the smallest gap, otherwise f64 cannot tell the knots apart after the shift.
    let big = k[0].abs().max(k[n].abs());
    let delta = 2.0 * UNIT_ROUNDOFF * (2.0 * period + big);
    let gap = k.windows(2).map(|w| w[1] - w[0]).filter(|&d| d > 0.0).fold(f64::INFINITY, f64::min);
    if gap <= 4.0 * delta {
        return Err(NumericError::AmbiguousKnots.into());
    }
    // Wrap: control points repeat, knots shift by the period, with the seam knot kept bit for bit.
    for i in 0..p {
        pts.push(pts[i]);
    }
    let mut u = Vec::with_capacity(n + 2 * p + 1);
    for i in (1..=p).rev() {
        u.push(if k[n - i] == k[n] { k[0] } else { k[n - i] - period });
    }
    u.extend_from_slice(&k);
    for i in 1..=p {
        u.push(if k[i] == k[0] { k[n] } else { k[i] + period });
    }
    // Validate the wrapped form as the curve it is before touching it.
    Core::new(p, pts.iter().map(|q| Point2::new(q[0], q[1])).collect(), u.clone(), None)?;
    // Clamp: insert the two ends of the domain up to multiplicity p + 1, then cut.
    let (lo, hi) = (k[0], k[n]);
    let mut inserted = 0usize;
    for t in [lo, hi] {
        let have = u.iter().filter(|&&v| v == t).count();
        for _ in have..=p {
            insert_knot(p, &mut pts, &mut u, t);
            inserted += 1;
        }
    }
    let a = u.partition_point(|&v| v < lo);
    let b = u.partition_point(|&v| v <= lo) - 1; // last index of the start knots
    debug_assert_eq!(b - a, p);
    let first_hi = u.partition_point(|&v| v < hi);
    let cut: Vec<f64> = u[a..first_hi + p + 1].to_vec();
    let kept: Vec<Point2> = pts[a..first_hi].iter().map(|q| Point2::new(q[0], q[1])).collect();
    let curve = BSpline::new(p, kept, cut)?.declare_closed()?;
    // Each insertion is a convex combination of two points: the weight a carries three roundoffs, the
    // two products and the sum three more, so at most 10 u m per level in each coordinate, and a
    // convex combination does not amplify an earlier error. 15 covers the Euclidean norm.
    let deviation_bound = 15.0 * inserted as f64 * UNIT_ROUNDOFF * m;
    // A blending weight (t - u) / d moves by at most 4 delta / (gap - 2 delta) when every knot moves by
    // delta; p of them are blended along an evaluation path, each moving a point by the hull diameter.
    let alpha = 4.0 * delta / (gap - 2.0 * delta);
    let periodic_deviation_bound = deviation_bound + p as f64 * alpha * 3.0 * m;
    Ok(Unwrapped { curve, deviation_bound, periodic_deviation_bound })
}
