//! Affine maps and the parameter maps that come out of transforming a curve.

use crate::Error;
use crate::{POLICY, Pt};

/// An invertible 2x3 affine map `[a b tx; c d ty]`, validated under NP-19.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    /// `[a, b, c, d, tx, ty]`.
    pub m: [f64; 6],
}

/// How the parameter of the transformed curve relates to the parameter of the original:
/// `t_new = offset + scale * t_old`. `scale` is `+-1` for conics and `|A d|` for a line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamMap {
    /// Added after scaling.
    pub offset: f64,
    /// Multiplier, never zero.
    pub scale: f64,
}

impl ParamMap {
    /// The identity map.
    pub const IDENTITY: ParamMap = ParamMap { offset: 0.0, scale: 1.0 };
    /// Applies the map.
    pub fn apply(&self, t: f64) -> f64 {
        self.offset + self.scale * t
    }
}

impl Affine {
    /// Validates and wraps a matrix.
    pub fn new(m: [f64; 6]) -> Result<Affine, Error> {
        POLICY.validate_affine(m)?;
        Ok(Affine { m })
    }
    /// Applies the full map to a point.
    pub fn point(&self, p: Pt) -> Pt {
        let m = &self.m;
        [m[0] * p[0] + m[1] * p[1] + m[4], m[2] * p[0] + m[3] * p[1] + m[5]]
    }
    /// Applies only the linear part, for vectors.
    pub fn vector(&self, v: Pt) -> Pt {
        let m = &self.m;
        [m[0] * v[0] + m[1] * v[1], m[2] * v[0] + m[3] * v[1]]
    }
    /// Determinant of the linear part.
    pub fn det(&self) -> f64 {
        self.m[0] * self.m[3] - self.m[1] * self.m[2]
    }
}
