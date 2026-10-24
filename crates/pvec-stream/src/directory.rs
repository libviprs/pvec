//! The spatial directory (issue P5.1). Stubs: the tests are written first and are red.

use pvec_container::directory::BlockEntry;
use pvec_container::reader::ReadError;
use pvec_container::source::BlockSource;
use pvec_container::{BlockId, ContainerReader, Limits};
use pvec_entity::EntityId;
use std::num::NonZeroU32;

/// Leaf capacity: entities per cell (P1.6: 64 on the small leaf setting, 512 on the large).
pub const CELL_CAPACITY: u32 = 64;
/// Children per internal node.
pub const FANOUT: u32 = 32;
/// The most children a file may declare.
pub const MAX_FANOUT: u32 = 256;
/// The tallest tree a file may declare.
pub const MAX_HEIGHT: u32 = 8;
/// Bytes of the block header.
pub const HEADER_LEN: usize = 64;
/// Bytes of one node.
pub const NODE_LEN: usize = 48;

/// How a node's entities are cut into cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitPolicy {
    /// Sort by the Hilbert key of the bounds centre, then cut every `CELL_CAPACITY`.
    HilbertPack,
}

/// The split policy in use.
pub const SPLIT_POLICY: SplitPolicy = SplitPolicy::HilbertPack;

/// The refusals, rule by rule.
pub const DIRECTORY_RULES: &[(&str, &str)] = &[];

/// Why a directory was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DirectoryError {
    /// Not implemented yet.
    Todo,
    /// A P4.8 ceiling.
    Limit(pvec_container::LimitExceeded),
    /// The container refused.
    Read(ReadError),
}

impl DirectoryError {
    /// The rule identifier of `docs/STREAMING.md`.
    pub fn rule(&self) -> &'static str {
        "SPD-00"
    }
}

impl core::fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for DirectoryError {}

/// An axis-aligned box in source space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    min: [f64; 2],
    max: [f64; 2],
}

impl Aabb {
    /// A box from two corners.
    pub fn new(_min: [f64; 2], _max: [f64; 2]) -> Result<Aabb, DirectoryError> {
        Err(DirectoryError::Todo)
    }
    /// The lower corner.
    pub fn min(&self) -> [f64; 2] {
        self.min
    }
    /// The upper corner.
    pub fn max(&self) -> [f64; 2] {
        self.max
    }
    /// Whether the closed boxes share a point.
    pub fn intersects(&self, _other: &Aabb) -> bool {
        false
    }
    /// Whether `other` lies inside this box.
    pub fn contains(&self, _other: &Aabb) -> bool {
        false
    }
}

/// A spatial cell. Zero is not a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(NonZeroU32);

impl CellId {
    /// The cell, or `None` for 0.
    pub fn new(id: u32) -> Option<CellId> {
        NonZeroU32::new(id).map(CellId)
    }
    /// The id as stored in a block directory entry.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// The blocks of a cell, contiguous in the block directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockRange {
    /// The first block.
    pub first: BlockId,
    /// How many.
    pub count: u32,
}

/// One cell a query yields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cell {
    /// The cell id.
    pub id: CellId,
    /// The bounds that contain every entity in it.
    pub bounds: Aabb,
    /// Entities in it.
    pub entities: u32,
    /// Its hot blocks.
    pub blocks: BlockRange,
}

/// An entity and its source-space bounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityBox {
    /// The id.
    pub id: EntityId,
    /// The bounds.
    pub bounds: Aabb,
}

/// A built directory.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltDirectory {
    /// The block.
    pub bytes: Vec<u8>,
    /// Its node count.
    pub nodes: u32,
    /// The box of the whole tree.
    pub bounds: Aabb,
    /// The cell of each input entity, in input order.
    pub cells: Vec<CellId>,
}

/// Builds the directory over source-space boxes.
pub fn build_directory(_entities: &[EntityBox]) -> Result<BuiltDirectory, DirectoryError> {
    Err(DirectoryError::Todo)
}

/// A validated spatial directory.
#[derive(Debug)]
pub struct SpatialDirectory {
    _private: (),
}

/// The cells a query touches.
#[derive(Debug)]
pub struct CellCursor<'a> {
    _dir: &'a SpatialDirectory,
}

impl Iterator for CellCursor<'_> {
    type Item = Cell;
    fn next(&mut self) -> Option<Cell> {
        None
    }
}

impl SpatialDirectory {
    /// The block of the directory among `entries`, if there is one.
    pub fn find(_entries: &[BlockEntry]) -> Result<Option<BlockId>, DirectoryError> {
        Err(DirectoryError::Todo)
    }
    /// Parses and validates the block.
    pub fn parse(
        _bytes: &[u8],
        _entries: &[BlockEntry],
        _limits: &Limits,
    ) -> Result<SpatialDirectory, DirectoryError> {
        Err(DirectoryError::Todo)
    }
    /// Reads the directory through a container reader.
    pub fn open<S: BlockSource>(
        _reader: &ContainerReader<S>,
    ) -> Result<Option<SpatialDirectory>, DirectoryError> {
        Err(DirectoryError::Todo)
    }
    /// The box of the whole tree.
    pub fn bounds(&self) -> Aabb {
        unimplemented!()
    }
    /// The bounds of the advanced curve set, which the directory cannot split.
    pub fn advanced_bounds(&self) -> Option<Aabb> {
        unimplemented!()
    }
    /// Entities indexed.
    pub fn entities(&self) -> u64 {
        unimplemented!()
    }
    /// Cells.
    pub fn cells(&self) -> usize {
        unimplemented!()
    }
    /// Levels of the tree.
    pub fn height(&self) -> u32 {
        unimplemented!()
    }
    /// The cells whose bounds touch `q`, in the documented order.
    pub fn query(&self, _q: &Aabb) -> CellCursor<'_> {
        unimplemented!()
    }
}
