//! The STEP AP242 / Part 21 reference export (issue P6.2).
//!
//! PVec curves go out as an ISO 10303-21:2016 file whose entities are the ISO 10303-42 constructs the
//! semantic profile (`docs/ISO_PROFILE.md`) maps each capability onto. The parameters are taken straight
//! from the curve model, so what a STEP reader gets back is the mathematics and not a picture of it.
//! `docs/INTERCHANGE.md` chapter 1 has the mapping table, generated from [`MAPPING`].

mod mapping;
mod writer;

pub use mapping::*;
pub use writer::*;
pub mod verify;
