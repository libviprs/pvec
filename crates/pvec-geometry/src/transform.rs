//! The affine map, and the parameter maps that come out of applying it to a curve.

use crate::error::Result;
use crate::primitives::{Point2, Vector2};
use pvec_numeric::Policy;

/// An invertible 2D affine map `[a b tx; c d ty]`, validated under NP-19.
///
/// A `Transform2` that exists is usable: its six numbers are finite and normal, the translation is
/// inside the coordinate bound, the linear entries are inside their bound and the determinant is not
/// singular.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform2 {
    m: [f64; 6],
}

impl Transform2 {
    /// The identity.
    pub const IDENTITY: Transform2 = Transform2 { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };

    /// Validates `[a, b, c, d, tx, ty]` (rows `[a b tx]` and `[c d ty]`) under NP-19.
    pub fn new(m: [f64; 6]) -> Result<Transform2> {
        Policy::V1.validate_affine(m)?;
        // Negative zero is stored as positive zero (NP-03), so equal maps have equal bits.
        let mut m = m;
        for x in &mut m {
            if *x == 0.0 {
                *x = 0.0;
            }
        }
        Ok(Transform2 { m })
    }

    /// A translation.
    pub fn translate(dx: f64, dy: f64) -> Result<Transform2> {
        Transform2::new([1.0, 0.0, 0.0, 1.0, dx, dy])
    }

    /// A counter-clockwise rotation about the origin, in radians.
    pub fn rotate(radians: f64) -> Result<Transform2> {
        let (s, c) = radians.sin_cos();
        Transform2::new([c, -s, s, c, 0.0, 0.0])
    }

    /// A scale along the axes. A zero factor is a singular map and is refused (NP-19).
    pub fn scale(sx: f64, sy: f64) -> Result<Transform2> {
        Transform2::new([sx, 0.0, 0.0, sy, 0.0, 0.0])
    }

    /// The reflection `y -> -y` across the x axis.
    pub fn mirror_x() -> Result<Transform2> {
        Transform2::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0])
    }

    /// The reflection `x -> -x` across the y axis.
    pub fn mirror_y() -> Result<Transform2> {
        Transform2::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0])
    }

    /// A shear: `x' = x + shx * y`, `y' = y + shy * x`.
    pub fn shear(shx: f64, shy: f64) -> Result<Transform2> {
        Transform2::new([1.0, shx, shy, 1.0, 0.0, 0.0])
    }

    /// The map that applies `self` first and `next` second. The result is validated like any other map
    /// (NP-19), so a composition that leaves the coordinate range or loses its determinant is refused
    /// instead of wrapping or collapsing.
    pub fn then(&self, next: &Transform2) -> Result<Transform2> {
        let (a, b) = (&self.m, &next.m);
        Transform2::new([
            b[0] * a[0] + b[1] * a[2],
            b[0] * a[1] + b[1] * a[3],
            b[2] * a[0] + b[3] * a[2],
            b[2] * a[1] + b[3] * a[3],
            b[0] * a[4] + b[1] * a[5] + b[4],
            b[2] * a[4] + b[3] * a[5] + b[5],
        ])
    }

    /// The map that undoes this one, validated like any other (NP-19).
    pub fn inverse(&self) -> Result<Transform2> {
        let m = &self.m;
        let det = self.determinant();
        let (ia, ib, ic, id) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
        Transform2::new([ia, ib, ic, id, -(ia * m[4] + ib * m[5]), -(ic * m[4] + id * m[5])])
    }

    /// The six numbers `[a, b, c, d, tx, ty]`.
    pub fn matrix(&self) -> [f64; 6] {
        self.m
    }

    /// Applies the whole map to a point.
    pub fn apply_point(&self, p: Point2) -> Point2 {
        let m = &self.m;
        Point2::new(m[0] * p.x + m[1] * p.y + m[4], m[2] * p.x + m[3] * p.y + m[5])
    }

    /// Applies only the linear part, which is what a vector needs.
    pub fn apply_vector(&self, v: Vector2) -> Vector2 {
        let m = &self.m;
        Vector2::new(m[0] * v.x + m[1] * v.y, m[2] * v.x + m[3] * v.y)
    }

    /// The determinant of the linear part; negative when the map reverses handedness.
    pub fn determinant(&self) -> f64 {
        self.m[0] * self.m[3] - self.m[1] * self.m[2]
    }
}

/// How the parameter of a transformed curve relates to the parameter of the original:
/// `t_new = offset + scale * t_old`.
///
/// `scale` is `+1` or `-1` for conics (the image is re-expressed with orthogonal axes), the length of
/// the image of the direction for a line, and `+1` for every other kind. It is never zero.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamMap {
    /// Added after scaling.
    pub offset: f64,
    /// The multiplier.
    pub scale: f64,
}

impl ParamMap {
    /// The identity map.
    pub const IDENTITY: ParamMap = ParamMap { offset: 0.0, scale: 1.0 };

    /// Maps an old parameter to the new curve's parameter.
    pub fn apply(&self, t: f64) -> f64 {
        self.offset + self.scale * t
    }
}
