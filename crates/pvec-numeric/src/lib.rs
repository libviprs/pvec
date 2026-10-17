//! Numeric policy for PVec (rules `NP-01` to `NP-20` of `docs/NUMERIC_POLICY.md`).
//!
//! This crate is deliberately tiny and dependency free. It defines when two numbers are the same,
//! when a shape is degenerate, which values are refused at parse time and how big anything may be.
//! It does not evaluate curves.

pub mod error;
pub mod policy;
pub mod predicates;

pub use error::NumericError;
pub use policy::*;
pub use predicates::*;
