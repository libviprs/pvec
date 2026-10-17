//! Computable predicates for the rules of `docs/NUMERIC_POLICY.md`.
//!
//! Every function here is a direct transcription of one rule. There is no tolerance literal in this
//! file: each one comes from a [`Policy`].

use crate::error::NumericError as E;
use crate::policy::*;

/// Result of an orientation test (NP-09).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// `c` is to the left of the directed line `a -> b`.
    Ccw,
    /// `c` is to the right.
    Cw,
    /// `c` is within the tolerance band of the line.
    Collinear,
}

/// What a control hull degenerates to (NP-08).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HullClass {
    /// Every control point coincides: not a curve.
    Point,
    /// All control points lie on one line: a legal curve with a linear locus.
    Linear,
    /// Anything else.
    Planar,
}

/// Which side a derivative is taken from (NP-16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Limit from below the parameter.
    Left,
    /// Limit from above the parameter.
    Right,
    /// The two-sided derivative, which exists only where both limits agree.
    Both,
}

/// A trim after normalisation: an increasing interval plus a direction flag (NP-11, NP-12).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trim {
    /// Start of the interval, in the basis parameter.
    pub start: f64,
    /// Width of the interval, strictly positive.
    pub span: f64,
    /// True when the curve runs from the end of the interval back to its start.
    pub reversed: bool,
}

/// Spacing between `x` and the next larger `f64` magnitude (the ULP of `x`), computed from the bit
/// pattern so it does not need `f64::next_up` (not available at the MSRV).
pub fn ulp_of(x: f64) -> f64 {
    let a = x.abs();
    if !a.is_finite() {
        return f64::INFINITY;
    }
    f64::from_bits(a.to_bits() + 1) - a
}

/// NP-01, NP-02, NP-03: accepts a finite, normal (or zero) value within the coordinate bound and
/// returns it with negative zero normalised to positive zero.
pub fn check_value(x: f64) -> Result<f64, E> {
    if !x.is_finite() {
        return Err(E::NotFinite);
    }
    if x != 0.0 && x.abs() < f64::MIN_POSITIVE {
        return Err(E::Subnormal);
    }
    if x.abs() > MAX_ABS_COORD {
        return Err(E::OutOfRange);
    }
    Ok(if x == 0.0 { 0.0 } else { x })
}

/// NP-18: refuses a count above `max`.
pub fn check_count(count: usize, max: usize) -> Result<(), E> {
    if count > max { Err(E::Limit) } else { Ok(()) }
}

impl Policy {
    /// NP-04: the tolerance for values of magnitude `mag` belonging to features of size `scale`.
    pub fn tol(&self, scale: f64, mag: f64) -> f64 {
        self.abs.max(self.rel * scale.abs()).max(f64::from(self.ulp) * ulp_of(mag))
    }

    /// NP-05: `a` and `b` are the same value at feature scale `scale`.
    pub fn near(&self, a: f64, b: f64, scale: f64) -> bool {
        if !(a.is_finite() && b.is_finite()) {
            return false;
        }
        a == b || (a - b).abs() <= self.tol(scale, a.abs().max(b.abs()))
    }

    /// NP-05 for points: both coordinates are `near`.
    pub fn near_point(&self, a: [f64; 2], b: [f64; 2], scale: f64) -> bool {
        self.near(a[0], b[0], scale) && self.near(a[1], b[1], scale)
    }

    /// NP-06: a length at or below the floor for magnitude `mag`. The relative term is left out on
    /// purpose, because a length has no feature larger than itself to be relative to.
    pub fn degenerate_length(&self, len: f64, mag: f64) -> bool {
        !len.is_finite() || len <= self.abs.max(f64::from(self.ulp) * ulp_of(mag))
    }

    /// NP-07: a radius is usable when finite, positive and not degenerate.
    pub fn check_radius(&self, r: f64, center_mag: f64) -> Result<(), E> {
        check_value(r)?;
        if r <= 0.0 || self.degenerate_length(r, center_mag.max(r)) {
            Err(E::DegenerateRadius)
        } else {
            Ok(())
        }
    }

    /// NP-07: both semi-axes usable and their ratio within the conditioning bound.
    pub fn check_semi_axes(&self, a: f64, b: f64, center_mag: f64) -> Result<(), E> {
        self.check_radius(a, center_mag)?;
        self.check_radius(b, center_mag)?;
        if a.max(b) / a.min(b) > self.axis_ratio_max { Err(E::DegenerateRadius) } else { Ok(()) }
    }

    /// NP-10: normalises a direction to unit length, or refuses a zero direction.
    pub fn normalise_direction(&self, dx: f64, dy: f64) -> Result<[f64; 2], E> {
        if !(dx.is_finite() && dy.is_finite()) {
            return Err(E::NotFinite);
        }
        let m = dx.abs().max(dy.abs());
        if m <= self.abs {
            return Err(E::ZeroDirection);
        }
        // Scale by the largest component first so squaring cannot overflow or underflow.
        let (sx, sy) = (dx / m, dy / m);
        let n = sx.hypot(sy);
        Ok([sx / n, sy / n])
    }

    /// NP-10: a stored direction is unit length within tolerance.
    pub fn is_unit(&self, d: [f64; 2]) -> bool {
        self.near(d[0].hypot(d[1]), 1.0, 1.0)
    }

    /// NP-09: orientation of `c` relative to `a -> b`, with a collinear band.
    ///
    /// The band is the larger of the policy tolerance times the longer edge (a distance tolerance
    /// turned into a determinant bound) and Shewchuk's static error bound for the f64 determinant,
    /// so a decision outside the band is the sign of the exact determinant.
    pub fn orient2d(&self, a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Orientation {
        let left = (a[0] - c[0]) * (b[1] - c[1]);
        let right = (a[1] - c[1]) * (b[0] - c[0]);
        let det = left - right;
        let u = 1.0 / (1u64 << 53) as f64;
        let err = (3.0 + 16.0 * u) * u * (left.abs() + right.abs());
        let l = (b[0] - a[0]).hypot(b[1] - a[1]).max((c[0] - a[0]).hypot(c[1] - a[1]));
        let mag = a[0].abs().max(a[1].abs()).max(b[0].abs()).max(b[1].abs()).max(c[0].abs()).max(c[1].abs());
        let band = err.max(self.tol(l, mag) * l);
        if det.abs() <= band {
            Orientation::Collinear
        } else if det > 0.0 {
            Orientation::Ccw
        } else {
            Orientation::Cw
        }
    }

    /// NP-08: classifies a control hull. A [`HullClass::Point`] hull is refused by
    /// [`Policy::check_hull`].
    pub fn hull_class(&self, pts: &[[f64; 2]]) -> HullClass {
        let Some(&p0) = pts.first() else { return HullClass::Point };
        let mag = pts.iter().fold(0.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
        let dist = |p: [f64; 2], q: [f64; 2]| (p[0] - q[0]).hypot(p[1] - q[1]);
        let far = pts.iter().copied().fold(p0, |best, p| if dist(p0, p) > dist(p0, best) { p } else { best });
        if self.degenerate_length(dist(p0, far), mag) {
            return HullClass::Point;
        }
        if pts.iter().all(|&c| self.orient2d(p0, far, c) == Orientation::Collinear) {
            HullClass::Linear
        } else {
            HullClass::Planar
        }
    }

    /// NP-08: refuses a hull that collapses to a point.
    pub fn check_hull(&self, pts: &[[f64; 2]]) -> Result<HullClass, E> {
        match self.hull_class(pts) {
            HullClass::Point => Err(E::DegenerateHull),
            other => Ok(other),
        }
    }

    /// NP-11 and NP-12 for a periodic basis of period `period`: turns `(t1, t2, sense)` into an
    /// increasing interval starting in `[0, period)` plus a direction flag.
    pub fn normalise_periodic_trim(&self, t1: f64, t2: f64, sense: bool, period: f64) -> Result<Trim, E> {
        let (t1, t2) = (check_value(t1)?, check_value(t2)?);
        let tp = self.tol(period, t1.abs().max(t2.abs()));
        let raw = if sense { t2 - t1 } else { t1 - t2 };
        if raw.abs() > period + tp {
            return Err(E::PeriodicSpan);
        }
        if raw.abs() <= tp {
            return Err(E::CollapsedTrim);
        }
        let span = if (raw - period).abs() <= tp {
            period
        } else if (raw + period).abs() <= tp {
            // A full turn written backwards is ambiguous with an empty one, so it is refused.
            return Err(E::PeriodicSpan);
        } else {
            raw.rem_euclid(period)
        };
        let mut start = (if sense { t1 } else { t2 }).rem_euclid(period);
        if start >= period {
            start = 0.0;
        }
        Ok(Trim { start, span, reversed: !sense })
    }

    /// NP-12 for a basis with domain `[lo, hi]` that is not periodic.
    pub fn normalise_trim(&self, t1: f64, t2: f64, sense: bool, lo: f64, hi: f64) -> Result<Trim, E> {
        let (t1, t2, lo, hi) = (check_value(t1)?, check_value(t2)?, check_value(lo)?, check_value(hi)?);
        let width = hi - lo;
        let tp = self.tol(width, lo.abs().max(hi.abs()).max(t1.abs()).max(t2.abs()));
        if width <= tp {
            return Err(E::EmptyDomain);
        }
        let inside = |t: f64| t >= lo - tp && t <= hi + tp;
        if !(inside(t1) && inside(t2)) {
            return Err(E::TrimOutsideDomain);
        }
        if (t2 - t1).abs() <= tp {
            return Err(E::CollapsedTrim);
        }
        if (t1 < t2) != sense {
            return Err(E::InconsistentSense);
        }
        let (a, b) = if sense { (t1, t2) } else { (t2, t1) };
        let (a, b) = (a.max(lo), b.min(hi));
        Ok(Trim { start: a, span: b - a, reversed: !sense })
    }

    /// NP-13: maps `t` in `[lo, hi]` onto `[0, 1]`.
    pub fn unit_param(&self, t: f64, lo: f64, hi: f64) -> Result<f64, E> {
        let width = hi - lo;
        if !width.is_finite() || width <= self.tol(width, lo.abs().max(hi.abs())) {
            return Err(E::EmptyDomain);
        }
        Ok((t - lo) / width)
    }

    /// NP-14: validates a knot vector and returns the evaluated domain `[knots[degree], knots[n]]`.
    pub fn validate_knots(&self, degree: usize, n_cp: usize, knots: &[f64]) -> Result<(f64, f64), E> {
        if !(1..=MAX_DEGREE).contains(&degree) {
            return Err(E::Limit);
        }
        if !(degree + 1..=MAX_CONTROL_POINTS).contains(&n_cp) {
            return Err(E::Limit);
        }
        if knots.len() != n_cp + degree + 1 {
            return Err(E::BadKnots);
        }
        for &k in knots {
            check_value(k)?;
        }
        if knots.windows(2).any(|w| w[1] < w[0]) {
            return Err(E::BadKnots);
        }
        let (lo, hi) = (knots[degree], knots[n_cp]);
        let mag = knots[0].abs().max(knots[knots.len() - 1].abs());
        let width = hi - lo;
        let tp = self.tol(width, mag);
        if width <= tp {
            return Err(E::EmptyDomain);
        }
        if knots.windows(2).any(|w| w[1] != w[0] && w[1] - w[0] <= tp) {
            return Err(E::AmbiguousKnots);
        }
        let mut i = 0;
        while i < knots.len() {
            let mut j = i;
            while j + 1 < knots.len() && knots[j + 1] == knots[i] {
                j += 1;
            }
            let mult = j - i + 1;
            let interior = knots[i] > lo && knots[i] < hi;
            if mult > degree + 1 || (interior && mult > degree) {
                return Err(E::BadKnots);
            }
            i = j + 1;
        }
        Ok((lo, hi))
    }

    /// NP-15: every weight finite and positive, and none below the floor relative to the largest.
    pub fn validate_weights(&self, w: &[f64]) -> Result<(), E> {
        let mut max = 0.0f64;
        for &x in w {
            check_value(x)?;
            if x <= 0.0 {
                return Err(E::BadWeight);
            }
            max = max.max(x);
        }
        if w.iter().any(|&x| x < self.weight_floor * max) { Err(E::BadWeight) } else { Ok(()) }
    }

    /// NP-17: the end of one segment meets the start of the next at feature scale `scale`.
    pub fn check_join(&self, end: [f64; 2], start: [f64; 2], scale: f64) -> Result<(), E> {
        if self.near_point(end, start, scale) { Ok(()) } else { Err(E::Gap) }
    }

    /// NP-20: a curve flagged closed must end where it starts.
    pub fn check_closed_flag(
        &self,
        closed: bool,
        start: [f64; 2],
        end: [f64; 2],
        scale: f64,
    ) -> Result<(), E> {
        if closed && !self.near_point(start, end, scale) { Err(E::ClosedMismatch) } else { Ok(()) }
    }

    /// NP-19: an affine map `[a, b, c, d, tx, ty]` (rows `[a b tx]`, `[c d ty]`) is usable.
    pub fn validate_affine(&self, m: [f64; 6]) -> Result<(), E> {
        for &x in &m {
            check_value(x)?;
        }
        let lin = [m[0], m[1], m[2], m[3]];
        if lin.iter().any(|x| x.abs() > MAX_AFFINE_ENTRY) {
            return Err(E::OutOfRange);
        }
        let big = lin.iter().fold(0.0f64, |a, x| a.max(x.abs()));
        let det = m[0] * m[3] - m[1] * m[2];
        if det.abs() <= self.rel * big * big { Err(E::SingularTransform) } else { Ok(()) }
    }
}

/// NP-16: highest derivative order that exists two-sided at a knot of multiplicity `multiplicity`
/// on a curve of degree `degree`. `None` means not even the curve itself is continuous there.
pub fn max_two_sided_order(degree: usize, multiplicity: usize) -> Option<usize> {
    degree.checked_sub(multiplicity)
}

/// NP-16: derivative requests at a knot. One-sided limits always exist inside the domain; the
/// two-sided derivative exists only up to [`max_two_sided_order`].
pub fn check_derivative_request(
    order: usize,
    degree: usize,
    multiplicity: usize,
    side: Side,
) -> Result<(), E> {
    match side {
        Side::Left | Side::Right => Ok(()),
        Side::Both => match max_two_sided_order(degree, multiplicity) {
            Some(k) if order <= k => Ok(()),
            _ => Err(E::TwoSidedAtBreak),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulp_matches_known_values() {
        assert_eq!(ulp_of(1.0), 1.0 / (1u64 << 52) as f64);
        assert_eq!(ulp_of(-1.0), ulp_of(1.0));
        assert_eq!(ulp_of(2.0), 2.0 * ulp_of(1.0));
    }

    #[test]
    fn negative_zero_is_canonicalised() {
        assert!(check_value(-0.0).unwrap().is_sign_positive());
    }
}
