//! The semantic class stream (P3.9). RED STUB: the types and signatures are final, the behaviour is not
//! written yet, so the tests for the acceptance criteria fail on assertions.

use crate::error::ComponentError as E;
use crate::frame::Stream;
use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;

/// Most holes in one region.
pub const MAX_HOLES: usize = 256;

/// What a drawn element is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementClass {
    /// Not classified.
    Other,
    /// A wall.
    Wall,
    /// A column.
    Column,
}

/// What a region is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// A room.
    Room,
    /// A corridor.
    Corridor,
}

/// How an entry came to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTag {
    /// A layer rule.
    LayerRule,
    /// A block rule.
    BlockRule,
    /// Inferred.
    Inferred,
    /// Stated explicitly.
    Explicit,
}

/// The source tag of an entry and, for an inferred one, an optional confidence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    /// The tag.
    pub source: SourceTag,
    /// Confidence in 0 to 1, only with `Inferred`.
    pub confidence: Option<f64>,
}

/// A class on an existing entity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementEntry {
    /// The entity.
    pub entity: EntityId,
    /// Its class.
    pub class: ElementClass,
    /// Where the class came from.
    pub origin: Origin,
}

/// A region bounded by references to entities.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionRecord {
    /// Room or corridor.
    pub kind: RegionKind,
    /// Where the region came from.
    pub origin: Origin,
    /// The outer ring.
    pub outer: Vec<EntityId>,
    /// Holes.
    pub holes: Vec<Vec<EntityId>>,
}

/// What a reader can say about the entities a stream points at.
pub trait EntityGeometry {
    /// Start and end of the entity, or `None` when there is no such entity.
    fn endpoints(&self, entity: EntityId) -> Option<([f64; 2], [f64; 2])>;
}

/// Why a stream failed validation against the geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionError {
    /// A reference to an entity that does not exist.
    MissingEntity(EntityId),
    /// A ring that does not close.
    NotClosed {
        /// Index of the region.
        region: usize,
    },
}

/// The semantic class stream.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticClassStream {
    /// Classes on entities.
    pub elements: Vec<ElementEntry>,
    /// Regions.
    pub regions: Vec<RegionRecord>,
}

impl SemanticClassStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::SEMANTIC_CLASS;

    /// The payload bytes.
    pub fn encode(&self) -> Result<Vec<u8>, E> {
        Err(E::Truncated)
    }

    /// Reads a payload.
    pub fn decode(_bytes: &[u8]) -> Result<SemanticClassStream, E> {
        Err(E::Truncated)
    }

    /// The stream.
    pub fn into_stream(&self, _registry: &Registry) -> Result<Stream, E> {
        Err(E::Truncated)
    }

    /// The entities with this class.
    pub fn entities_of(&self, _class: ElementClass) -> Vec<EntityId> {
        Vec::new()
    }

    /// The regions of this kind.
    pub fn regions_of(&self, _kind: RegionKind) -> Vec<&RegionRecord> {
        Vec::new()
    }

    /// Checks the stream against the geometry it points at.
    pub fn validate(&self, _geometry: &dyn EntityGeometry, _tolerance: f64) -> Result<(), RegionError> {
        Ok(())
    }
}
