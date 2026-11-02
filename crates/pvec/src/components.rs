//! The component stream payloads. Each carries its own stream version, which is how ADR 0014 lets
//! them evolve without touching the geometry core.

pub use pvec_components::{
    Fill, HatchStream, LayerStream, LineStyleStream, ProvenanceStream, SemanticClassStream, SkipReason,
    Skipped, Stream, TextStream,
};
