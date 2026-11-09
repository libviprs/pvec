//! The component stream payloads. Each carries its own stream version, which is how ADR 0014 lets
//! them evolve without touching the geometry core.

pub use pvec_components::{
    ElementClass, ElementEntry, EntityGeometry, Fill, FillAssignment, FillPaintStream, FillRule, FillSource,
    Gradient, GradientCoords, HatchStream, ItemKind, LAYER_FROZEN, LAYER_LOCKED, LAYER_OFF, LayerStream,
    LineStyleDef, LineStyleStream, Origin, PaintOrderStream, PaintRun, ProvenanceRecord, ProvenanceStream,
    RegionError, RegionKind, RegionRecord, SemanticClassStream, SkipReason, Skipped, SourceTag, Stop, Stream,
    StrokePaintStream, StrokeWidth, TextStream, Tiling, TilingPrimitive,
};
