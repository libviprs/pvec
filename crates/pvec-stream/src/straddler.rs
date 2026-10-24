//! Long entities and straddlers (issue P5.3): the entities too big for any cell of the spatial
//! directory to hold without swelling.
//!
//! One 400-metre grid line or a drawing-wide hatch boundary intersects nearly every cell. Both naive
//! answers turn the first read into the whole file: promoting it to the lowest tree node that contains
//! it puts it in the root, where every viewport reads it, and duplicating its reference into every cell
//! it crosses puts it everywhere. A long entity is also poison inside a cell, because the cell's bounds
//! grow to contain it and the cell is then fetched by every viewport that touches that swollen box.
//!
//! So a long entity is kept out of the cells. It is classified against the cell size of each level of
//! the directory ([`classify`]), and an entity whose longer side exceeds a level's cell size by
//! [`STRADDLER_FACTOR`] is a *straddler at that level*. It is listed once, at the coarsest level where
//! that holds, with its own bounds, so a viewport that does not touch those bounds never reads its
//! payload and a viewport that does reads it once. The directory is then built over the local entities
//! alone and keeps its small, tight cells.
//!
//! ADR 0005 kept "leaf cells with long-entity buckets" (its model M4) as the safety valve beside the
//! packed R-tree. Lists per level are that valve with the bucket split by scale: P1.6 measured one
//! bucket, and one bucket makes a short-but-wide entity cost what a drawing-wide one does. The choice
//! and the numbers are in `docs/STREAMING.md` chapter 3.
//!
//! The lists are their own block of bytes, laid out in section 3.2 of that document and validated by
//! [`StraddlerLists::parse`] under the rules in [`STRADDLER_RULES`] before anything is searched. The
//! block carries references, not geometry: each entry is the entity's id, its bounds and the shape of
//! the reference P5.2 gives every payload (`PayloadLoc`), so the payload is stored exactly once wherever
//! the writer put it.

#![allow(unused)]
use crate::directory::{
    Aabb, BuiltDirectory, CELL_CAPACITY, Cell, EntityBox, FANOUT, HEADER_LEN, MAX_HEIGHT, NODE_LEN,
    SpatialDirectory, hilbert,
};
use pvec_container::{LimitExceeded, LimitKind, Limits};
use pvec_entity::EntityId;
use pvec_numeric::check_value;
use std::collections::BTreeSet;

/// How many times a level's cell size an entity's longer side must exceed to be a straddler there.
///
/// 4 means an entity at least four cells long at that level: it touches four or more cells of the level
/// in a row, so listing it once with its bounds beats a cell's worth of references per level. Below it
/// the entity stays in a cell and only widens that cell by less than four cells. P1.6 did not measure a
/// factor (it measured one oversized bucket, ADR 0005's M4); this constant is the one thing `docs/
/// STREAMING.md` chapter 3 tunes against the fixtures, and tuning it on real drawings is P8.6.
pub const STRADDLER_FACTOR: f64 = 4.0;

/// Bytes of the block header.
pub const STRADDLER_HEADER_LEN: usize = 96;

/// Bytes of one page descriptor.
pub const PAGE_LEN: usize = 40;

/// Entries in a full page. A level's list is cut into pages of this many entries that start near each
/// other, each with its own bounds, so a viewport fetches the pages it touches and not the whole list.
/// A page of 64 entries is 2,816 bytes, one descriptor is 1.4 per cent of it.
pub const STRADDLER_PAGE: usize = 64;

/// Bytes of one entry: four `f64` bounds, the entity id, the block and the slot.
pub const STRADDLER_ENTRY_LEN: usize = 44;

/// The refusals, rule by rule (`docs/STREAMING.md` section 3.3).
pub const STRADDLER_RULES: &[(&str, &str)] = &[
    (
        "STR-01",
        "the block is shorter than its header, or its length is not the header plus 40 bytes per declared page plus 44 bytes per declared entry",
    ),
    (
        "STR-02",
        "a magic, version, flags or reserved field that is not defined, a level count of 0 or above the height ceiling, or a factor that is not finite and above 1",
    ),
    ("STR-03", "more entries declared than the artifact ceiling of the reader's limits (P4.8)"),
    ("STR-04", "an entry or page whose bounds are not finite, in range and ordered"),
    (
        "STR-05",
        "a page that breaks the shape: levels not ascending or not below the level count, no entries or more than a page holds, entries that do not tile the pages exactly, or bounds that are not the union of the page's entries",
    ),
    ("STR-06", "a level cell size that is not finite and above 0, or a size declared beyond the levels"),
    (
        "STR-07",
        "an entry at the wrong level: its longer side must exceed the factor times the cell size of its level and must not exceed it for any coarser level",
    ),
    ("STR-08", "an entity listed twice, in one page, in one level or in two"),
    (
        "STR-09",
        "a block cannot be built from the input: a duplicate entity id, an entity that is no straddler at any level, or more entities than NP-18 allows",
    ),
];

/// Why a straddler block was refused or could not be built.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StraddlerError {
    /// The block's length (STR-01).
    Length {
        /// Its length.
        length: u64,
        /// The length its header implies, or the header length when it is shorter.
        expected: u64,
    },
    /// A header field (STR-02).
    Header {
        /// What is wrong.
        reason: &'static str,
    },
    /// A P4.8 ceiling (STR-03).
    Limit(LimitExceeded),
    /// An entry or list box (STR-04).
    Bounds {
        /// The entry, or the list when `list` is set.
        index: u32,
        /// Whether `index` names a list.
        list: bool,
    },
    /// A list's shape (STR-05).
    List {
        /// The list.
        list: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// A level cell size (STR-06).
    Scale {
        /// The level.
        level: u16,
    },
    /// An entry at the wrong level (STR-07).
    Placement {
        /// The entity.
        id: u32,
        /// The level it is listed at.
        level: u16,
    },
    /// An entity listed twice, or out of order (STR-08).
    Duplicate {
        /// The entity.
        id: u32,
    },
    /// The input of a build (STR-09).
    Build {
        /// What is wrong.
        reason: &'static str,
    },
}

impl StraddlerError {
    /// The rule identifier of `docs/STREAMING.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            StraddlerError::Length { .. } => "STR-01",
            StraddlerError::Header { .. } => "STR-02",
            StraddlerError::Limit(_) => "STR-03",
            StraddlerError::Bounds { .. } => "STR-04",
            StraddlerError::List { .. } => "STR-05",
            StraddlerError::Scale { .. } => "STR-06",
            StraddlerError::Placement { .. } => "STR-07",
            StraddlerError::Duplicate { .. } => "STR-08",
            StraddlerError::Build { .. } => "STR-09",
        }
    }
}

impl core::fmt::Display for StraddlerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            StraddlerError::Length { length, expected } => {
                write!(f, "{rule}: the block is {length} bytes and its header implies {expected}")
            }
            StraddlerError::Header { reason } => write!(f, "{rule}: {reason}"),
            StraddlerError::Limit(l) => write!(f, "{rule}: {l}"),
            StraddlerError::Bounds { index, list: true } => {
                write!(f, "{rule}: list {index} has invalid bounds")
            }
            StraddlerError::Bounds { index, list: false } => {
                write!(f, "{rule}: entry {index} has invalid bounds")
            }
            StraddlerError::List { list, reason } => write!(f, "{rule}: list {list}: {reason}"),
            StraddlerError::Scale { level } => write!(f, "{rule}: level {level} has no usable cell size"),
            StraddlerError::Placement { id, level } => {
                write!(f, "{rule}: entity {id} is listed at level {level}, which is not its level")
            }
            StraddlerError::Duplicate { id } => write!(f, "{rule}: entity {id} is listed twice or out of order"),
            StraddlerError::Build { reason } => write!(f, "{rule}: {reason}"),
        }
    }
}

impl std::error::Error for StraddlerError {}

/// Where a payload is: the shape of P5.2's `PayloadRef { block_id, slot }`, eight bytes, kept as two
/// opaque numbers here because the placement pass that assigns them is P5.2's. A straddler's payload is
/// stored once, wherever the writer put it, and this says where.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PayloadLoc {
    /// The block the payload is in.
    pub block: u32,
    /// The slot inside it.
    pub slot: u32,
}

const _: () = assert!(core::mem::size_of::<PayloadLoc>() == 8);

/// A long entity to list: its id, its source-space bounds and where its payload is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongEntity {
    /// The entity.
    pub id: EntityId,
    /// Its bounds.
    pub bounds: Aabb,
    /// Where its payload is.
    pub payload: PayloadLoc,
}

/// The longer side of a box, which is what an entity's reach is measured by.
pub fn extent(b: &Aabb) -> f64 {
    (b.max()[0] - b.min()[0]).max(b.max()[1] - b.min()[1])
}

fn union(a: &Aabb, b: &Aabb) -> Aabb {
    Aabb::new(
        [a.min()[0].min(b.min()[0]), a.min()[1].min(b.min()[1])],
        [a.max()[0].max(b.max()[0]), a.max()[1].max(b.max()[1])],
    )
    .expect("the union of two valid boxes is valid")
}

/// The cell size of each level of the directory and the factor that makes an entity a straddler there.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelScale {
    factor: f64,
    sizes: Vec<f64>,
}

impl LevelScale {
    /// A scale from a factor above 1 and one cell size per level, finest first: finite and above 0.
    pub fn new(factor: f64, sizes: Vec<f64>) -> Result<LevelScale, StraddlerError> { unimplemented!("P5.3: not written yet") }

    /// The factor.
    pub fn factor(&self) -> f64 {
        self.factor
    }

    /// The cell size of each level, finest first.
    pub fn sizes(&self) -> &[f64] {
        &self.sizes
    }

    /// The level an entity of this longer side is listed at: the coarsest level whose cell size it
    /// exceeds by the factor. `None` when it exceeds none, which is a local entity.
    pub fn level_of(&self, extent: f64) -> Option<u16> { unimplemented!("P5.3: not written yet") }
}

/// The entities of an artifact split into those a cell holds and those a list does.
#[derive(Clone, Debug, PartialEq)]
pub struct Classification {
    /// The entities that stay in cells, in input order. Build the directory over these.
    pub local: Vec<EntityBox>,
    /// The straddlers, in input order.
    pub long: Vec<EntityBox>,
    /// The scale they were classified against.
    pub scale: LevelScale,
}

/// The height of the directory `n` entities make at the default capacity and fanout.
fn height_for(n: usize) -> usize {
    let mut nodes = n.div_ceil(CELL_CAPACITY as usize);
    let mut h = 1;
    while nodes > 1 {
        nodes = nodes.div_ceil(FANOUT as usize);
        h += 1;
    }
    h
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    f64::from_le_bytes(a)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// The nodes of a directory this crate just built: bounds, first child, child count and level. The
/// bytes are the builder's own, so they are not validated again.
fn built_nodes(built: &BuiltDirectory) -> Vec<(Aabb, u32, u32, u16)> {
    (0..built.nodes as usize)
        .map(|i| {
            let at = HEADER_LEN + NODE_LEN * i;
            let b = &built.bytes;
            let r = |k: usize| f64_at(b, at + 8 * k);
            (
                Aabb::new([r(0), r(1)], [r(2), r(3)]).expect("the builder wrote valid bounds"),
                u32_at(b, at + 32),
                u32_at(b, at + 36),
                u16_at(b, at + 44),
            )
        })
        .collect()
}

/// The finest cell size of a directory over `entities`: the lower quartile of the longer sides of its
/// leaves.
fn finest_cell(entities: &[EntityBox]) -> Result<f64, StraddlerError> {
    let built = crate::directory::build_directory(entities)
        .map_err(|_| StraddlerError::Build { reason: "a directory cannot be built over the entities" })?;
    let mut leaves: Vec<f64> =
        built_nodes(&built).into_iter().filter(|n| n.3 == 0).map(|n| extent(&n.0)).collect();
    leaves.sort_by(f64::total_cmp);
    Ok(leaves[leaves.len() / 4])
}

/// Splits `entities` into local entities and straddlers.
///
/// The cell size of the finest level is estimated from the shorter half of the entities (by longer
/// side, ties by id): the lower quartile of the longer sides of the leaves of a directory built over
/// them. Long entities are what swell leaves, and as long as fewer than half of the entities are long
/// none of them is in that half, so the estimate is the size of a cell of ordinary entities however
/// many straddlers there are and wherever they lie. Half the entities make cells larger by at most the
/// square root of two than all of the local ones would, which errs towards keeping a borderline entity
/// local, and [`STRADDLER_FACTOR`] has room for that. A drawing in which most entities are long has no
/// ordinary cell to measure: the estimate is then the size of a cell of long entities and little or
/// nothing is called long, which is correct, only not helpful.
///
/// Each level above the leaves grows the cell by the square root of [`FANOUT`] (a node covers [`FANOUT`]
/// times the area of the one below), capped at the longer side of the local drawing. The tests hold the
/// estimate to the cells a directory over the local entities really builds.
pub fn classify(entities: &[EntityBox], factor: f64) -> Result<Classification, StraddlerError> { unimplemented!("P5.3: not written yet") }

/// A straddler block as built.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltStraddlers {
    /// The block bytes.
    pub bytes: Vec<u8>,
    /// Entries in it.
    pub entries: u32,
    /// Pages in it.
    pub pages: u32,
    /// Entries per level, finest first, one slot per level of the scale.
    pub per_level: Vec<u32>,
}

const MAGIC: [u8; 4] = *b"PVST";
const VERSION: u16 = 1;

fn put_f64(out: &mut Vec<u8>, v: f64) {
    out.extend_from_slice(&(if v == 0.0 { 0.0 } else { v }).to_le_bytes());
}

fn put_box(out: &mut Vec<u8>, b: &Aabb) {
    for v in [b.min()[0], b.min()[1], b.max()[0], b.max()[1]] {
        put_f64(out, v);
    }
}

/// Builds the straddler block for `long` against `scale`.
///
/// Each entity goes at the coarsest level whose cell size it exceeds by the factor. Inside a level the
/// entries are ordered by the Hilbert key of their lower-left corner (ties by entity id) and cut into
/// pages of [`STRADDLER_PAGE`], so a page holds entities that start near each other and its bounds
/// stay tight: a viewport reads the pages its box touches and no others. The result depends on the set
/// of entities and not on the order they arrive in. An entity that is no straddler at any level is
/// refused: it belongs in a cell.
pub fn build_straddlers(scale: &LevelScale, long: &[LongEntity]) -> Result<BuiltStraddlers, StraddlerError> { unimplemented!("P5.3: not written yet") }

/// One listed straddler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StraddlerRef {
    /// The entity.
    pub id: EntityId,
    /// Its bounds.
    pub bounds: Aabb,
    /// Where its payload is.
    pub payload: PayloadLoc,
    /// The level it is listed at.
    pub level: u16,
}

#[derive(Clone, Debug)]
struct Page {
    level: u16,
    bounds: Aabb,
    first: usize,
    count: usize,
}

/// The per-level straddler lists of an artifact, validated.
#[derive(Clone, Debug)]
pub struct StraddlerLists {
    scale: LevelScale,
    pages: Vec<Page>,
    entries: Vec<StraddlerRef>,
}

impl StraddlerLists {
    /// Parses and validates a block against [`STRADDLER_RULES`] before anything is searched. The length
    /// is held to the header before any page or entry is allocated, so a header that claims four
    /// billion entries costs nothing.
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<StraddlerLists, StraddlerError> { unimplemented!("P5.3: not written yet") }

    /// The scale the entries were classified against.
    pub fn scale(&self) -> &LevelScale {
        &self.scale
    }

    /// Entries in all lists.
    pub fn len(&self) -> usize { unimplemented!("P5.3: not written yet") }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool { unimplemented!("P5.3: not written yet") }

    /// Pages in all lists.
    pub fn pages(&self) -> usize { unimplemented!("P5.3: not written yet") }

    /// Entries at a level.
    pub fn level_len(&self, level: u16) -> usize { unimplemented!("P5.3: not written yet") }

    /// Every entry, finest level first, in page order.
    pub fn entries(&self) -> &[StraddlerRef] { unimplemented!("P5.3: not written yet") }

    /// The straddlers whose bounds touch `q` (closed boxes), finest level first, in page order. A page
    /// whose bounds miss `q` is skipped whole. Each entity is yielded at most once because the block
    /// lists it once (STR-08).
    pub fn query<'a>(&'a self, q: &'a Aabb) -> impl Iterator<Item = StraddlerRef> + 'a {
        if true {
            unimplemented!("P5.3: not written yet")
        }
        std::iter::empty()
    }

    /// The bytes of the block a reader fetches for `q`: the header and every page descriptor, which it
    /// needs to choose pages, and the entries of the pages whose bounds touch `q`.
    pub fn read_bytes(&self, q: &Aabb) -> u64 { unimplemented!("P5.3: not written yet") }
}

/// What one viewport touches: the cells of the directory and the straddlers of the lists.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewportHits {
    /// The cells whose bounds touch the viewport, ascending cell id (`docs/STREAMING.md` 1.4).
    pub cells: Vec<Cell>,
    /// The straddlers whose bounds touch it.
    pub long: Vec<StraddlerRef>,
}

/// The union of the cells the directory yields for `q` and the straddlers of every level whose bounds
/// touch `q`.
///
/// A list is searched whenever `q` touches its bounds, not only when `q` touches a node of that level,
/// because the directory is built over the local entities alone: a straddler crossing a gap in the
/// drawing is reachable only through its list, and a query that matched no cell must still find it.
pub fn query_viewport(dir: &SpatialDirectory, long: Option<&StraddlerLists>, q: &Aabb) -> ViewportHits { unimplemented!("P5.3: not written yet") }

/// The entities of a viewport, each once. `cell_entities` are the entity ids the caller decoded from
/// the hit cells' blocks; an entity that is also in a straddler list (a writer that referenced it from
/// both) is yielded where it is first met, which is the cell, and not again. The order is the order of
/// `cell_entities`, then the straddlers by level and id.
pub fn unique_entities(
    cell_entities: impl IntoIterator<Item = EntityId>,
    hits: &ViewportHits,
) -> Vec<EntityId> { unimplemented!("P5.3: not written yet") }

/// One level of the congestion report.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelCongestion {
    /// The level, 0 for the leaves.
    pub level: u16,
    /// The cell size the classifier used at this level.
    pub cell_size: f64,
    /// Entities placed at the level: all of the local entities at level 0 (every payload is owned by
    /// one cell) beside the straddlers listed there, and at a higher level the straddlers alone.
    pub entities: u64,
    /// Straddlers listed at the level.
    pub straddlers: u64,
    /// Pages of the level's list.
    pub pages: u64,
    /// Bytes of the level's pages and descriptors, 0 when it has none.
    pub list_bytes: u64,
}

/// The congestion metrics of one artifact (`docs/STREAMING.md` 3.5).
#[derive(Clone, Debug, PartialEq)]
pub struct CongestionReport {
    /// Entities in the cells.
    pub local_entities: u64,
    /// Entities in the lists.
    pub straddlers: u64,
    /// One entry per level of the scale, finest first.
    pub levels: Vec<LevelCongestion>,
    /// References the root level carries: the straddlers listed at the top level of the directory.
    pub root_references: u64,
    /// Bytes of the whole straddler block.
    pub block_bytes: u64,
    /// Bytes of the block every viewport reads: the header and every page descriptor.
    pub index_bytes: u64,
    /// Bytes a minimal viewport read must fetch from the coarsest level: the directory's root node, the
    /// block's index and one page of the coarsest level that has a list (a viewport that touches a
    /// single straddler there).
    pub coarsest_bytes: u64,
}

/// The congestion metrics of a directory and its lists.
pub fn congestion(dir: &SpatialDirectory, long: Option<&StraddlerLists>) -> CongestionReport { unimplemented!("P5.3: not written yet") }

/// The baseline the straddler lists are measured against: promote each entity to the lowest node of
/// `built` whose bounds contain it (the root when none lower does). Returns how many of `entities` land
/// at each level, finest first. Nothing writes this placement; it exists so the report can say what the
/// naive answer would have put in the root.
pub fn ancestor_placement(built: &BuiltDirectory, entities: &[EntityBox]) -> Vec<u64> { unimplemented!("P5.3: not written yet") }
