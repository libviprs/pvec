//! Bezier, B-spline and rational B-spline, with two storage shapes for control points and knots.
//!
//! * owned: `Vec<[f64; 2]>` and `Vec<f64>`;
//! * borrowed: [`Flat`] (interleaved `x0 y0 x1 y1 ...`) and `&[f64]` over a buffer the caller owns,
//!   which is what a container reader hands over without copying.
//!
//! Both go through the same generic core, so a measured difference is the storage and nothing else.

use crate::{Affine, Bounds, Curve, Curve2d, Error, POLICY, ParamMap, Pt, Side};
use pvec_numeric::{NumericError, check_derivative_request, check_value};

/// Read access to control points, whatever they are stored in.
pub trait Cps {
    /// Number of points.
    fn n(&self) -> usize;
    /// Point `i`.
    fn get(&self, i: usize) -> Pt;
}

impl Cps for Vec<Pt> {
    #[inline(always)]
    fn n(&self) -> usize {
        self.len()
    }
    #[inline(always)]
    fn get(&self, i: usize) -> Pt {
        self[i]
    }
}

/// Control points interleaved in one caller-owned slice: `x0 y0 x1 y1 ...`.
#[derive(Clone, Copy, Debug)]
pub struct Flat<'a>(pub &'a [f64]);

impl Cps for Flat<'_> {
    #[inline(always)]
    fn n(&self) -> usize {
        self.0.len() / 2
    }
    #[inline(always)]
    fn get(&self, i: usize) -> Pt {
        [self.0[2 * i], self.0[2 * i + 1]]
    }
}

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

#[inline(always)]
fn deboor<const N: usize>(
    get: impl Fn(usize) -> [f64; N],
    knots: &[f64],
    p: usize,
    i: usize,
    t: f64,
) -> [f64; N] {
    let mut d = [[0.0; N]; 17];
    for j in 0..=p {
        d[j] = get(i - p + j);
    }
    for r in 1..=p {
        for j in (r..=p).rev() {
            let lo = knots[i - p + j];
            let alpha = (t - lo) / (knots[i + 1 + j - r] - lo);
            for c in 0..N {
                d[j][c] = (1.0 - alpha) * d[j - 1][c] + alpha * d[j][c];
            }
        }
    }
    d[p]
}

/// Differentiates a curve given by homogeneous control points `k` times.
fn derive<const N: usize>(
    mut cps: Vec<[f64; N]>,
    knots: &[f64],
    mut p: usize,
    k: usize,
) -> (Vec<[f64; N]>, Vec<f64>, usize) {
    let mut kn = knots.to_vec();
    for _ in 0..k {
        let mut q = Vec::with_capacity(cps.len().saturating_sub(1));
        for i in 0..cps.len() - 1 {
            let den = kn[i + p + 1] - kn[i + 1];
            let mut v = [0.0; N];
            if den != 0.0 {
                for c in 0..N {
                    v[c] = p as f64 * (cps[i + 1][c] - cps[i][c]) / den;
                }
            }
            q.push(v);
        }
        cps = q;
        kn = kn[1..kn.len() - 1].to_vec();
        p -= 1;
    }
    (cps, kn, p)
}

fn binom(n: usize, k: usize) -> f64 {
    (0..k).fold(1.0, |acc, i| acc * (n - i) as f64 / (i + 1) as f64)
}

/// The shared implementation behind [`BSpline`] and [`RationalBSpline`].
#[derive(Clone, Debug)]
struct Core<P: Cps, F: AsRef<[f64]>> {
    degree: usize,
    cps: P,
    knots: F,
    weights: Option<F>,
    lo: f64,
    hi: f64,
}

impl<P: Cps, F: AsRef<[f64]>> Core<P, F> {
    fn new(degree: usize, cps: P, knots: F, weights: Option<F>) -> Result<Self, Error> {
        let n = cps.n();
        let (lo, hi) = POLICY.validate_knots(degree, n, knots.as_ref())?;
        let mut pts = Vec::with_capacity(n);
        for i in 0..n {
            let q = cps.get(i);
            pts.push([check_value(q[0])?, check_value(q[1])?]);
        }
        POLICY.check_hull(&pts)?;
        if let Some(w) = &weights {
            if w.as_ref().len() != n {
                return Err(NumericError::Limit.into());
            }
            POLICY.validate_weights(w.as_ref())?;
        }
        Ok(Core { degree, cps, knots, weights, lo, hi })
    }

    fn param(&self, t: f64) -> Result<f64, Error> {
        let t = check_value(t)?;
        if t >= self.lo && t <= self.hi {
            return Ok(t);
        }
        let w = self.hi - self.lo;
        let tp = POLICY.tol(w, self.lo.abs().max(self.hi.abs()).max(t.abs()));
        if t >= self.lo - tp && t <= self.hi + tp {
            Ok(t.clamp(self.lo, self.hi))
        } else {
            Err(Error::Domain)
        }
    }

    fn eval(&self, t: f64) -> Result<Pt, Error> {
        let t = self.param(t)?;
        let kn = self.knots.as_ref();
        let i = find_span(kn, self.degree, self.cps.n(), t, Side::Right);
        Ok(match &self.weights {
            None => deboor::<2>(|j| self.cps.get(j), kn, self.degree, i, t),
            Some(w) => {
                let w = w.as_ref();
                let h = deboor::<3>(
                    |j| {
                        let p = self.cps.get(j);
                        [w[j] * p[0], w[j] * p[1], w[j]]
                    },
                    kn,
                    self.degree,
                    i,
                    t,
                );
                [h[0] / h[2], h[1] / h[2]]
            }
        })
    }

    fn hom_cps<const N: usize>(&self) -> Vec<[f64; N]> {
        (0..self.cps.n())
            .map(|j| {
                let p = self.cps.get(j);
                let w = self.weights.as_ref().map_or(1.0, |w| w.as_ref()[j]);
                let mut v = [0.0; N];
                if N == 3 {
                    v[0] = w * p[0];
                    v[1] = w * p[1];
                    v[2] = w;
                } else {
                    v[0] = p[0];
                    v[1] = p[1];
                }
                v
            })
            .collect()
    }

    fn mult_at(&self, t: f64) -> usize {
        self.knots.as_ref().iter().filter(|&&k| k == t).count()
    }

    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error> {
        let t = self.param(t)?;
        if order == 0 {
            return self.eval(t);
        }
        let side = if t == self.lo {
            if side == Side::Left {
                return Err(Error::Domain);
            }
            Side::Right
        } else if t == self.hi {
            if side == Side::Right {
                return Err(Error::Domain);
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
            return Ok([0.0, 0.0]);
        }
        let kn = self.knots.as_ref();
        match &self.weights {
            None => {
                let (c, k, p) = derive::<2>(self.hom_cps::<2>(), kn, self.degree, order);
                let i = find_span(&k, p, c.len(), t, side);
                Ok(deboor::<2>(|j| c[j], &k, p, i, t))
            }
            Some(_) => {
                let hom = self.hom_cps::<3>();
                // A^(j)(t) for j = 0..=order, in homogeneous coordinates.
                let mut a = Vec::with_capacity(order + 1);
                for j in 0..=order {
                    if j > self.degree {
                        a.push([0.0; 3]);
                        continue;
                    }
                    let (c, k, p) = derive::<3>(hom.clone(), kn, self.degree, j);
                    let i = find_span(&k, p, c.len(), t, side);
                    a.push(deboor::<3>(|q| c[q], &k, p, i, t));
                }
                // C^(k) = (A^(k) - sum_{i=1..k} binom(k,i) w^(i) C^(k-i)) / w
                let mut cd: Vec<Pt> = Vec::with_capacity(order + 1);
                for k in 0..=order {
                    let mut v = [a[k][0], a[k][1]];
                    for i in 1..=k {
                        let b = binom(k, i) * a[i][2];
                        v[0] -= b * cd[k - i][0];
                        v[1] -= b * cd[k - i][1];
                    }
                    cd.push([v[0] / a[0][2], v[1] / a[0][2]]);
                }
                Ok(cd[order])
            }
        }
    }

    /// Convex-hull bounds: a guaranteed enclosure, not the tight extent.
    fn hull_bounds(&self) -> Bounds {
        let mut b = Bounds::of(self.cps.get(0));
        for i in 1..self.cps.n() {
            b.add(self.cps.get(i));
        }
        b
    }

    fn mapped(&self, a: &Affine) -> (Vec<Pt>, Vec<f64>, Option<Vec<f64>>) {
        let pts = (0..self.cps.n()).map(|i| a.point(self.cps.get(i))).collect();
        (pts, self.knots.as_ref().to_vec(), self.weights.as_ref().map(|w| w.as_ref().to_vec()))
    }
}

/// A non-rational B-spline of any degree up to `MAX_DEGREE`.
#[derive(Clone, Debug)]
pub struct BSpline<P: Cps = Vec<Pt>, F: AsRef<[f64]> = Vec<f64>>(Core<P, F>);

impl<P: Cps, F: AsRef<[f64]>> BSpline<P, F> {
    /// Validates under NP-01, NP-02, NP-08, NP-14, NP-18 and builds the curve.
    pub fn new(degree: usize, cps: P, knots: F) -> Result<Self, Error> {
        Core::new(degree, cps, knots, None).map(BSpline)
    }
}

/// A rational B-spline: a B-spline plus strictly positive weights, carried in homogeneous
/// coordinates during evaluation.
#[derive(Clone, Debug)]
pub struct RationalBSpline<P: Cps = Vec<Pt>, F: AsRef<[f64]> = Vec<f64>>(Core<P, F>);

impl<P: Cps, F: AsRef<[f64]>> RationalBSpline<P, F> {
    /// As [`BSpline::new`] plus NP-15 on the weights.
    pub fn new(degree: usize, cps: P, knots: F, weights: F) -> Result<Self, Error> {
        Core::new(degree, cps, knots, Some(weights)).map(RationalBSpline)
    }
}

macro_rules! spline_curve2d {
    ($ty:ident, $variant:ident) => {
        impl<P: Cps, F: AsRef<[f64]>> Curve2d for $ty<P, F> {
            fn domain(&self) -> (f64, f64) {
                (self.0.lo, self.0.hi)
            }
            fn eval(&self, t: f64) -> Result<Pt, Error> {
                self.0.eval(t)
            }
            fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error> {
                self.0.derivative(t, order, side)
            }
            fn bounds(&self) -> Result<Bounds, Error> {
                Ok(self.0.hull_bounds())
            }
            fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
                let (pts, kn, w) = self.0.mapped(a);
                let degree = self.0.degree;
                let c = match w {
                    None => Curve::BSpline(BSpline::new(degree, pts, kn)?),
                    Some(w) => Curve::RationalBSpline(RationalBSpline::new(degree, pts, kn, w)?),
                };
                Ok((c, ParamMap::IDENTITY))
            }
        }
    };
}
spline_curve2d!(BSpline, BSpline);
spline_curve2d!(RationalBSpline, RationalBSpline);

impl<P: Cps, F: AsRef<[f64]>> BSpline<P, F> {
    /// Degree.
    pub fn degree(&self) -> usize {
        self.0.degree
    }
}

/// A Bezier curve: `degree + 1` control points, parameter `[0, 1]`, no knot array, de Casteljau.
#[derive(Clone, Debug)]
pub struct Bezier {
    cps: Vec<Pt>,
}

impl Bezier {
    /// Builds a Bezier of degree `cps.len() - 1`.
    pub fn new(cps: Vec<Pt>) -> Result<Bezier, Error> {
        if cps.len() < 2 || cps.len() > pvec_numeric::MAX_DEGREE + 1 {
            return Err(NumericError::Limit.into());
        }
        for p in &cps {
            check_value(p[0])?;
            check_value(p[1])?;
        }
        POLICY.check_hull(&cps)?;
        Ok(Bezier { cps })
    }
    /// Degree.
    pub fn degree(&self) -> usize {
        self.cps.len() - 1
    }
    fn casteljau(cps: &[Pt], t: f64) -> Pt {
        let mut d = [[0.0; 2]; 17];
        d[..cps.len()].copy_from_slice(cps);
        for r in 1..cps.len() {
            for j in 0..cps.len() - r {
                d[j] = [(1.0 - t) * d[j][0] + t * d[j + 1][0], (1.0 - t) * d[j][1] + t * d[j + 1][1]];
            }
        }
        d[0]
    }
    fn param(t: f64) -> Result<f64, Error> {
        let t = check_value(t)?;
        let tp = POLICY.tol(1.0, 1.0);
        if (-tp..=1.0 + tp).contains(&t) { Ok(t.clamp(0.0, 1.0)) } else { Err(Error::Domain) }
    }
}

impl Curve2d for Bezier {
    fn domain(&self) -> (f64, f64) {
        (0.0, 1.0)
    }
    fn eval(&self, t: f64) -> Result<Pt, Error> {
        Ok(Self::casteljau(&self.cps, Self::param(t)?))
    }
    fn derivative(&self, t: f64, order: usize, _side: Side) -> Result<Pt, Error> {
        let t = Self::param(t)?;
        if order == 0 {
            return Ok(Self::casteljau(&self.cps, t));
        }
        if order > self.degree() {
            return Ok([0.0, 0.0]);
        }
        let mut c = self.cps.clone();
        let mut n = self.degree();
        for _ in 0..order {
            c = (0..c.len() - 1)
                .map(|i| [n as f64 * (c[i + 1][0] - c[i][0]), n as f64 * (c[i + 1][1] - c[i][1])])
                .collect();
            n -= 1;
        }
        Ok(Self::casteljau(&c, t))
    }
    fn bounds(&self) -> Result<Bounds, Error> {
        let mut b = Bounds::of(self.cps[0]);
        b.add(*self.cps.last().unwrap());
        let n = self.degree();
        if n <= 3 {
            // Closed form: the derivative is a polynomial of degree <= 2 per axis.
            for k in 0..2 {
                let d: Vec<f64> = (0..n).map(|i| n as f64 * (self.cps[i + 1][k] - self.cps[i][k])).collect();
                for t in quadratic_roots_in_unit(&d) {
                    b.add(Self::casteljau(&self.cps, t));
                }
            }
        } else {
            for p in &self.cps {
                b.add(*p);
            }
        }
        Ok(b)
    }
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error> {
        Ok((Curve::Bezier(Bezier::new(self.cps.iter().map(|&p| a.point(p)).collect())?), ParamMap::IDENTITY))
    }
}

/// Roots in (0, 1) of the Bernstein-form polynomial with 1, 2 or 3 coefficients `d`
/// (degree 0, 1 or 2 in `t`).
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
