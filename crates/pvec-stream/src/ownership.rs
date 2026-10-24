//! Single payload ownership with duplicated spatial references (issue P5.2).
//!
//! Skeleton: the types and signatures only, so the tests of pvec#11 can be written first and fail.

use crate::directory::{Aabb, CellId, SpatialDirectory};
use pvec_container::blocks::hot::HotBlockView;
use pvec_container::directory::BlockEntry;
use pvec_container::source::BlockSource;
use pvec_container::{BlockId, ContainerReader};
use pvec_entity::EntityId;

/// The most bytes a spatial reference may take.
pub const REF_BUDGET: usize = 8;

/// Bytes of one encoded reference.
pub const PAYLOAD_REF_LEN: usize = 8;

/// A reference to one entity's payload: the hot block that holds it and its row in that block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadRef {
    /// The block.
    pub block_id: BlockId,
    /// The row inside the block.
    pub slot: u32,
}

const _: () = assert!(core::mem::size_of::<PayloadRef>() <= REF_BUDGET);

impl PayloadRef {
    /// The 8 byte encoding.
    pub fn encode(self) -> [u8; PAYLOAD_REF_LEN] {
        unimplemented!()
    }

    /// The reference an encoding holds.
    pub fn decode(_bytes: [u8; PAYLOAD_REF_LEN]) -> PayloadRef {
        unimplemented!()
    }
}

/// The most cells an entity may be referenced from unless the writer says otherwise.
pub const DEFAULT_MAX_CELLS: u32 = 16;

/// The refusals, rule by rule.
pub const OWNERSHIP_RULES: &[(&str, &str)] = &[];

/// Why a placement, a reference array or a read was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum OwnershipError {
    /// A rule.
    Rule {
        /// The rule id.
        rule: &'static str,
    },
}

impl OwnershipError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            OwnershipError::Rule { rule } => rule,
        }
    }
}

impl core::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.rule())
    }
}

impl std::error::Error for OwnershipError {}

/// How many cells one entity may be referenced from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuplicationPolicy {
    max_cells: u32,
}

impl DuplicationPolicy {
    /// A ceiling of `max_cells`, which is at least 1.
    pub fn new(_max_cells: u32) -> Result<DuplicationPolicy, OwnershipError> {
        unimplemented!()
    }

    /// The ceiling.
    pub fn max_cells(&self) -> u32 {
        self.max_cells
    }
}

impl Default for DuplicationPolicy {
    fn default() -> DuplicationPolicy {
        DuplicationPolicy { max_cells: DEFAULT_MAX_CELLS }
    }
}

/// One entity as the placement pass sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacedEntity {
    /// The entity.
    pub id: EntityId,
    /// Its bounds in source space.
    pub bounds: Aabb,
    /// Where its payload is.
    pub payload: PayloadRef,
}

/// An entity that intersects more cells than the ceiling: it is P5.3's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Straddler {
    /// The entity.
    pub id: EntityId,
    /// Where its payload is.
    pub payload: PayloadRef,
}

/// The references of every cell.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CellRefs {
    cells: Vec<u32>,
    ends: Vec<u32>,
    refs: Vec<PayloadRef>,
}

impl CellRefs {
    /// The cells that have references, ascending.
    pub fn cells(&self) -> usize {
        self.cells.len()
    }

    /// All references, in every cell.
    pub fn total_refs(&self) -> usize {
        self.refs.len()
    }

    /// The references of one cell, ascending, or none.
    pub fn refs_of(&self, _cell: CellId) -> &[PayloadRef] {
        unimplemented!()
    }

    /// The bytes of the reference arrays.
    pub fn encode(&self) -> Vec<u8> {
        unimplemented!()
    }

    /// The reference arrays of `bytes`, checked.
    pub fn parse(_bytes: &[u8]) -> Result<CellRefs, OwnershipError> {
        unimplemented!()
    }
}

/// What the duplication cost.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplicationReport {
    /// Entities that got references.
    pub entities: u64,
    /// Entities routed to the straddler path.
    pub straddlers: u64,
    /// References emitted.
    pub references: u64,
    /// Mean references per entity.
    pub mean: f64,
    /// 99th percentile (nearest rank).
    pub p99: u32,
    /// Most references of one entity.
    pub max: u32,
    /// `references` times the reference size.
    pub reference_bytes: u64,
    /// Decoded bytes of the hot blocks.
    pub payload_bytes: u64,
}

impl DuplicationReport {
    /// Reference bytes over payload bytes.
    pub fn fraction(&self) -> f64 {
        unimplemented!()
    }
}

/// The outcome of the placement pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The per-cell arrays.
    pub refs: CellRefs,
    /// Entities over the ceiling.
    pub straddlers: Vec<Straddler>,
    /// The accounting.
    pub report: DuplicationReport,
}

/// Assigns references: each entity's payload stays where it is, and its reference goes into every cell
/// its bounds intersect, unless that is more than the ceiling.
pub fn place(
    _dir: &SpatialDirectory,
    _entities: &[PlacedEntity],
    _policy: DuplicationPolicy,
    _payload_bytes: u64,
) -> Result<Placement, OwnershipError> {
    unimplemented!()
}

/// Decoded bytes of the hot blocks in `entries`.
pub fn payload_bytes(_entries: &[BlockEntry]) -> u64 {
    unimplemented!()
}

/// One payload slot found in a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PayloadSlot {
    /// The entity in it.
    pub id: EntityId,
    /// Its location.
    pub payload: PayloadRef,
    /// The cell of its block.
    pub cell: u32,
}

/// Every payload slot of every hot block, in block order.
pub fn scan_payloads<S: BlockSource>(_reader: &ContainerReader<S>) -> Result<Vec<PayloadSlot>, OwnershipError> {
    unimplemented!()
}

/// The references a viewport touches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportRefs {
    /// References before deduplication.
    pub raw: usize,
    /// Each payload once, ascending.
    pub refs: Vec<PayloadRef>,
}

/// The references of every cell the viewport touches, each payload once.
pub fn gather(_dir: &SpatialDirectory, _refs: &CellRefs, _viewport: &Aabb) -> ViewportRefs {
    unimplemented!()
}

/// Decodes each payload of `gathered` once, block by block, and calls `visit` with the entity, the block
/// and the row. Returns how many payloads were visited.
pub fn read_viewport<S: BlockSource>(
    _reader: &ContainerReader<S>,
    _gathered: &ViewportRefs,
    _visit: impl FnMut(EntityId, &HotBlockView<'_>, usize),
) -> Result<usize, OwnershipError> {
    unimplemented!()
}
