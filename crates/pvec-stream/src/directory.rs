//! The spatial directory (issue P5.1): which cells of a drawing a viewport touches.
//!
//! ADR 0005 chose a packed R-tree over a Hilbert ordering of entity centres (its model M2): every
//! payload is owned by exactly one leaf, leaves are small, and the tree is built over source-space
//! `f64` boxes and nothing else. There is no display zoom, tile grid or projection anywhere in the
//! input, so one file serves any display scale and the directory block is byte-identical whatever scale
//! the producer declared. The tree is one block of kind "spatial" in the container, always stored as it
//! is, so opening it is one range read of a length the block directory already gives.
//!
//! A leaf is a *cell*: its id is the `cell` field of the hot blocks that hold its entities, so the block
//! directory finds a cell's blocks without a second table. Cell 0 is not a cell (it is "no cell" in the
//! block directory), so leaf ids start at 1. The layout, the invariants and what a reader validates
//! before it traverses are in `docs/STREAMING.md` chapter 1.
//!
//! The directory is untrusted input like the rest of the file (P4.8). [`SpatialDirectory::parse`]
//! checks the whole block against the rules in [`DIRECTORY_RULES`] before anything is walked, and the
//! walk itself is iterative with a stack bounded by `MAX_HEIGHT * MAX_FANOUT`, so a hostile directory
//! costs at most its own length.

use pvec_container::directory::{BlockEntry, BlockKind, STRADDLER_CELL_FIRST};
use pvec_container::reader::ReadError;
use pvec_container::source::BlockSource;
use pvec_container::{BlockId, Bounds, ContainerReader, LimitExceeded, LimitKind, Limits};
use pvec_entity::EntityId;
use pvec_numeric::{MAX_ENTITIES, check_value};
use std::num::NonZeroU32;

/// Entities per cell (the leaf capacity) that a writer cuts the directory at, and the checked-in default
/// of this crate. [`build_directory`] and [`classify`](crate::classify) take the capacity as an argument, because a file
/// records the one it was built with (SPD-02) and the structural fixtures of the tests are small leaves;
/// a writer passes this.
///
/// P1.6 (`docs/spikes/01-6-spatial-index.md`, ADR 0005) measured two settings of the packed R-tree on
/// the `million` fixture, native x64, byte model at a 64 KiB merge gap, interactive steps only:
///
/// | leaf / fanout | index bytes | interactive MB / ranges | `stress-long` interactive MB |
/// |---|---|---|---|
/// | 64 / 32 | 516,256 | 29.08 / 95 | 15.89 |
/// | 512 / 64 | 63,616 | 31.12 / 80 | 20.02 |
///
/// The small leaf won on bytes read, so it was the default (64) until P8.6. P8.5 then measured what a
/// small leaf costs in time on a link: the block directory and the spatial directory of a million entities
/// are 5.7 MB that every viewer reads before it can show anything, and a Tier 0 block per 64 entities is
/// about 26 requests per viewport. `docs/bench/TUNING.md` has the sweep; 32,768 is the smallest power of
/// two at which every progressive threshold the capacity can move is met on both platforms. The price is
/// bytes: a viewport reads whole leaves, and the writer cuts a leaf's blocks of one kind at 16 MiB.
pub const CELL_CAPACITY: u32 = 32768;

/// Children per internal node, from the P1.6 setting (64 / 32). P8.6 did not tune it: no benchmark moved with it.
pub const FANOUT: u32 = 32;

/// The most children a node of a file may declare, whatever [`FANOUT`] is, so a hostile header cannot
/// ask for a wide walk.
pub const MAX_FANOUT: u32 = 256;

/// The tallest tree a file may declare. At [`FANOUT`] 32 and a leaf of 64 the entity ceiling of NP-18
/// (2^28) needs 5 levels, so 8 leaves room for the narrowest fanouts and leaves a writer may choose.
pub const MAX_HEIGHT: u32 = 8;

/// Bytes of the block header.
pub const HEADER_LEN: usize = 64;

/// Bytes of one node.
pub const NODE_LEN: usize = 48;

const MAGIC: [u8; 4] = *b"PVS1";
const VERSION: u16 = 1;

/// How entities are cut into cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitPolicy {
    /// Sort by the Hilbert key of the bounds centre (ties by entity id), then cut every
    /// the leaf capacity entities; internal levels cut every [`FANOUT`] nodes.
    HilbertPack,
}

/// The split policy in use.
pub const SPLIT_POLICY: SplitPolicy = SplitPolicy::HilbertPack;

/// The refusals, rule by rule (`docs/STREAMING.md` section 1.3).
pub const DIRECTORY_RULES: &[(&str, &str)] = &[
    (
        "SPD-01",
        "the block is shorter than its header, or its length is not the header plus 48 bytes per declared node",
    ),
    (
        "SPD-02",
        "a magic, version, flags or reserved field that is not defined, a fanout of 0 or above 256, a leaf capacity of 0, or a height of 0 or above 8",
    ),
    ("SPD-03", "more entities declared than the artifact ceiling of the reader's limits (P4.8)"),
    ("SPD-04", "node bounds that are not finite, in range and ordered"),
    (
        "SPD-05",
        "a child pointer outside the nodes, or one that does not point strictly before its parent (a cycle)",
    ),
    (
        "SPD-06",
        "a node that breaks the shape: a leaf with children, an internal node with none or more than the declared fanout, children that are not one level down or do not tile the level below, a root that is not alone on the top level, or a leaf count that differs from the header",
    ),
    ("SPD-07", "an internal node whose bounds do not contain the bounds of a child"),
    (
        "SPD-08",
        "an entity count that is wrong: a leaf with none, an internal node that is not the sum of its children, or a root that is not the header's total",
    ),
    (
        "SPD-09",
        "a leaf cell id that is 0, is in the straddler payload cells or does not ascend strictly from one leaf to the next",
    ),
    (
        "SPD-10",
        "the cells and the hot blocks disagree: a hot block in cell 0 or in no leaf (the straddler payload cells are left alone), a hot block without bounds or with bounds outside its leaf, a leaf with no block, blocks of one cell that are not contiguous, or a leaf whose entity count is not its blocks' total",
    ),
    ("SPD-11", "more than one spatial directory block, or a block entry whose count is not the node count"),
    (
        "SPD-12",
        "a directory cannot be built from the input: no entity, a duplicate entity id, or more entities than NP-18 allows",
    ),
    ("SPD-13", "the container refused the read (the error carries its own rule)"),
    ("SPD-14", "a box that is not finite, in range and ordered"),
];

/// Why a directory was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DirectoryError {
    /// The block's length (SPD-01).
    Length {
        /// Its length.
        length: u64,
        /// The length its header implies, or the header length when it is shorter.
        expected: u64,
    },
    /// A header field (SPD-02).
    Header {
        /// What is wrong.
        reason: &'static str,
    },
    /// A P4.8 ceiling (SPD-03).
    Limit(LimitExceeded),
    /// Bounds of a node (SPD-04).
    NodeBounds {
        /// The node.
        node: u32,
    },
    /// A child pointer (SPD-05).
    ChildPointer {
        /// The node.
        node: u32,
        /// Its first child.
        first: u32,
        /// Its child count.
        count: u32,
    },
    /// The shape of the tree (SPD-06).
    Shape {
        /// The node.
        node: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// A child outside its parent (SPD-07).
    Containment {
        /// The parent.
        node: u32,
    },
    /// An entity count (SPD-08).
    Entities {
        /// The node.
        node: u32,
        /// What the node declares.
        declared: u64,
        /// What it should be.
        expected: u64,
    },
    /// A leaf cell id (SPD-09).
    CellId {
        /// The node.
        node: u32,
        /// The id.
        cell: u32,
    },
    /// The cells against the blocks (SPD-10).
    Blocks {
        /// The cell, or 0 for a block that has none.
        cell: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// More than one directory, or a count that is not the node count (SPD-11).
    Duplicate {
        /// How many blocks, or the entry's count.
        found: u32,
    },
    /// The input of a build (SPD-12).
    Build {
        /// What is wrong.
        reason: &'static str,
    },
    /// The container refused (SPD-13).
    Read(ReadError),
    /// A box (SPD-14).
    BadBox,
}

impl DirectoryError {
    /// The rule identifier of `docs/STREAMING.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            DirectoryError::Length { .. } => "SPD-01",
            DirectoryError::Header { .. } => "SPD-02",
            DirectoryError::Limit(_) => "SPD-03",
            DirectoryError::NodeBounds { .. } => "SPD-04",
            DirectoryError::ChildPointer { .. } => "SPD-05",
            DirectoryError::Shape { .. } => "SPD-06",
            DirectoryError::Containment { .. } => "SPD-07",
            DirectoryError::Entities { .. } => "SPD-08",
            DirectoryError::CellId { .. } => "SPD-09",
            DirectoryError::Blocks { .. } => "SPD-10",
            DirectoryError::Duplicate { .. } => "SPD-11",
            DirectoryError::Build { .. } => "SPD-12",
            DirectoryError::Read(_) => "SPD-13",
            DirectoryError::BadBox => "SPD-14",
        }
    }
}

impl core::fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            DirectoryError::Length { length, expected } => {
                write!(f, "{rule}: the block is {length} bytes and its header implies {expected}")
            }
            DirectoryError::Header { reason } => write!(f, "{rule}: {reason}"),
            DirectoryError::Limit(l) => write!(f, "{rule}: {l}"),
            DirectoryError::NodeBounds { node } => write!(f, "{rule}: node {node} has invalid bounds"),
            DirectoryError::ChildPointer { node, first, count } => {
                write!(f, "{rule}: node {node} points at {count} children from {first}")
            }
            DirectoryError::Shape { node, reason } => write!(f, "{rule}: node {node}: {reason}"),
            DirectoryError::Containment { node } => {
                write!(f, "{rule}: node {node} does not contain a child")
            }
            DirectoryError::Entities { node, declared, expected } => {
                write!(f, "{rule}: node {node} declares {declared} entities, expected {expected}")
            }
            DirectoryError::CellId { node, cell } => {
                write!(f, "{rule}: leaf {node} has cell id {cell}, which is 0 or does not ascend")
            }
            DirectoryError::Blocks { cell, reason } => write!(f, "{rule}: cell {cell}: {reason}"),
            DirectoryError::Duplicate { found } => write!(f, "{rule}: {found}"),
            DirectoryError::Build { reason } => write!(f, "{rule}: {reason}"),
            DirectoryError::Read(e) => write!(f, "{rule}: {e}"),
            DirectoryError::BadBox => write!(f, "{rule}: a box is not finite, in range and ordered"),
        }
    }
}

impl std::error::Error for DirectoryError {}

impl From<ReadError> for DirectoryError {
    fn from(e: ReadError) -> DirectoryError {
        DirectoryError::Read(e)
    }
}

/// An axis-aligned box in source space: finite, inside the numeric policy's range, `min <= max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    min: [f64; 2],
    max: [f64; 2],
}

impl Aabb {
    /// A box from its corners. Refused when a value breaks NP-01 to NP-03 or a `min` is above its `max`.
    pub fn new(min: [f64; 2], max: [f64; 2]) -> Result<Aabb, DirectoryError> {
        let mut v = [0.0f64; 4];
        for (slot, x) in v.iter_mut().zip([min[0], min[1], max[0], max[1]]) {
            *slot = check_value(x).map_err(|_| DirectoryError::BadBox)?;
        }
        if v[0] > v[2] || v[1] > v[3] {
            return Err(DirectoryError::BadBox);
        }
        Ok(Aabb { min: [v[0], v[1]], max: [v[2], v[3]] })
    }

    /// The lower corner.
    pub fn min(&self) -> [f64; 2] {
        self.min
    }

    /// The upper corner.
    pub fn max(&self) -> [f64; 2] {
        self.max
    }

    /// Whether the closed boxes share a point: touching counts.
    pub fn intersects(&self, other: &Aabb) -> bool {
        self.min[0] <= other.max[0]
            && other.min[0] <= self.max[0]
            && self.min[1] <= other.max[1]
            && other.min[1] <= self.max[1]
    }

    /// Whether `other` lies inside this box (its edges may touch).
    pub fn contains(&self, other: &Aabb) -> bool {
        self.min[0] <= other.min[0]
            && self.min[1] <= other.min[1]
            && other.max[0] <= self.max[0]
            && other.max[1] <= self.max[1]
    }

    pub(crate) fn union(&self, other: &Aabb) -> Aabb {
        Aabb {
            min: [self.min[0].min(other.min[0]), self.min[1].min(other.min[1])],
            max: [self.max[0].max(other.max[0]), self.max[1].max(other.max[1])],
        }
    }

    fn from_container(b: Bounds) -> Option<Aabb> {
        Aabb::new(b.min, b.max).ok()
    }
}

impl From<Aabb> for Bounds {
    fn from(b: Aabb) -> Bounds {
        Bounds { min: b.min, max: b.max }
    }
}

/// A spatial cell. Zero is "no cell" in the block directory and is never a cell of the directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellId(NonZeroU32);

impl CellId {
    /// The cell, or `None` for 0.
    pub fn new(id: u32) -> Option<CellId> {
        NonZeroU32::new(id).map(CellId)
    }

    /// The id as the `cell` field of a block directory entry stores it.
    pub fn get(self) -> u32 {
        self.0.get()
    }
}

/// The hot blocks of one cell. They are adjacent in the block directory, which orders blocks by kind and
/// then cell, so a range names them.
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
    /// Bounds that contain every entity in the cell.
    pub bounds: Aabb,
    /// Entities in the cell.
    pub entities: u32,
    /// The hot blocks that hold them.
    pub blocks: BlockRange,
}

/// An entity and its source-space bounds: all the directory is built from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityBox {
    /// The entity.
    pub id: EntityId,
    /// Its bounds.
    pub bounds: Aabb,
}

/// A directory built over some entities.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltDirectory {
    /// The block, as the container stores it.
    pub bytes: Vec<u8>,
    /// Its node count, which is the block directory entry's count.
    pub nodes: u32,
    /// The box of the whole tree, which is the block directory entry's bounds.
    pub bounds: Aabb,
    /// The cell of each input entity, in the order the input was given. Hand it to the writer as the
    /// entity's `cell`.
    pub cells: Vec<CellId>,
}

/// The Hilbert distance of a cell of a 65,536 x 65,536 grid (the classic rotate-and-flip walk, integer
/// arithmetic only).
pub(crate) fn hilbert(mut x: u32, mut y: u32) -> u32 {
    let n: u32 = 1 << 16;
    let mut d: u64 = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = u32::from(x & s != 0);
        let ry = u32::from(y & s != 0);
        d += u64::from(s) * u64::from(s) * u64::from((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            core::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d as u32
}

/// Builds the directory over source-space boxes.
///
/// The result depends on the set of boxes and nothing else: not on the order they arrive in, not on a
/// display scale (there is no parameter for one), not on the machine (comparisons, one division and a
/// floor, no transcendental function). Entities are ordered by the Hilbert key of their bounds centre
/// and then by entity id, cut into cells of `cell_capacity` ([`CELL_CAPACITY`] unless a fixture wants small
/// leaves), and the cells into nodes of [`FANOUT`] children, level by level. Cell ids run from 1 in that
/// order. The capacity is written in the header, and a capacity of 0 is refused.
pub fn build_directory(entities: &[EntityBox], cell_capacity: u32) -> Result<BuiltDirectory, DirectoryError> {
    let build = |reason| DirectoryError::Build { reason };
    if cell_capacity == 0 {
        return Err(build("a leaf holds no entity"));
    }
    let first = entities.first().ok_or(build("there is no entity"))?;
    if entities.len() > MAX_ENTITIES {
        return Err(build("more entities than NP-18 allows"));
    }
    let mut ids: Vec<u32> = entities.iter().map(|e| e.id.index()).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(build("an entity id is used twice"));
    }
    let world = entities.iter().fold(first.bounds, |w, e| w.union(&e.bounds));
    let (wx, wy) = (world.max[0] - world.min[0], world.max[1] - world.min[1]);
    let grid = |v: f64, lo: f64, w: f64| -> u32 {
        if w > 0.0 { (((v - lo) / w * 65535.0).floor().clamp(0.0, 65535.0)) as u32 } else { 0 }
    };
    let mut order: Vec<(u32, u32, usize)> = entities
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let cx = e.bounds.min[0] + (e.bounds.max[0] - e.bounds.min[0]) / 2.0;
            let cy = e.bounds.min[1] + (e.bounds.max[1] - e.bounds.min[1]) / 2.0;
            (hilbert(grid(cx, world.min[0], wx), grid(cy, world.min[1], wy)), e.id.index(), i)
        })
        .collect();
    order.sort_unstable();

    let mut cells = vec![CellId::new(1).ok_or(build("no cell"))?; entities.len()];
    let mut nodes: Vec<Node> = Vec::new();
    for (leaf, chunk) in order.chunks(cell_capacity as usize).enumerate() {
        let cell = CellId::new(leaf as u32 + 1).ok_or(build("no cell"))?;
        let mut bounds = entities[chunk[0].2].bounds;
        for (_, _, i) in chunk {
            bounds = bounds.union(&entities[*i].bounds);
            cells[*i] = cell;
        }
        nodes.push(Node { bounds, first: cell.get(), count: 0, entities: chunk.len() as u32, level: 0 });
    }
    let leaf_count = nodes.len();
    let mut level_start = 0usize;
    let mut level = 0u16;
    while nodes.len() - level_start > 1 {
        let end = nodes.len();
        level += 1;
        let mut at = level_start;
        while at < end {
            let to = (at + FANOUT as usize).min(end);
            let mut bounds = nodes[at].bounds;
            let mut total = 0u32;
            for n in &nodes[at..to] {
                bounds = bounds.union(&n.bounds);
                total += n.entities;
            }
            nodes.push(Node { bounds, first: at as u32, count: (to - at) as u32, entities: total, level });
            at = to;
        }
        level_start = end;
    }
    let root = nodes.last().ok_or(build("no node"))?;
    let bounds = root.bounds;
    let mut bytes = Vec::with_capacity(HEADER_LEN + NODE_LEN * nodes.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    for v in [
        nodes.len() as u32,
        leaf_count as u32,
        u32::from(level) + 1,
        FANOUT,
        cell_capacity,
        entities.len() as u32,
    ] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes.resize(HEADER_LEN, 0);
    for n in &nodes {
        for v in [n.bounds.min[0], n.bounds.min[1], n.bounds.max[0], n.bounds.max[1]] {
            // negative zero is written as zero, like every other bound of the format
            bytes.extend_from_slice(&(if v == 0.0 { 0.0 } else { v }).to_le_bytes());
        }
        for v in [n.first, n.count, n.entities] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&n.level.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
    }
    Ok(BuiltDirectory { bytes, nodes: nodes.len() as u32, bounds, cells })
}

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// A leaf: its cell id. Internal: the index of its first child.
    first: u32,
    /// A leaf: 0. Internal: its child count.
    count: u32,
    entities: u32,
    level: u16,
}

/// A validated spatial directory.
#[derive(Debug)]
pub struct SpatialDirectory {
    nodes: Vec<Node>,
    /// The hot blocks of each leaf, by leaf index (the leaves are the first nodes).
    blocks: Vec<BlockRange>,
    leaves: usize,
    height: u32,
    entities: u64,
    advanced: Option<Aabb>,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    f64::from_le_bytes(a)
}

impl SpatialDirectory {
    /// The block of the directory among `entries`: `None` for a file that has none, an error for one
    /// that has more than one.
    pub fn find(entries: &[BlockEntry]) -> Result<Option<BlockId>, DirectoryError> {
        let mut found = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.block_kind() == Some(BlockKind::SpatialDirectory))
            .map(|(i, _)| BlockId(i as u32));
        let first = found.next();
        match (first, found.count()) {
            (_, n) if n > 0 => Err(DirectoryError::Duplicate { found: n as u32 + 1 }),
            (first, _) => Ok(first),
        }
    }

    /// Parses and validates a directory block against the block directory `entries` it came with.
    ///
    /// Everything is checked before the tree is used (`docs/STREAMING.md` section 1.3): the length is
    /// held to the header before any node is allocated, the counts to `limits`, every pointer to the
    /// nodes and to its parent, every box to its children and every cell to the blocks. A source that
    /// reads the block asynchronously calls this with the bytes it got.
    pub fn parse(
        bytes: &[u8],
        entries: &[BlockEntry],
        limits: &Limits,
    ) -> Result<SpatialDirectory, DirectoryError> {
        let length = bytes.len() as u64;
        if bytes.len() < HEADER_LEN {
            return Err(DirectoryError::Length { length, expected: HEADER_LEN as u64 });
        }
        let node_count = u32_at(bytes, 8);
        let expected = HEADER_LEN as u64 + NODE_LEN as u64 * u64::from(node_count);
        if length != expected {
            return Err(DirectoryError::Length { length, expected });
        }
        let header = |reason| Err(DirectoryError::Header { reason });
        if bytes[..4] != MAGIC {
            return header("bad magic");
        }
        if u16_at(bytes, 4) != VERSION {
            return header("unsupported version");
        }
        if u16_at(bytes, 6) != 0 || bytes[32..HEADER_LEN].iter().any(|b| *b != 0) {
            return header("a flag or reserved field is set");
        }
        let (leaf_count, height, fanout) = (u32_at(bytes, 12), u32_at(bytes, 16), u32_at(bytes, 20));
        if height == 0 || height > MAX_HEIGHT {
            return header("height is 0 or above the ceiling");
        }
        if fanout == 0 || fanout > MAX_FANOUT {
            return header("fanout is 0 or above the ceiling");
        }
        if u32_at(bytes, 24) == 0 {
            return header("leaf capacity is 0");
        }
        if node_count == 0 || leaf_count == 0 || leaf_count > node_count {
            return header("no nodes, or more leaves than nodes");
        }
        let total = u32_at(bytes, 28);
        limits.check(LimitKind::EntitiesPerArtifact, u64::from(total)).map_err(DirectoryError::Limit)?;

        // the nodes: bounds, the shape and the pointers, in one pass over the bytes
        let n = node_count as usize;
        let mut nodes: Vec<Node> = Vec::with_capacity(n);
        for i in 0..n {
            let at = HEADER_LEN + NODE_LEN * i;
            let mut v = [0.0f64; 4];
            for (k, slot) in v.iter_mut().enumerate() {
                *slot = check_value(f64_at(bytes, at + 8 * k))
                    .map_err(|_| DirectoryError::NodeBounds { node: i as u32 })?;
            }
            if v[0] > v[2] || v[1] > v[3] {
                return Err(DirectoryError::NodeBounds { node: i as u32 });
            }
            if u16_at(bytes, at + 46) != 0 {
                return Err(DirectoryError::Shape { node: i as u32, reason: "a reserved field is set" });
            }
            nodes.push(Node {
                bounds: Aabb { min: [v[0], v[1]], max: [v[2], v[3]] },
                first: u32_at(bytes, at + 32),
                count: u32_at(bytes, at + 36),
                entities: u32_at(bytes, at + 40),
                level: u16_at(bytes, at + 44),
            });
        }

        // `next[l]` is the child the next node of level `l` must start at, so children tile the level
        // below exactly and every node has exactly one parent; `start[l]` is where level `l` begins
        let h = height as usize;
        let mut start: Vec<Option<usize>> = vec![None; h];
        let mut next: Vec<usize> = vec![0; h];
        let mut level_len: Vec<usize> = vec![0; h];
        let mut previous_cell = 0u32;
        let mut leaves = 0usize;
        for (i, node) in nodes.iter().enumerate() {
            let me = i as u32;
            let shape = |reason| Err(DirectoryError::Shape { node: me, reason });
            let l = usize::from(node.level);
            if l >= h {
                return shape("level is not below the height");
            }
            if i > 0 && node.level < nodes[i - 1].level {
                return shape("levels do not ascend through the nodes");
            }
            if start[l].is_none() {
                if l > 0 && start[l - 1].is_none() {
                    return shape("a level is empty");
                }
                start[l] = Some(i);
                if l > 0 {
                    next[l] = start[l - 1].unwrap_or(0);
                }
            }
            level_len[l] += 1;
            if l == 0 {
                if node.count != 0 {
                    return shape("a leaf has children");
                }
                if node.first == 0 || node.first >= STRADDLER_CELL_FIRST || node.first <= previous_cell {
                    return Err(DirectoryError::CellId { node: me, cell: node.first });
                }
                previous_cell = node.first;
                if node.entities == 0 {
                    return Err(DirectoryError::Entities { node: me, declared: 0, expected: 1 });
                }
                leaves += 1;
            } else {
                let end = u64::from(node.first) + u64::from(node.count);
                if end > u64::from(me) || node.first >= me {
                    return Err(DirectoryError::ChildPointer {
                        node: me,
                        first: node.first,
                        count: node.count,
                    });
                }
                if node.count == 0 {
                    return shape("an internal node has no children");
                }
                if node.count > fanout {
                    return shape("an internal node has more children than the declared fanout");
                }
                if node.first as usize != next[l] {
                    return shape("children do not tile the level below");
                }
                let (a, b) = (node.first as usize, end as usize);
                if nodes[a].level as usize != l - 1 || nodes[b - 1].level as usize != l - 1 {
                    return shape("a child is not one level down");
                }
                next[l] = b;
            }
        }
        if leaves != leaf_count as usize {
            return Err(DirectoryError::Shape { node: 0, reason: "the leaf count is not the header's" });
        }
        for l in 1..h {
            let Some(s) = start[l] else {
                return Err(DirectoryError::Shape { node: node_count - 1, reason: "a level is empty" });
            };
            let _ = s;
            let below_end = start[l - 1].unwrap_or(0) + level_len[l - 1];
            if next[l] != below_end {
                return Err(DirectoryError::Shape { node: node_count - 1, reason: "a node has no parent" });
            }
        }
        if start[h - 1].is_none() || level_len[h - 1] != 1 || start[h - 1] != Some(n - 1) {
            return Err(DirectoryError::Shape {
                node: node_count - 1,
                reason: "the top level is not one root",
            });
        }

        // containment and entity counts, children before parents (they are stored that way)
        for (i, node) in nodes.iter().enumerate().filter(|(_, n)| n.level > 0) {
            let kids = &nodes[node.first as usize..(node.first + node.count) as usize];
            let mut sum = 0u64;
            for k in kids {
                if !node.bounds.contains(&k.bounds) {
                    return Err(DirectoryError::Containment { node: i as u32 });
                }
                sum += u64::from(k.entities);
            }
            if sum != u64::from(node.entities) {
                return Err(DirectoryError::Entities {
                    node: i as u32,
                    declared: u64::from(node.entities),
                    expected: sum,
                });
            }
        }
        let root = &nodes[n - 1];
        if u64::from(root.entities) != u64::from(total) {
            return Err(DirectoryError::Entities {
                node: node_count - 1,
                declared: u64::from(root.entities),
                expected: u64::from(total),
            });
        }

        let blocks = check_blocks(&nodes[..leaves], entries)?;
        if let Some(id) = SpatialDirectory::find(entries)? {
            let count = entries[id.0 as usize].count;
            if count != node_count {
                return Err(DirectoryError::Duplicate { found: count });
            }
        }
        let advanced = entries
            .iter()
            .filter(|e| e.block_kind() == Some(BlockKind::Records))
            .filter_map(|e| e.bounds.and_then(Aabb::from_container))
            .reduce(|a, b| a.union(&b));
        Ok(SpatialDirectory { nodes, blocks, leaves, height, entities: u64::from(total), advanced })
    }

    /// Reads the directory through a container reader: one range read, of the length the block
    /// directory already gave, on top of what opening the container cost. `None` when the file has no
    /// directory (an index is optional, and without one a reader reads what it needs by other means).
    pub fn open<S: BlockSource>(
        reader: &ContainerReader<S>,
    ) -> Result<Option<SpatialDirectory>, DirectoryError> {
        let Some(id) = SpatialDirectory::find(reader.entries())? else { return Ok(None) };
        let block = reader.read_block(id)?;
        SpatialDirectory::parse(block.as_bytes(), reader.entries(), &reader.config().limits).map(Some)
    }

    /// The box of the whole tree.
    pub fn bounds(&self) -> Aabb {
        self.nodes[self.nodes.len() - 1].bounds
    }

    /// The bounds of the advanced curve set. The container keeps those curves in one set in cell 0 (they
    /// cannot be split by cell), so no leaf holds them: a reader that wants every entity in a viewport
    /// fetches the set when this box touches the query. `None` when the file has no advanced curves.
    pub fn advanced_bounds(&self) -> Option<Aabb> {
        self.advanced
    }

    /// The entities the cells hold (hot entities).
    pub fn entities(&self) -> u64 {
        self.entities
    }

    /// The number of cells.
    pub fn cells(&self) -> usize {
        self.leaves
    }

    /// The levels of the tree: 1 for a single cell.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The cells whose bounds touch `q` (closed boxes: touching counts), in ascending cell id. That is
    /// the order of the leaves, which is the Hilbert order of their entities' centres, so a cell is
    /// never yielded twice and the same query on the same file always gives the same sequence.
    pub fn query(&self, q: &Aabb) -> CellCursor<'_> {
        let root = self.nodes.len() - 1;
        let stack = if self.nodes[root].bounds.intersects(q) { vec![root as u32] } else { Vec::new() };
        CellCursor { dir: self, q: *q, stack }
    }
}

/// Checks the leaves against the hot blocks and returns each leaf's block range.
fn check_blocks(leaves: &[Node], entries: &[BlockEntry]) -> Result<Vec<BlockRange>, DirectoryError> {
    let blocks = |cell, reason| Err(DirectoryError::Blocks { cell, reason });
    let mut ranges: Vec<Option<BlockRange>> = vec![None; leaves.len()];
    let mut seen = vec![0u64; leaves.len()];
    let mut current: Option<usize> = None;
    for (i, e) in entries.iter().enumerate() {
        if e.block_kind() != Some(BlockKind::Hot) {
            continue;
        }
        if e.cell == 0 {
            return blocks(0, "a hot block is in cell 0, which is no cell");
        }
        // the payloads of the straddler lists: no leaf holds them, the lists point at them (P5.3b)
        if e.cell >= STRADDLER_CELL_FIRST {
            continue;
        }
        let Ok(leaf) = leaves.binary_search_by(|n| n.first.cmp(&e.cell)) else {
            return blocks(e.cell, "a hot block is in a cell the directory does not have");
        };
        let Some(b) = e.bounds.and_then(Aabb::from_container) else {
            return blocks(e.cell, "a hot block has no bounds");
        };
        if !leaves[leaf].bounds.contains(&b) {
            return blocks(e.cell, "a hot block's bounds are outside its cell");
        }
        match (&mut ranges[leaf], current) {
            (slot @ None, _) => *slot = Some(BlockRange { first: BlockId(i as u32), count: 1 }),
            (Some(r), Some(c)) if c == leaf => r.count += 1,
            (Some(_), _) => return blocks(e.cell, "the blocks of a cell are not adjacent"),
        }
        current = Some(leaf);
        seen[leaf] += u64::from(e.count);
    }
    let mut out = Vec::with_capacity(leaves.len());
    for ((leaf, range), total) in leaves.iter().zip(ranges).zip(seen) {
        let Some(range) = range else { return blocks(leaf.first, "a cell has no hot block") };
        if total != u64::from(leaf.entities) {
            return blocks(leaf.first, "a cell's entity count is not its blocks' total");
        }
        out.push(range);
    }
    Ok(out)
}

/// The cells a query touches, in ascending cell id. It walks the tree with an explicit stack that never
/// holds more than `MAX_HEIGHT * MAX_FANOUT` entries, whatever the file says.
#[derive(Debug)]
pub struct CellCursor<'a> {
    dir: &'a SpatialDirectory,
    q: Aabb,
    stack: Vec<u32>,
}

impl Iterator for CellCursor<'_> {
    type Item = Cell;

    fn next(&mut self) -> Option<Cell> {
        while let Some(i) = self.stack.pop() {
            let node = &self.dir.nodes[i as usize];
            if node.level == 0 {
                let id = CellId::new(node.first)?;
                return Some(Cell {
                    id,
                    bounds: node.bounds,
                    entities: node.entities,
                    blocks: self.dir.blocks[i as usize],
                });
            }
            // pushed last to first, so the first child is popped first and the leaves come out ascending
            for c in (node.first..node.first + node.count).rev() {
                if self.dir.nodes[c as usize].bounds.intersects(&self.q) {
                    self.stack.push(c);
                }
            }
        }
        None
    }
}
