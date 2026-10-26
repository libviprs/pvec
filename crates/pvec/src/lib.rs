//! The PVec facade (issue P7.1): the semantic reader and writer surface, and nothing of the encoding.
//!
//! A consumer compiles against curve semantics here, not against the block layout of
//! `pvec-container`. The frozen-intent core is [`PvecReader`], [`PvecWriter`], [`Manifest`],
//! [`CapabilityId`], [`EntityId`], [`Curve`], [`CurveRef`], [`Domain`], [`Bounds2`], [`Transform2`] and
//! [`PvecError`], plus the component payloads under [`components`]. It is frozen at P8.7, and until then
//! the snapshot in `public-api.txt` is what makes any change loud. The streaming types sit behind the
//! `unstable-stream` feature in `unstable` and have their own snapshot. Nothing from `pvec-container`
//! is re-exported; `docs/API.md` has the semver rule for each part.
#![deny(missing_docs)]

mod bounds;
mod error;
mod eval;
mod hit;
mod manifest;
mod pick;
mod reader;
mod tessellate;
mod writer;

pub mod components;
#[cfg(feature = "unstable-stream")]
pub mod unstable;

pub use bounds::{Bounds2, CurveRef};
pub use error::PvecError;
pub use eval::CurveEval;
pub use manifest::Manifest;
pub use pick::{EntityIdBatch, Pick, PickIndex, PickTolerance, RectMode};
pub use pvec_capabilities::CapabilityId;
pub use pvec_entity::EntityId;
pub use pvec_geometry::{Curve, Domain, Point2, Side, Transform2};
pub use reader::PvecReader;
pub use tessellate::{Conformance, Tessellate, Tessellation, TessellationContext};
pub use writer::PvecWriter;
