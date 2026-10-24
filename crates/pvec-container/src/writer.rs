//! The deterministic container writer (issue P4.4).

use crate::blocks::advanced::SplineSpec;
use crate::blocks::arena::ArenaError;
use crate::blocks::hot::HotError;
use crate::error::ContainerError;
use crate::manifest::{Bounds, LengthUnit, Metadata};
use core::fmt;
use pvec_capabilities::CapabilityId;
use pvec_components::{Stream, StreamError};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, GeometryError};

/// A key of the canonical ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Blocks by block kind.
    BlockKind,
    /// Blocks by spatial cell id.
    SpatialCell,
    /// Blocks by geometry type.
    GeometryType,
    /// Entities in a block by `EntityId`.
    EntityId,
    /// Manifest capabilities by capability id.
    CapabilityId,
    /// Component streams by stream id.
    StreamId,
}

impl SortKey {
    /// Every key.
    pub const ALL: [SortKey; 6] = [
        SortKey::BlockKind,
        SortKey::SpatialCell,
        SortKey::GeometryType,
        SortKey::EntityId,
        SortKey::CapabilityId,
        SortKey::StreamId,
    ];
}

/// What the caller decides about an artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct WriterConfig {
    /// Unit of length of source space.
    pub unit: LengthUnit,
    /// Producer identity and corpus revision, as the caller supplies them.
    pub metadata: Metadata,
    /// Source bounds; the union of the entity bounds when `None`.
    pub bounds: Option<Bounds>,
    /// Capabilities to declare, in any order.
    pub capabilities: Vec<CapabilityId>,
}

/// What every entity carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityMeta {
    /// The entity id.
    pub id: EntityId,
    /// Opaque style reference.
    pub style: u32,
    /// Spatial cell id.
    pub cell: u32,
    /// Bounds of the entity in source space.
    pub bounds: Bounds,
}

/// Why writing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError {
    /// The writer is not implemented yet.
    NotImplemented,
    /// There is no entity to write.
    Empty,
    /// An entity id is used twice.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// An advanced curve was given a spatial cell.
    CellOnAdvanced {
        /// The id.
        id: u32,
    },
    /// A capability id the registry does not know.
    UnknownCapability {
        /// The id.
        id: u32,
    },
    /// Two component streams of one capability.
    DuplicateStream {
        /// The capability id.
        capability: u32,
    },
    /// Bounds that are not finite, in range and ordered.
    BadBounds {
        /// The entity, or `None` for the source bounds.
        id: Option<u32>,
    },
    /// A block or the file does not fit its length fields.
    TooLarge,
    /// A curve the writer cannot store.
    Unsupported {
        /// What it was.
        what: &'static str,
    },
    /// A hot block builder refused.
    Hot(HotError),
    /// An advanced block builder refused.
    Arena(ArenaError),
    /// The component stream encoder refused.
    Stream(StreamError),
    /// The manifest refused.
    Container(ContainerError),
    /// The semantic model refused.
    Geometry(GeometryError),
}

impl WriteError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        "WRT-00"
    }
}

/// The refusals, rule by rule.
pub const WRITE_RULES: &[(&str, &str)] = &[];

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "writer not implemented")
    }
}

impl std::error::Error for WriteError {}

/// The writer.
#[derive(Debug)]
pub struct ContainerWriter {
    _config: WriterConfig,
}

impl ContainerWriter {
    /// A writer with nothing in it.
    pub fn new(config: WriterConfig) -> ContainerWriter {
        ContainerWriter { _config: config }
    }

    /// Switches one sort off, to check that the determinism tests notice.
    #[doc(hidden)]
    pub fn skip_sort(&mut self, _key: SortKey) {}

    /// Adds a line segment.
    pub fn push_line(&mut self, _meta: EntityMeta, _a: [f64; 2], _b: [f64; 2]) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a polyline.
    pub fn push_polyline(&mut self, _meta: EntityMeta, _points: &[[f64; 2]]) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a circle or circular arc.
    pub fn push_circle_arc(
        &mut self,
        _meta: EntityMeta,
        _centre: [f64; 2],
        _radius: f64,
        _start_sweep: [f64; 2],
    ) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds an ellipse or elliptical arc.
    pub fn push_ellipse_arc(
        &mut self,
        _meta: EntityMeta,
        _centre: [f64; 2],
        _radii: [f64; 2],
        _angles: [f64; 3],
    ) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a B-spline.
    pub fn push_spline(&mut self, _meta: EntityMeta, _spec: SplineSpec<'_>) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a composite curve.
    pub fn push_composite(
        &mut self,
        _meta: EntityMeta,
        _segments: &[EntityId],
        _trims: Option<&[[f64; 2]]>,
        _closed: bool,
    ) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a curve of the semantic model.
    pub fn push_curve(
        &mut self,
        _id: EntityId,
        _style: u32,
        _cell: u32,
        _curve: &Curve,
    ) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// Adds a component stream.
    pub fn push_stream(&mut self, _stream: Stream) -> Result<(), WriteError> {
        Err(WriteError::NotImplemented)
    }

    /// The artifact.
    pub fn finish(self) -> Result<Vec<u8>, WriteError> {
        Err(WriteError::NotImplemented)
    }
}
