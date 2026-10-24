//! Translation fidelity for PVec exports (issue P6.1).
//!
//! Every export has to say what it did to the geometry, and "exported successfully" is not that.
//! This crate defines the five-value vocabulary once ([`Fidelity`]), the record an export writes per
//! entity ([`FidelityRecord`]) and the [`FidelityLedger`] it accumulates, so every later export and
//! report cites the same words. The normative text is `docs/FIDELITY_TAXONOMY.md`.
//!
//! The crate has no dependencies beyond the capability registry and the entity id, no I/O and no
//! `unsafe`. The serialized form of a ledger is a small versioned text format written by hand, so
//! no serialization crate ever shows up in a public signature.

mod fidelity;

pub use fidelity::*;
