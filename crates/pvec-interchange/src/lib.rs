//! Translation fidelity for PVec exports (issue P6.1).
//!
//! Every export has to say what it did to the geometry, and "exported successfully" is not that.
//! This crate defines the five-value vocabulary once ([`Fidelity`]), the record an export writes per
//! entity ([`FidelityRecord`]) and the [`FidelityLedger`] it accumulates, so every later export and
//! report cites the same words. The normative text is `docs/FIDELITY_TAXONOMY.md`.
//!
//! The display-only vector tile export (P6.6) lives in [`mvt`]; it is the one place that reads curves,
//! through the exact evaluator of `pvec-geometry`.
//!
//! The crate has no dependencies beyond the capability registry, the entity id and the curve model,
//! no I/O and no `unsafe`. The serialized form of a ledger is a small versioned text format written by hand, so
//! no serialization crate ever shows up in a public signature.

mod fidelity;
pub mod mvt;

pub use fidelity::*;
