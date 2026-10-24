//! Translation fidelity for PVec exports (issue P6.1) and the GeoPackage export (issue P6.5).
//!
//! Every export has to say what it did to the geometry, and "exported successfully" is not that.
//! This crate defines the five-value vocabulary once ([`Fidelity`]), the record an export writes per
//! entity ([`FidelityRecord`]) and the [`FidelityLedger`] it accumulates, so every later export and
//! report cites the same words. The normative text is `docs/FIDELITY_TAXONOMY.md`. The GeoPackage
//! writer ([`gpkg`]) is the first export built on it; `docs/INTERCHANGE.md` chapter 4 says which curve
//! becomes which geometry and with what claim.
//!
//! The crate has no dependencies beyond the curve model, the capability registry and the entity id,
//! no I/O and no `unsafe`. The serialized form of a ledger is a small versioned text format written by
//! hand, and the GeoPackage is a SQLite file written by hand too, so no serialization crate and no
//! SQLite library (and so no C source) ever shows up in a public signature or in the build.

mod fidelity;
pub mod gpkg;

pub use fidelity::*;
