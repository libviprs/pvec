//! Translation fidelity for PVec exports (issue P6.1).
//!
//! Every export has to say what it did to the geometry, and "exported successfully" is not that.
//! This crate defines the five-value vocabulary once ([`Fidelity`]), the record an export writes per
//! entity ([`FidelityRecord`]) and the [`FidelityLedger`] it accumulates, so every later export and
//! report cites the same words. The normative text is `docs/FIDELITY_TAXONOMY.md`.
//!
//! The GML export (P6.4) lives in [`gml`]: it maps PVec curves onto GML 3.2.1 constructs and writes one
//! fidelity record per segment, see `docs/INTERCHANGE.md` chapter 3.
//!
//! The display-only vector tile export (P6.6) lives in [`mvt`]; it is the one place that reads curves,
//! through the exact evaluator of `pvec-geometry`.
//!
//! The crate has no I/O and no `unsafe`; it depends on the capability registry, the entity id and the
//! curve model. The serialized form of a ledger is a small versioned text format written by hand, so
//! no serialization crate ever shows up in a public signature.

mod fidelity;
pub mod gml;
pub mod mvt;

pub use fidelity::*;
