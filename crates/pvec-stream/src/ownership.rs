//! Single payload ownership with duplicated spatial references (issue P5.2).
//!
//! An entity that spans several cells is stored once: its geometry payload lives in exactly one row of
//! one hot block (the block of the cell that owns it, which is how P5.1 already places it), and the other
//! cells it reaches hold only a [`PayloadRef`], 8 bytes naming that block and row. Forcing a long entity
//! up to a common ancestor would congest the root, and copying its geometry into every cell would store
//! it many times; a reference is the cheap middle, and the panel's finding 4 allows exactly that.
//!
//! Three pieces make the policy:
//!
//! * the writer-side placement pass, [`place`], which puts each entity's reference into every cell its
//!   bounds intersect, up to a ceiling ([`DuplicationPolicy`]). An entity over the ceiling is not
//!   duplicated further: it comes back as a [`Straddler`] for the P5.3 path;
//! * the accounting, [`DuplicationReport`], so the cost of the duplication is a number on the artifact
//!   and not a guess;
//! * the read-side deduplication, [`gather`] and [`read_viewport`], so a payload is decoded once per
//!   viewport however many of the touched cells listed it.
//!
//! The reference arrays ([`CellRefs`]) have their own byte layout, checked on parse like any other
//! untrusted input. The rules are in [`OWNERSHIP_RULES`] and `docs/STREAMING.md` chapter 2.

use crate::directory::{Aabb, CellId, SpatialDirectory};
use pvec_container::blocks::hot::HotBlockView;
use pvec_container::directory::{BlockEntry, BlockKind};
use pvec_container::reader::ReadError;
use pvec_container::source::BlockSource;
use pvec_container::{BlockId, ContainerReader};
use pvec_entity::EntityId;
use std::collections::{BTreeSet, HashSet};

/// The most bytes a spatial reference may take. [`PayloadRef`] is held to it by a `const` assertion, so
/// a field added to it that pushes it past this fails the build.
pub const REF_BUDGET: usize = 8;

/// Bytes of one encoded reference.
pub const PAYLOAD_REF_LEN: usize = 8;

/// A reference to one entity's payload: the hot block that holds it and its row in that block.
///
/// Both halves are `u32`: a block id is a position in the block directory (which the reader's limits keep
/// far below 2^32), and a row is a position in one block (a block is capped by `max_block_bytes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PayloadRef {
    /// The block.
    pub block_id: BlockId,
    /// The row inside the block.
    pub slot: u32,
}

const _: () = assert!(core::mem::size_of::<PayloadRef>() <= REF_BUDGET);
const _: () = assert!(PAYLOAD_REF_LEN <= REF_BUDGET);

impl PayloadRef {
    /// The 8 byte encoding: the block id, then the row, little endian.
    pub fn encode(self) -> [u8; PAYLOAD_REF_LEN] {
        let mut b = [0u8; PAYLOAD_REF_LEN];
        b[..4].copy_from_slice(&self.block_id.0.to_le_bytes());
        b[4..].copy_from_slice(&self.slot.to_le_bytes());
        b
    }

    /// The reference an encoding holds. Every 8 bytes decode to some reference; whether it names a
    /// payload is checked when it is read ([`read_viewport`]).
    pub fn decode(bytes: [u8; PAYLOAD_REF_LEN]) -> PayloadRef {
        PayloadRef {
            block_id: BlockId(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            slot: u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        }
    }
}

/// The most cells an entity may be referenced from unless the writer says otherwise.
///
/// Measured on the fixtures of `docs/STREAMING.md` section 2.4: it keeps the plan-like mix and the
/// stress-long fixture well inside the reference budget, and an entity that reaches more cells than this
/// is a straddler in the sense of ADR 0005 and goes to P5.3.
pub const DEFAULT_MAX_CELLS: u32 = 16;

const MAGIC: [u8; 4] = *b"PVR1";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 16;

/// The refusals, rule by rule (`docs/STREAMING.md` section 2.6).
pub const OWNERSHIP_RULES: &[(&str, &str)] = &[
    (
        "OWN-01",
        "a reference block shorter than its header, or whose length is not the header plus 8 bytes per declared cell and per declared reference",
    ),
    ("OWN-02", "a magic, version or flags field that is not defined"),
    (
        "OWN-03",
        "a cell table that is wrong: a cell id of 0 or one that does not ascend strictly, an empty cell, or ends that do not reach the declared reference count",
    ),
    ("OWN-04", "references inside a cell that do not ascend strictly"),
    ("OWN-05", "a placement input that gives one entity id twice, or one payload to two entities"),
    ("OWN-06", "an entity whose bounds intersect no cell of the directory"),
    (
        "OWN-07",
        "a duplication ceiling of 0, or a placement that would emit more references than a u32 can count",
    ),
    (
        "OWN-08",
        "a reference that names no payload: a block that does not exist or is not a hot block, or a row past the end of the block",
    ),
    ("OWN-09", "the container refused the read, or a block is not a valid hot block"),
    ("OWN-10", "more than one reference block in the file"),
    (
        "OWN-11",
        "a reference block whose entry disagrees with its bytes: it is in a cell, or its count is not the number of cells the arrays hold",
    ),
];

/// Why a placement, a reference array or a read was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum OwnershipError {
    /// The length of a reference block (OWN-01).
    Length {
        /// Its length.
        length: u64,
        /// The length its header implies, or the header length when it is shorter.
        expected: u64,
    },
    /// A header field (OWN-02).
    Header {
        /// What is wrong.
        reason: &'static str,
    },
    /// The cell table (OWN-03).
    CellTable {
        /// What is wrong.
        reason: &'static str,
    },
    /// The order of references (OWN-04).
    Order {
        /// The cell.
        cell: u32,
    },
    /// A placement input (OWN-05).
    Input {
        /// What is wrong.
        reason: &'static str,
        /// The entity.
        entity: u32,
    },
    /// An entity outside every cell (OWN-06).
    Outside {
        /// The entity.
        entity: u32,
    },
    /// A ceiling of 0 (OWN-07).
    Ceiling,
    /// More references than the array can count (OWN-07).
    TooManyRefs,
    /// A reference that names no payload (OWN-08).
    NoPayload {
        /// The block.
        block: u32,
        /// The row.
        slot: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// The container refused (OWN-09).
    Read(ReadError),
    /// A block is not a valid hot block (OWN-09).
    Block {
        /// The block.
        block: u32,
        /// What is wrong.
        reason: &'static str,
    },
    /// More than one reference block (OWN-10).
    DuplicateBlock,
    /// The entry of the reference block disagrees with its bytes (OWN-11).
    Entry {
        /// What is wrong.
        reason: &'static str,
    },
}

impl OwnershipError {
    /// The rule identifier of `docs/STREAMING.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            OwnershipError::Length { .. } => "OWN-01",
            OwnershipError::Header { .. } => "OWN-02",
            OwnershipError::CellTable { .. } => "OWN-03",
            OwnershipError::Order { .. } => "OWN-04",
            OwnershipError::Input { .. } => "OWN-05",
            OwnershipError::Outside { .. } => "OWN-06",
            OwnershipError::Ceiling | OwnershipError::TooManyRefs => "OWN-07",
            OwnershipError::NoPayload { .. } => "OWN-08",
            OwnershipError::Read(_) | OwnershipError::Block { .. } => "OWN-09",
            OwnershipError::DuplicateBlock => "OWN-10",
            OwnershipError::Entry { .. } => "OWN-11",
        }
    }
}

impl core::fmt::Display for OwnershipError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            OwnershipError::Length { length, expected } => {
                write!(f, "{rule}: the block is {length} bytes and its header implies {expected}")
            }
            OwnershipError::Header { reason } | OwnershipError::CellTable { reason } => {
                write!(f, "{rule}: {reason}")
            }
            OwnershipError::Order { cell } => {
                write!(f, "{rule}: the references of cell {cell} do not ascend")
            }
            OwnershipError::Input { reason, entity } => write!(f, "{rule}: entity {entity}: {reason}"),
            OwnershipError::Outside { entity } => write!(f, "{rule}: entity {entity} is outside every cell"),
            OwnershipError::Ceiling => write!(f, "{rule}: the ceiling must be at least 1"),
            OwnershipError::TooManyRefs => write!(f, "{rule}: more references than a u32 can count"),
            OwnershipError::NoPayload { block, slot, reason } => {
                write!(f, "{rule}: block {block} row {slot}: {reason}")
            }
            OwnershipError::Read(e) => write!(f, "{rule}: {e}"),
            OwnershipError::Block { block, reason } => write!(f, "{rule}: block {block}: {reason}"),
            OwnershipError::DuplicateBlock => write!(f, "{rule}: the file has more than one reference block"),
            OwnershipError::Entry { reason } => write!(f, "{rule}: {reason}"),
        }
    }
}

impl std::error::Error for OwnershipError {}

impl From<ReadError> for OwnershipError {
    fn from(e: ReadError) -> OwnershipError {
        OwnershipError::Read(e)
    }
}

/// How many cells one entity may be referenced from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DuplicationPolicy {
    max_cells: u32,
}

impl DuplicationPolicy {
    /// A ceiling of `max_cells`, which is at least 1 (the owning cell always lists its own entity).
    pub fn new(max_cells: u32) -> Result<DuplicationPolicy, OwnershipError> {
        if max_cells == 0 {
            return Err(OwnershipError::Ceiling);
        }
        Ok(DuplicationPolicy { max_cells })
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
    /// Where its payload is: its one row in its one block.
    pub payload: PayloadRef,
}

/// An entity that intersects more cells than the ceiling allows. It is not duplicated; P5.3 decides
/// where a reader finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Straddler {
    /// The entity.
    pub id: EntityId,
    /// Where its payload is.
    pub payload: PayloadRef,
}

/// The references of every cell: for each cell that has any, its references in ascending order.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CellRefs {
    /// Cell ids, ascending, none 0.
    cells: Vec<u32>,
    /// For each cell, the end of its references in `refs` (it starts where the previous one ended).
    ends: Vec<u32>,
    refs: Vec<PayloadRef>,
}

impl CellRefs {
    /// How many cells have references.
    pub fn cells(&self) -> usize {
        self.cells.len()
    }

    /// All references, in every cell.
    pub fn total_refs(&self) -> usize {
        self.refs.len()
    }

    /// The references of one cell, ascending, or none.
    pub fn refs_of(&self, cell: CellId) -> &[PayloadRef] {
        match self.cells.binary_search(&cell.get()) {
            Ok(i) => {
                let start = if i == 0 { 0 } else { self.ends[i - 1] as usize };
                &self.refs[start..self.ends[i] as usize]
            }
            Err(_) => &[],
        }
    }

    /// The reference block of a file: its position in the block directory, or `None` when the file has
    /// none. This reads the directory entries only. More than one is refused (OWN-10).
    pub fn find(entries: &[BlockEntry]) -> Result<Option<BlockId>, OwnershipError> {
        let mut found = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.block_kind() == Some(BlockKind::CellRefs))
            .map(|(i, _)| BlockId(i as u32));
        let first = found.next();
        if found.next().is_some() {
            return Err(OwnershipError::DuplicateBlock);
        }
        Ok(first)
    }

    /// The reference arrays of a file, read now: the block is fetched here and not when the file or its
    /// spatial directory is opened, so a reader that never asks for references never pays for them.
    /// `None` when the file has no reference block. The entry has to agree with the bytes (OWN-11): the
    /// block is in no cell and its count is the number of cells the arrays hold.
    pub fn open<S: BlockSource>(reader: &ContainerReader<S>) -> Result<Option<CellRefs>, OwnershipError> {
        let Some(id) = CellRefs::find(reader.entries())? else { return Ok(None) };
        let entry = reader.entries()[id.0 as usize];
        if entry.cell != 0 {
            return Err(OwnershipError::Entry { reason: "the reference block is in a cell" });
        }
        let block = reader.read_block(id)?;
        let refs = CellRefs::parse(block.as_bytes())?;
        if entry.count as usize != refs.cells() {
            return Err(OwnershipError::Entry {
                reason: "the entry's count is not the number of cells in the block",
            });
        }
        Ok(Some(refs))
    }

    /// The bytes of the reference arrays: a 16 byte header (`PVR1`, version, flags, cell count, reference
    /// count), the cell ids, the end offset of each cell's references, then the references.
    pub fn encode(&self) -> Vec<u8> {
        let mut out =
            Vec::with_capacity(HEADER_LEN + 8 * self.cells.len() + PAYLOAD_REF_LEN * self.refs.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(self.cells.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.refs.len() as u32).to_le_bytes());
        for c in &self.cells {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for e in &self.ends {
            out.extend_from_slice(&e.to_le_bytes());
        }
        for r in &self.refs {
            out.extend_from_slice(&r.encode());
        }
        out
    }

    /// The reference arrays of `bytes`. The length is held to the header before anything is allocated,
    /// so a header that claims four billion references costs nothing.
    pub fn parse(bytes: &[u8]) -> Result<CellRefs, OwnershipError> {
        let u32_at = |at: usize| u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        if bytes.len() < HEADER_LEN {
            return Err(OwnershipError::Length { length: bytes.len() as u64, expected: HEADER_LEN as u64 });
        }
        if bytes[..4] != MAGIC {
            return Err(OwnershipError::Header { reason: "the magic is not PVR1" });
        }
        if u16::from_le_bytes([bytes[4], bytes[5]]) != VERSION {
            return Err(OwnershipError::Header { reason: "the version is not 1" });
        }
        if u16::from_le_bytes([bytes[6], bytes[7]]) != 0 {
            return Err(OwnershipError::Header { reason: "a flag is set" });
        }
        let (n, r) = (u64::from(u32_at(8)), u64::from(u32_at(12)));
        let expected = HEADER_LEN as u64 + 8 * n + PAYLOAD_REF_LEN as u64 * r;
        if bytes.len() as u64 != expected {
            return Err(OwnershipError::Length { length: bytes.len() as u64, expected });
        }
        let (n, r) = (n as usize, r as usize);
        let cells: Vec<u32> = (0..n).map(|i| u32_at(HEADER_LEN + 4 * i)).collect();
        let ends: Vec<u32> = (0..n).map(|i| u32_at(HEADER_LEN + 4 * n + 4 * i)).collect();
        if cells.first() == Some(&0) {
            return Err(OwnershipError::CellTable { reason: "a cell id is 0, which is no cell" });
        }
        if cells.windows(2).any(|w| w[0] >= w[1]) {
            return Err(OwnershipError::CellTable { reason: "cell ids do not ascend strictly" });
        }
        let mut prev = 0u32;
        for e in &ends {
            if *e <= prev {
                return Err(OwnershipError::CellTable {
                    reason: "a cell has no references, or the ends do not ascend",
                });
            }
            prev = *e;
        }
        if prev as usize != r {
            return Err(OwnershipError::CellTable { reason: "the ends do not reach the reference count" });
        }
        let base = HEADER_LEN + 8 * n;
        let refs: Vec<PayloadRef> = (0..r)
            .map(|i| {
                let at = base + PAYLOAD_REF_LEN * i;
                let mut b = [0u8; PAYLOAD_REF_LEN];
                b.copy_from_slice(&bytes[at..at + PAYLOAD_REF_LEN]);
                PayloadRef::decode(b)
            })
            .collect();
        let mut start = 0usize;
        for (cell, end) in cells.iter().zip(&ends) {
            let run = &refs[start..*end as usize];
            if run.windows(2).any(|w| w[0] >= w[1]) {
                return Err(OwnershipError::Order { cell: *cell });
            }
            start = *end as usize;
        }
        Ok(CellRefs { cells, ends, refs })
    }
}

/// What the duplication cost: the numbers that say whether the policy is cheap enough.
#[derive(Clone, Debug, PartialEq)]
pub struct DuplicationReport {
    /// Entities that got references.
    pub entities: u64,
    /// Entities routed to the straddler path (they got none).
    pub straddlers: u64,
    /// References emitted.
    pub references: u64,
    /// Mean references per entity that got any.
    pub mean: f64,
    /// 99th percentile of references per entity (nearest rank).
    pub p99: u32,
    /// Most references of one entity.
    pub max: u32,
    /// `references` times [`PAYLOAD_REF_LEN`].
    pub reference_bytes: u64,
    /// Decoded bytes of the hot blocks (see [`payload_bytes`]).
    pub payload_bytes: u64,
}

impl DuplicationReport {
    /// Reference bytes over payload bytes. 0 when there are neither, infinite when there are references
    /// and no payload.
    pub fn fraction(&self) -> f64 {
        if self.payload_bytes == 0 {
            return if self.reference_bytes == 0 { 0.0 } else { f64::INFINITY };
        }
        self.reference_bytes as f64 / self.payload_bytes as f64
    }
}

/// The outcome of the placement pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The per-cell arrays.
    pub refs: CellRefs,
    /// Entities over the ceiling, for P5.3.
    pub straddlers: Vec<Straddler>,
    /// The accounting.
    pub report: DuplicationReport,
}

/// Assigns references. Each entity's payload stays in its one row of its one block; its reference goes
/// into every cell of `dir` whose bounds intersect the entity's, unless that is more than the ceiling,
/// in which case it goes nowhere and the entity is returned as a [`Straddler`].
///
/// The cost is one directory query per entity, which stops as soon as the ceiling is passed, so a long
/// entity costs the ceiling and not the number of cells it crosses. `payload_bytes` is the figure the
/// report divides by (see [`payload_bytes`]).
pub fn place(
    dir: &SpatialDirectory,
    entities: &[PlacedEntity],
    policy: DuplicationPolicy,
    payload_bytes: u64,
) -> Result<Placement, OwnershipError> {
    let mut ids: HashSet<EntityId> = HashSet::with_capacity(entities.len());
    let mut payloads: HashSet<PayloadRef> = HashSet::with_capacity(entities.len());
    let mut pairs: Vec<(u32, PayloadRef)> = Vec::new();
    let mut straddlers = Vec::new();
    let mut per_entity: Vec<u32> = Vec::with_capacity(entities.len());
    let ceiling = policy.max_cells() as usize;
    let probe = ceiling.saturating_add(1);
    for e in entities {
        if !ids.insert(e.id) {
            return Err(OwnershipError::Input { reason: "the entity is given twice", entity: e.id.index() });
        }
        if !payloads.insert(e.payload) {
            return Err(OwnershipError::Input {
                reason: "its payload is another entity's: a payload has one owner",
                entity: e.id.index(),
            });
        }
        let cells: Vec<CellId> = dir.query(&e.bounds).take(probe).map(|c| c.id).collect();
        if cells.is_empty() {
            return Err(OwnershipError::Outside { entity: e.id.index() });
        }
        if cells.len() > ceiling {
            straddlers.push(Straddler { id: e.id, payload: e.payload });
            continue;
        }
        if pairs.len() + cells.len() > u32::MAX as usize {
            return Err(OwnershipError::TooManyRefs);
        }
        per_entity.push(cells.len() as u32);
        pairs.extend(cells.iter().map(|c| (c.get(), e.payload)));
    }
    pairs.sort_unstable();
    let mut refs = CellRefs::default();
    for (cell, payload) in pairs {
        if refs.cells.last() != Some(&cell) {
            if !refs.cells.is_empty() {
                refs.ends.push(refs.refs.len() as u32);
            }
            refs.cells.push(cell);
        }
        refs.refs.push(payload);
    }
    if !refs.cells.is_empty() {
        refs.ends.push(refs.refs.len() as u32);
    }
    per_entity.sort_unstable();
    let n = per_entity.len();
    let references = refs.refs.len() as u64;
    let report = DuplicationReport {
        entities: n as u64,
        straddlers: straddlers.len() as u64,
        references,
        mean: if n == 0 { 0.0 } else { references as f64 / n as f64 },
        p99: if n == 0 { 0 } else { per_entity[(n * 99).div_ceil(100) - 1] },
        max: per_entity.last().copied().unwrap_or(0),
        reference_bytes: references * PAYLOAD_REF_LEN as u64,
        payload_bytes,
    };
    Ok(Placement { refs, straddlers, report })
}

/// Decoded bytes of the hot blocks in `entries`: the payload the reference bytes are measured against.
pub fn payload_bytes(entries: &[BlockEntry]) -> u64 {
    entries
        .iter()
        .filter(|e| e.block_kind() == Some(BlockKind::Hot))
        .map(|e| u64::from(e.uncompressed_length))
        .sum()
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

/// Every payload slot of every hot block, in block order. This is how a writer learns the block ids that
/// only exist once the container is laid out, and how a test counts slots against entities.
pub fn scan_payloads<S: BlockSource>(
    reader: &ContainerReader<S>,
) -> Result<Vec<PayloadSlot>, OwnershipError> {
    let mut out = Vec::new();
    for (i, entry) in reader.entries().iter().enumerate() {
        if entry.block_kind() != Some(BlockKind::Hot) {
            continue;
        }
        let block = reader.read_block(BlockId(i as u32))?;
        let view = block.hot()?;
        for (slot, raw) in view.entity_ids().iter().enumerate() {
            let id = EntityId::new(*raw).map_err(|_| OwnershipError::Block {
                block: i as u32,
                reason: "an entity id is out of range",
            })?;
            out.push(PayloadSlot {
                id,
                payload: PayloadRef { block_id: BlockId(i as u32), slot: slot as u32 },
                cell: entry.cell,
            });
        }
    }
    Ok(out)
}

/// Every advanced curve of the file, in record order (ascending entity id), each as a reference to its
/// row of the record block. The advanced set stays one set in cell 0, because a composite names other
/// records of its own block; what a cell gets is a reference to the row, like a hot payload, and a
/// reader that finds such a row in a viewport fetches the whole set once ([`advanced_rows`]). Empty when
/// the file has no advanced curves.
pub fn scan_advanced<S: BlockSource>(
    reader: &ContainerReader<S>,
) -> Result<Vec<PayloadSlot>, OwnershipError> {
    let Some(set) = reader.advanced()? else { return Ok(Vec::new()) };
    let records = reader.entries().iter().position(|e| e.block_kind() == Some(BlockKind::Records)).ok_or(
        OwnershipError::Block { block: 0, reason: "the file has advanced curves and no record block" },
    )?;
    let view = set.view()?;
    let mut out = Vec::with_capacity(view.len());
    for row in 0..view.len() {
        let rec = view
            .record(row)
            .ok_or(OwnershipError::Block { block: records as u32, reason: "a record is missing" })?;
        let id = EntityId::new(rec.id).map_err(|_| OwnershipError::Block {
            block: records as u32,
            reason: "an entity id is out of range",
        })?;
        out.push(PayloadSlot {
            id,
            payload: PayloadRef { block_id: BlockId(records as u32), slot: row as u32 },
            cell: 0,
        });
    }
    Ok(out)
}

/// The references a viewport touches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewportRefs {
    /// References the touched cells listed, before deduplication.
    pub raw: usize,
    /// Each payload once, ascending.
    pub refs: Vec<PayloadRef>,
}

/// The references of every cell the viewport touches, each payload once. This is the read-side
/// deduplication: an entity that spans nine touched cells is listed nine times and comes out once.
pub fn gather(dir: &SpatialDirectory, refs: &CellRefs, viewport: &Aabb) -> ViewportRefs {
    let mut all: Vec<PayloadRef> = Vec::new();
    for cell in dir.query(viewport) {
        all.extend_from_slice(refs.refs_of(cell.id));
    }
    let raw = all.len();
    all.sort_unstable();
    all.dedup();
    ViewportRefs { raw, refs: all }
}

/// The rows of the advanced record block that `gathered` names, ascending: when this is not empty the
/// viewport needs the advanced set (`ContainerReader::advanced`), and which curves of it are in view is
/// these rows. A row past the end of the record block, or a reference into a block that is neither hot nor
/// the record block, is refused (OWN-08). This reads no block.
pub fn advanced_rows<S: BlockSource>(
    reader: &ContainerReader<S>,
    gathered: &ViewportRefs,
) -> Result<Vec<u32>, OwnershipError> {
    let mut rows = Vec::new();
    for r in &gathered.refs {
        let entry = check_payload(reader.entries(), r)?;
        if entry.block_kind() == Some(BlockKind::Records) {
            rows.push(r.slot);
        }
    }
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

/// The entry a reference names, if it names a row of a hot block or of the record block.
fn check_payload<'a>(entries: &'a [BlockEntry], r: &PayloadRef) -> Result<&'a BlockEntry, OwnershipError> {
    let bad = |reason| OwnershipError::NoPayload { block: r.block_id.0, slot: r.slot, reason };
    let entry = entries.get(r.block_id.0 as usize).ok_or(bad("no such block"))?;
    if !matches!(entry.block_kind(), Some(BlockKind::Hot | BlockKind::Records)) {
        return Err(bad("not a hot block"));
    }
    if r.slot >= entry.count {
        return Err(bad("past the last row"));
    }
    Ok(entry)
}

/// Decodes each payload of `gathered` once and calls `visit` with the entity, its block and its row.
/// The blocks are fetched through the reader's plan, so blocks close together arrive in one request, and
/// each is parsed once however many rows are wanted from it. Returns how many payloads were visited.
pub fn read_viewport<S: BlockSource>(
    reader: &ContainerReader<S>,
    gathered: &ViewportRefs,
    mut visit: impl FnMut(EntityId, &HotBlockView<'_>, usize),
) -> Result<usize, OwnershipError> {
    let entries = reader.entries();
    let mut wanted: BTreeSet<BlockId> = BTreeSet::new();
    for r in &gathered.refs {
        // a row of the advanced record block is checked and left to `advanced_rows`
        if check_payload(entries, r)?.block_kind() == Some(BlockKind::Hot) {
            wanted.insert(r.block_id);
        }
    }
    let ids: Vec<BlockId> = wanted.into_iter().collect();
    let blocks = reader.read_blocks(&ids)?;
    let mut visited = 0usize;
    let mut views: Vec<Option<HotBlockView<'_>>> = (0..blocks.len()).map(|_| None).collect();
    for r in gathered.refs.iter().filter(|r| ids.binary_search(&r.block_id).is_ok()) {
        let k = ids.binary_search(&r.block_id).map_err(|_| OwnershipError::Block {
            block: r.block_id.0,
            reason: "the block was not fetched",
        })?;
        if views[k].is_none() {
            views[k] = Some(blocks[k].hot()?);
        }
        let view = views[k]
            .as_ref()
            .ok_or(OwnershipError::Block { block: r.block_id.0, reason: "the block did not parse" })?;
        let slot = r.slot as usize;
        let raw = *view.entity_ids().get(slot).ok_or(OwnershipError::NoPayload {
            block: r.block_id.0,
            slot: r.slot,
            reason: "past the last row",
        })?;
        let id = EntityId::new(raw).map_err(|_| OwnershipError::Block {
            block: r.block_id.0,
            reason: "an entity id is out of range",
        })?;
        visit(id, view, slot);
        visited += 1;
    }
    Ok(visited)
}
