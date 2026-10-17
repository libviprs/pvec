//! P1.3 spike: an ISO-aligned in-memory geometry API with no serialization type in it.
//!
//! Throwaway. What should survive into P3.1 is the trait shape and the storage decision recorded in
//! `docs/spikes/01-3-geometry-api.md`, not this code.
#![allow(clippy::needless_range_loop)]

pub mod affine;
pub mod conic;
pub mod curve;
pub mod fixtures;
pub mod spline;

pub use affine::{Affine, ParamMap};
pub use conic::{Circle, Ellipse, Frame, Line};
pub use curve::{Composite, Curve, Polyline, Trimmed};
pub use pvec_numeric::{NumericError, Policy, Side};
pub use spline::{BSpline, Bezier, Cps, Flat, RationalBSpline};

/// A 2D point or vector.
pub type Pt = [f64; 2];

/// Everything a geometry operation can refuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// A numeric policy rule refused the input.
    Numeric(NumericError),
    /// The parameter is outside the curve's domain.
    Domain,
    /// The operation needs a bounded curve.
    Unbounded,
}

impl From<NumericError> for Error {
    fn from(e: NumericError) -> Self {
        Error::Numeric(e)
    }
}

/// The policy every operation in the spike uses.
pub const POLICY: Policy = Policy::V1;

/// Axis-aligned bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Lower corner.
    pub min: Pt,
    /// Upper corner.
    pub max: Pt,
}

impl Bounds {
    pub(crate) fn of(p: Pt) -> Bounds {
        Bounds { min: p, max: p }
    }
    pub(crate) fn add(&mut self, p: Pt) {
        for k in 0..2 {
            self.min[k] = self.min[k].min(p[k]);
            self.max[k] = self.max[k].max(p[k]);
        }
    }
    pub(crate) fn union(mut self, o: Bounds) -> Bounds {
        self.add(o.min);
        self.add(o.max);
        self
    }
    /// Diagonal length.
    pub fn diagonal(&self) -> f64 {
        (self.max[0] - self.min[0]).hypot(self.max[1] - self.min[1])
    }
    /// Largest coordinate magnitude, the `mag` argument of the tolerance formula.
    pub fn magnitude(&self) -> f64 {
        self.min.iter().chain(self.max.iter()).fold(0.0, |m, v| m.max(v.abs()))
    }
}

/// The trait shape proposed for P3.1.
///
/// Only `std` and `pvec-numeric` types appear in it. A serialization crate can depend on this
/// crate to build curves, but this crate never depends on one.
pub trait Curve2d {
    /// The parameter domain `(lo, hi)`, `lo < hi`.
    fn domain(&self) -> (f64, f64);
    /// The point at parameter `t`.
    fn eval(&self, t: f64) -> Result<Pt, Error>;
    /// The derivative of `order` (>= 1) at `t`, from `side`. A two-sided request where only
    /// one-sided limits exist is refused (NP-16).
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error>;
    /// Closed-form bounds where the curve admits them, else a guaranteed enclosure.
    fn bounds(&self) -> Result<Bounds, Error>;
    /// The exact image under an affine map, with the map from old parameters to new ones.
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error>;
}
