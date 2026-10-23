//! Independent component streams (issue P3.7): layer, line style, text, hatch, source provenance and
//! reserved space for future DWG semantics.
//!
//! Style, text and hatch evolve on a different schedule from the geometry mathematics, so they live
//! outside the geometry oracle. Each stream has its own registry capability id and semantic version
//! (`pvec-capabilities`), is optional by construction, refers to entities by `EntityId` and never embeds
//! a curve, and is framed so that a stream the reader does not know is stepped over by its declared
//! length without its payload being read ([`read_streams`]).
//!
//! This crate deliberately depends on neither `pvec-geometry` nor `pvec-codec`: nothing in it can reach a
//! curve, which is why a stream cannot change a geometry verdict. The tests prove the independence per
//! stream anyway. The byte layout is in `docs/COMPONENT_STREAMS.md`.

mod error;
mod frame;
mod payloads;
mod semantic;

pub use error::{ComponentError, StreamError};
pub use frame::{MAX_STREAMS, Opened, SkipReason, Skipped, Stream, encode_streams, read_streams};
pub use payloads::{
    Fill, HatchRecord, HatchStream, LayerStream, LineStyleDef, LineStyleStream, MAX_BOUNDARY, MAX_DASHES,
    MAX_HANDLE, MAX_NAME, MAX_RECORDS, MAX_TABLE, MAX_TEXT, ProvenanceRecord, ProvenanceStream,
    ReservedStream, TextRecord, TextStream,
};
pub use semantic::{
    ElementClass, ElementEntry, EntityGeometry, MAX_HOLES, Origin, RegionError, RegionKind, RegionRecord,
    SemanticClassStream, SourceTag,
};
