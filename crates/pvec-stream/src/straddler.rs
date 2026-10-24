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
//! block carries references, not geometry: each entry is the entity's id, its bounds and the
//! reference P5.2 gives every payload (`PayloadRef`), so the payload is stored exactly once wherever
//! the writer put it.

use crate::directory::{
    Aabb, BuiltDirectory, CELL_CAPACITY, Cell, EntityBox, FANOUT, HEADER_LEN, MAX_HEIGHT, NODE_LEN,
    SpatialDirectory, hilbert,
};
use crate::ownership::{PayloadRef, PlacedEntity};
use pvec_container::{BlockId, LimitExceeded, LimitKind, Limits};
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
    (
        "STR-08",
        "an entity id at or above the NP-18 ceiling, or an entity listed twice, in one page, in one level or in two",
    ),
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
            StraddlerError::Duplicate { id } => {
                write!(f, "{rule}: entity {id} is listed twice or out of order")
            }
            StraddlerError::Build { reason } => write!(f, "{rule}: {reason}"),
        }
    }
}

impl std::error::Error for StraddlerError {}

/// A long entity to list: its id, its source-space bounds and where its payload is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongEntity {
    /// The entity.
    pub id: EntityId,
    /// Its bounds.
    pub bounds: Aabb,
    /// Where its payload is.
    pub payload: PayloadRef,
}

impl From<PlacedEntity> for LongEntity {
    /// An entity as P5.2's placement pass sees it, listed as a straddler.
    fn from(e: PlacedEntity) -> LongEntity {
        LongEntity { id: e.id, bounds: e.bounds, payload: e.payload }
    }
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
    pub fn new(factor: f64, sizes: Vec<f64>) -> Result<LevelScale, StraddlerError> {
        if !(factor.is_finite() && factor > 1.0) {
            return Err(StraddlerError::Header { reason: "the factor is not finite and above 1" });
        }
        if sizes.is_empty() || sizes.len() > MAX_HEIGHT as usize {
            return Err(StraddlerError::Header { reason: "a level count of 0 or above the height ceiling" });
        }
        for (l, s) in sizes.iter().enumerate() {
            if !(s.is_finite() && *s > 0.0) {
                return Err(StraddlerError::Scale { level: l as u16 });
            }
        }
        Ok(LevelScale { factor, sizes })
    }

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
    pub fn level_of(&self, extent: f64) -> Option<u16> {
        self.sizes.iter().rposition(|s| extent > self.factor * *s).map(|l| l as u16)
    }
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
pub fn classify(entities: &[EntityBox], factor: f64) -> Result<Classification, StraddlerError> {
    let build = |reason| StraddlerError::Build { reason };
    if entities.is_empty() {
        return Err(build("there is no entity"));
    }
    let mut order: Vec<usize> = (0..entities.len()).collect();
    order.sort_by(|a, b| {
        extent(&entities[*a].bounds)
            .total_cmp(&extent(&entities[*b].bounds))
            .then(entities[*a].id.cmp(&entities[*b].id))
    });
    let shorter: Vec<EntityBox> = order[..entities.len().div_ceil(2)].iter().map(|i| entities[*i]).collect();
    let s0 = finest_cell(&shorter)?;
    if !(s0.is_finite() && s0 > 0.0) {
        // entities that cell sizes cannot measure (points): nothing is long
        let scale = LevelScale::new(factor, vec![f64::MAX])?;
        return Ok(Classification { local: entities.to_vec(), long: Vec::new(), scale });
    }
    let (local, long): (Vec<EntityBox>, Vec<EntityBox>) =
        entities.iter().partition(|e| extent(&e.bounds) <= factor * s0);
    if local.is_empty() {
        return Err(build("every entity is long"));
    }
    let world = local.iter().skip(1).fold(local[0].bounds, |w, e| union(&w, &e.bounds));
    let cap = extent(&world).max(s0);
    let grow = f64::from(FANOUT).sqrt();
    let sizes: Vec<f64> = (0..height_for(local.len())).map(|l| (s0 * grow.powi(l as i32)).min(cap)).collect();
    let scale = LevelScale::new(factor, sizes)?;
    Ok(Classification { local, long, scale })
}

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
pub fn build_straddlers(scale: &LevelScale, long: &[LongEntity]) -> Result<BuiltStraddlers, StraddlerError> {
    let build = |reason| StraddlerError::Build { reason };
    if long.len() > pvec_numeric::MAX_ENTITIES {
        return Err(build("more entities than NP-18 allows"));
    }
    let mut ids: Vec<u32> = long.iter().map(|e| e.id.index()).collect();
    ids.sort_unstable();
    if ids.windows(2).any(|w| w[0] == w[1]) {
        return Err(build("an entity id is used twice"));
    }
    let levels = scale.sizes.len();
    let mut by_level: Vec<Vec<&LongEntity>> = vec![Vec::new(); levels];
    for e in long {
        let level =
            scale.level_of(extent(&e.bounds)).ok_or(build("an entity is no straddler at any level"))?;
        by_level[usize::from(level)].push(e);
    }
    let mut per_level = vec![0u32; levels];
    let mut pages: Vec<(u16, Vec<&LongEntity>)> = Vec::new();
    for (level, group) in by_level.iter_mut().enumerate() {
        per_level[level] = group.len() as u32;
        if group.is_empty() {
            continue;
        }
        let world = group.iter().skip(1).fold(group[0].bounds, |w, e| union(&w, &e.bounds));
        let (wx, wy) = (world.max()[0] - world.min()[0], world.max()[1] - world.min()[1]);
        let grid = |v: f64, lo: f64, w: f64| -> u32 {
            if w > 0.0 { (((v - lo) / w * 65535.0).floor().clamp(0.0, 65535.0)) as u32 } else { 0 }
        };
        group.sort_by_key(|e| {
            let key = hilbert(
                grid(e.bounds.min()[0], world.min()[0], wx),
                grid(e.bounds.min()[1], world.min()[1], wy),
            );
            (key, e.id.index())
        });
        for chunk in group.chunks(STRADDLER_PAGE) {
            pages.push((level as u16, chunk.to_vec()));
        }
    }
    let total: usize = pages.iter().map(|p| p.1.len()).sum();
    let mut bytes =
        Vec::with_capacity(STRADDLER_HEADER_LEN + PAGE_LEN * pages.len() + STRADDLER_ENTRY_LEN * total);
    bytes.extend_from_slice(&MAGIC);
    bytes.extend_from_slice(&VERSION.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    for v in [levels as u32, pages.len() as u32, total as u32, 0] {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    put_f64(&mut bytes, scale.factor);
    for l in 0..MAX_HEIGHT as usize {
        put_f64(&mut bytes, scale.sizes.get(l).copied().unwrap_or(0.0));
    }
    debug_assert_eq!(bytes.len(), STRADDLER_HEADER_LEN);
    for (level, page) in &pages {
        let bounds = page.iter().skip(1).fold(page[0].bounds, |w, e| union(&w, &e.bounds));
        bytes.extend_from_slice(&level.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(page.len() as u32).to_le_bytes());
        put_box(&mut bytes, &bounds);
    }
    for (_, page) in &pages {
        for e in page {
            put_box(&mut bytes, &e.bounds);
            for v in [e.id.index(), e.payload.block_id.0, e.payload.slot] {
                bytes.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    Ok(BuiltStraddlers { bytes, entries: total as u32, pages: pages.len() as u32, per_level })
}

/// One listed straddler.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StraddlerRef {
    /// The entity.
    pub id: EntityId,
    /// Its bounds.
    pub bounds: Aabb,
    /// Where its payload is.
    pub payload: PayloadRef,
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
    pub fn parse(bytes: &[u8], limits: &Limits) -> Result<StraddlerLists, StraddlerError> {
        let length = bytes.len() as u64;
        if bytes.len() < STRADDLER_HEADER_LEN {
            return Err(StraddlerError::Length { length, expected: STRADDLER_HEADER_LEN as u64 });
        }
        let (levels, pages, entries) = (u32_at(bytes, 8), u32_at(bytes, 12), u32_at(bytes, 16));
        let expected = STRADDLER_HEADER_LEN as u64
            + PAGE_LEN as u64 * u64::from(pages)
            + STRADDLER_ENTRY_LEN as u64 * u64::from(entries);
        if length != expected {
            return Err(StraddlerError::Length { length, expected });
        }
        let header = |reason| Err(StraddlerError::Header { reason });
        if bytes[..4] != MAGIC {
            return header("bad magic");
        }
        if u16_at(bytes, 4) != VERSION {
            return header("unsupported version");
        }
        if u16_at(bytes, 6) != 0 || u32_at(bytes, 20) != 0 {
            return header("a flag or reserved field is set");
        }
        if levels == 0 || levels > MAX_HEIGHT {
            return header("a level count of 0 or above the height ceiling");
        }
        let factor = f64_at(bytes, 24);
        if !(factor.is_finite() && factor > 1.0) {
            return header("the factor is not finite and above 1");
        }
        limits.check(LimitKind::EntitiesPerArtifact, u64::from(entries)).map_err(StraddlerError::Limit)?;

        let mut sizes = Vec::with_capacity(levels as usize);
        for l in 0..MAX_HEIGHT as usize {
            let s = f64_at(bytes, 32 + 8 * l);
            if l < levels as usize {
                if !(s.is_finite() && s > 0.0) {
                    return Err(StraddlerError::Scale { level: l as u16 });
                }
                sizes.push(s);
            } else if s != 0.0 {
                return Err(StraddlerError::Scale { level: l as u16 });
            }
        }
        let scale = LevelScale { factor, sizes };

        let read_box = |at: usize| -> Option<Aabb> {
            let mut v = [0.0f64; 4];
            for (k, slot) in v.iter_mut().enumerate() {
                *slot = check_value(f64_at(bytes, at + 8 * k)).ok()?;
            }
            Aabb::new([v[0], v[1]], [v[2], v[3]]).ok()
        };

        let mut parsed: Vec<Page> = Vec::with_capacity(pages as usize);
        let mut next = 0usize;
        let mut previous = 0u16;
        for i in 0..pages as usize {
            let at = STRADDLER_HEADER_LEN + PAGE_LEN * i;
            let shape = |reason| Err(StraddlerError::List { list: i as u32, reason });
            let level = u16_at(bytes, at);
            if u16_at(bytes, at + 2) != 0 {
                return shape("a reserved field is set");
            }
            if u32::from(level) >= levels || level < previous {
                return shape("levels are not ascending and below the level count");
            }
            previous = level;
            let count = u32_at(bytes, at + 4) as usize;
            if count == 0 || count > STRADDLER_PAGE {
                return shape("a page has no entries or more than a page holds");
            }
            let bounds = read_box(at + 8).ok_or(StraddlerError::Bounds { index: i as u32, list: true })?;
            parsed.push(Page { level, bounds, first: next, count });
            next += count;
        }
        if next != entries as usize {
            return Err(StraddlerError::List {
                list: pages.saturating_sub(1),
                reason: "entries do not tile the pages",
            });
        }

        let base = STRADDLER_HEADER_LEN + PAGE_LEN * pages as usize;
        let mut out: Vec<StraddlerRef> = Vec::with_capacity(entries as usize);
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        for (pi, page) in parsed.iter().enumerate() {
            let mut union_box: Option<Aabb> = None;
            for k in page.first..page.first + page.count {
                let at = base + STRADDLER_ENTRY_LEN * k;
                let b = read_box(at).ok_or(StraddlerError::Bounds { index: k as u32, list: false })?;
                let id = u32_at(bytes, at + 32);
                let eid = EntityId::new(id).map_err(|_| StraddlerError::Duplicate { id })?;
                if !seen.insert(id) {
                    return Err(StraddlerError::Duplicate { id });
                }
                // the coarsest level this entity exceeds: it must be this page's
                if scale.level_of(extent(&b)) != Some(page.level) {
                    return Err(StraddlerError::Placement { id, level: page.level });
                }
                union_box = Some(union_box.map_or(b, |u| union(&u, &b)));
                out.push(StraddlerRef {
                    id: eid,
                    bounds: b,
                    payload: PayloadRef {
                        block_id: BlockId(u32_at(bytes, at + 36)),
                        slot: u32_at(bytes, at + 40),
                    },
                    level: page.level,
                });
            }
            if union_box != Some(page.bounds) {
                return Err(StraddlerError::List {
                    list: pi as u32,
                    reason: "bounds are not the union of the entries",
                });
            }
        }
        Ok(StraddlerLists { scale, pages: parsed, entries: out })
    }

    /// The scale the entries were classified against.
    pub fn scale(&self) -> &LevelScale {
        &self.scale
    }

    /// Entries in all lists.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether there are no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Pages in all lists.
    pub fn pages(&self) -> usize {
        self.pages.len()
    }

    /// Entries at a level.
    pub fn level_len(&self, level: u16) -> usize {
        self.pages.iter().filter(|p| p.level == level).map(|p| p.count).sum()
    }

    /// Every entry, finest level first, in page order.
    pub fn entries(&self) -> &[StraddlerRef] {
        &self.entries
    }

    /// The straddlers whose bounds touch `q` (closed boxes), finest level first, in page order. A page
    /// whose bounds miss `q` is skipped whole. Each entity is yielded at most once because the block
    /// lists it once (STR-08).
    pub fn query<'a>(&'a self, q: &'a Aabb) -> impl Iterator<Item = StraddlerRef> + 'a {
        self.pages.iter().filter(move |p| p.bounds.intersects(q)).flat_map(move |p| {
            self.entries[p.first..p.first + p.count].iter().filter(move |e| e.bounds.intersects(q)).copied()
        })
    }

    /// The bytes of the block a reader fetches for `q`: the header and every page descriptor, which it
    /// needs to choose pages, and the entries of the pages whose bounds touch `q`.
    pub fn read_bytes(&self, q: &Aabb) -> u64 {
        let touched: usize = self.pages.iter().filter(|p| p.bounds.intersects(q)).map(|p| p.count).sum();
        (STRADDLER_HEADER_LEN + PAGE_LEN * self.pages.len() + STRADDLER_ENTRY_LEN * touched) as u64
    }
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
pub fn query_viewport(dir: &SpatialDirectory, long: Option<&StraddlerLists>, q: &Aabb) -> ViewportHits {
    ViewportHits {
        cells: dir.query(q).collect(),
        long: long.map(|l| l.query(q).collect()).unwrap_or_default(),
    }
}

/// The entities of a viewport, each once. `cell_entities` are the entity ids the caller decoded from
/// the hit cells' blocks; an entity that is also in a straddler list (a writer that referenced it from
/// both) is yielded where it is first met, which is the cell, and not again. The order is the order of
/// `cell_entities`, then the straddlers by level and id.
pub fn unique_entities(
    cell_entities: impl IntoIterator<Item = EntityId>,
    hits: &ViewportHits,
) -> Vec<EntityId> {
    let mut seen: BTreeSet<EntityId> = BTreeSet::new();
    let mut out = Vec::new();
    for id in cell_entities.into_iter().chain(hits.long.iter().map(|s| s.id)) {
        if seen.insert(id) {
            out.push(id);
        }
    }
    out
}

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
pub fn congestion(dir: &SpatialDirectory, long: Option<&StraddlerLists>) -> CongestionReport {
    let sizes: Vec<f64> = long.map_or_else(Vec::new, |l| l.scale.sizes.clone());
    let top = dir.height().saturating_sub(1) as u16;
    let mut levels = Vec::new();
    let mut straddlers = 0u64;
    let mut coarsest_page = 0u64;
    for (l, size) in sizes.iter().enumerate() {
        let n = long.map_or(0, |x| x.level_len(l as u16)) as u64;
        let pages = long.map_or(0, |x| x.pages.iter().filter(|p| usize::from(p.level) == l).count()) as u64;
        let list_bytes = PAGE_LEN as u64 * pages + STRADDLER_ENTRY_LEN as u64 * n;
        if n > 0 {
            coarsest_page = STRADDLER_ENTRY_LEN as u64 * n.min(STRADDLER_PAGE as u64);
        }
        straddlers += n;
        levels.push(LevelCongestion {
            level: l as u16,
            cell_size: *size,
            entities: if l == 0 { dir.entities() + n } else { n },
            straddlers: n,
            pages,
            list_bytes,
        });
    }
    let (block_bytes, index_bytes) = long.map_or((0, 0), |x| {
        let index = (STRADDLER_HEADER_LEN + PAGE_LEN * x.pages.len()) as u64;
        (index + STRADDLER_ENTRY_LEN as u64 * x.entries.len() as u64, index)
    });
    let root_references = long.map_or(0, |x| x.level_len(top)) as u64;
    let coarsest_bytes = NODE_LEN as u64 + index_bytes + coarsest_page;
    CongestionReport {
        local_entities: dir.entities(),
        straddlers,
        levels,
        root_references,
        block_bytes,
        index_bytes,
        coarsest_bytes,
    }
}

/// The baseline the straddler lists are measured against: promote each entity to the lowest node of
/// `built` whose bounds contain it (the root when none lower does). Returns how many of `entities` land
/// at each level, finest first. Nothing writes this placement; it exists so the report can say what the
/// naive answer would have put in the root.
pub fn ancestor_placement(built: &BuiltDirectory, entities: &[EntityBox]) -> Vec<u64> {
    let nodes = built_nodes(built);
    let height = nodes.last().map_or(1, |n| usize::from(n.3) + 1);
    let mut counts = vec![0u64; height];
    for e in entities {
        let level;
        // walk down from the root while one child contains the entity
        let mut at = nodes.len() - 1;
        'down: loop {
            let (_, first, count, l) = nodes[at];
            if l == 0 {
                level = 0;
                break;
            }
            for c in first..first + count {
                if nodes[c as usize].0.contains(&e.bounds) {
                    at = c as usize;
                    continue 'down;
                }
            }
            level = usize::from(l);
            break;
        }
        counts[level] += 1;
    }
    counts
}
