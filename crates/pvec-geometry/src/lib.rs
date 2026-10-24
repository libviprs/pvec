//! The source-neutral PVec curve model (issue P3.1).
//!
//! This crate is the normative semantic layer of PVec: the curve types of `docs/ISO_PROFILE.md`,
//! validated at construction under the numeric policy (`docs/NUMERIC_POLICY.md`, implemented by
//! `pvec-numeric`), with one exact evaluator that every consumer shares. There is no second
//! evaluator anywhere in the four repositories.
//!
//! # Boundaries
//!
//! * No serialization-generated or wire type appears in any public signature, and the crate cannot
//!   name one: its only dependency is `pvec-numeric`. A repository check scans the public API.
//! * No file or network I/O, no container knowledge and no source-format knowledge. A DWG `ARC` is
//!   not a type here; it is a [`Trimmed`] over a [`Circle`].
//! * No `unsafe`, and nothing here approximates: [`Curve2d::transform`] returns the exact image or
//!   a typed refusal, and no path calls a tessellator.
//!
//! # Validity
//!
//! A curve that exists is valid. Every constructor returns a [`GeometryError`] that names the rule it
//! enforced (see [`GeometryError::rule`]), and no refused input can be turned into a curve. Evaluation
//! never re-validates, and a successful call never returns a non-finite number.

// The spline kernels index several small fixed arrays with one counter; rewriting them as iterator
// chains would hide the recurrences they implement.
#![allow(clippy::needless_range_loop)]

mod conic;
mod curve;
mod error;
mod primitives;
mod spline;
mod transform;

mod sealed {
    pub trait Sealed {}
}
pub(crate) use sealed::Sealed;

pub use conic::{Circle, Ellipse, Line, Placement2};
pub use curve::{Composite, Curve, Curve2d, Polyline, Trimmed};
pub use error::GeometryError;
pub use primitives::{Bounds, CurveKind, Domain, Point2, Vector2};
pub use pvec_numeric::{Side, Trim};
pub use spline::{BSpline, Bezier, RationalBSpline, Unwrapped};
pub use transform::{ParamMap, Transform2};
