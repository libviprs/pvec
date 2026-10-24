//! Arena blocks for variable-length advanced geometry (issue P4.3).
//!
//! One arena block holds one payload kind (knots, weights, control points, composite segment lists,
//! trim domains) for every curve in a file. A curve does not carry its numbers: it carries an index
//! into the block's offset table, and the table entry is `(start, len)` into the arena, so a curve's
//! payload is located by two integer reads and is one contiguous run. The layout is chapter 3 of
//! `docs/CONTAINER.md`, rendered from the constants here and in [`crate::blocks::advanced`].

use crate::blocks::aligned::{AlignedBytes, f64s, u32s};
use crate::spec::FieldSpec;
use pvec_numeric::NumericError;
use std::collections::HashMap;

/// The four magic bytes of an arena block.
pub const ARENA_MAGIC: [u8; 4] = *b"PVA1";
/// The version this module reads and writes.
pub const ARENA_VERSION: u16 = 1;
/// The fixed header of an arena block, in bytes.
pub const ARENA_HEADER_LEN: usize = 32;
/// Bytes of one offset-table entry: `start: u32`, `len: u32`.
pub const OFFSET_ENTRY_LEN: usize = 8;
/// The table index that means "this curve has no payload of this kind".
pub const NO_ENTRY: u32 = u32::MAX;

/// The arena header fields, for `docs/CONTAINER.md` chapter 3.
pub const ARENA_HEADER_FIELDS: &[FieldSpec] = &[
    field("magic", 0, 4, "bytes", "`50 56 41 31` (`PVA1`)"),
    field("version", 4, 2, "u16", "1; a reader refuses any other"),
    field("payload_kind", 6, 2, "u16", "what the arena holds, from the table in section 3.2"),
    field("entry_count", 8, 4, "u32", "entries in the offset table"),
    field(
        "element_count",
        12,
        4,
        "u32",
        "elements in each payload column, the arena length the entries are checked against",
    ),
    field("block_length", 16, 4, "u32", "length of the whole block in bytes, equal to the bytes present"),
    field("reserved", 20, 12, "bytes", "zero"),
];

const fn field(
    name: &'static str,
    offset: usize,
    size: usize,
    encoding: &'static str,
    meaning: &'static str,
) -> FieldSpec {
    FieldSpec { name, offset, size, encoding, meaning }
}

/// What an arena block holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    /// Knot vectors, one `f64` column.
    Knots,
    /// Rational weights, one `f64` column.
    Weights,
    /// Control points, two `f64` columns (`x`, then `y`) of equal length.
    ControlPoints,
    /// Composite segment lists: `u32` record indices.
    Segments,
    /// Trim domains: `f64` pairs `(t1, t2)`, one pair per composite segment.
    Trims,
}

impl PayloadKind {
    /// Every kind.
    pub const ALL: [PayloadKind; 5] = [
        PayloadKind::Knots,
        PayloadKind::Weights,
        PayloadKind::ControlPoints,
        PayloadKind::Segments,
        PayloadKind::Trims,
    ];

    /// The id stored in the block header.
    pub fn id(self) -> u16 {
        match self {
            PayloadKind::Knots => 1,
            PayloadKind::Weights => 2,
            PayloadKind::ControlPoints => 3,
            PayloadKind::Segments => 4,
            PayloadKind::Trims => 5,
        }
    }

    /// The kind for an id.
    pub fn from_id(id: u16) -> Option<PayloadKind> {
        PayloadKind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// The name used in the document.
    pub fn name(self) -> &'static str {
        match self {
            PayloadKind::Knots => "Knots",
            PayloadKind::Weights => "Weights",
            PayloadKind::ControlPoints => "ControlPoints",
            PayloadKind::Segments => "Segments",
            PayloadKind::Trims => "Trims",
        }
    }

    /// The names of the payload columns, in the order they are written.
    pub fn columns(self) -> &'static [&'static str] {
        match self {
            PayloadKind::Knots => &["knots"],
            PayloadKind::Weights => &["weights"],
            PayloadKind::ControlPoints => &["x", "y"],
            PayloadKind::Segments => &["segments"],
            PayloadKind::Trims => &["trims"],
        }
    }

    /// What one element is, for the document.
    pub fn element(self) -> &'static str {
        match self {
            PayloadKind::Segments => "u32",
            _ => "f64",
        }
    }

    /// What an entry of this kind means, for the document.
    pub fn meaning(self) -> &'static str {
        match self {
            PayloadKind::Knots => "one knot vector per entry; the same vector may serve many curves",
            PayloadKind::Weights => "one weight per control point of a rational curve",
            PayloadKind::ControlPoints => {
                "all x of the entry, in one column, and all y at the same positions in the second"
            }
            PayloadKind::Segments => "record indices naming the segments of a composite curve, in order",
            PayloadKind::Trims => {
                "`t1`, `t2` per segment of a composite, flattened: twice as long as its segment list"
            }
        }
    }

    fn is_f64(self) -> bool {
        self != PayloadKind::Segments
    }

    fn f64_columns(self) -> usize {
        match self {
            PayloadKind::ControlPoints => 2,
            PayloadKind::Segments => 0,
            _ => 1,
        }
    }
}

/// Why an arena block, a record block or a curve was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArenaError {
    /// The block ends before its structure does.
    Truncated {
        /// Bytes needed.
        needed: u64,
        /// Bytes supplied.
        got: u64,
    },
    /// The first four bytes are not the block's magic.
    BadMagic,
    /// A version this reader does not implement.
    UnsupportedVersion {
        /// The version in the block.
        found: u16,
    },
    /// A payload or record kind id that is not in the table, or a block of the wrong kind.
    UnknownKind {
        /// The id.
        id: u16,
    },
    /// A reserved field is not zero.
    NonZeroReserved,
    /// A padding byte is not zero.
    NonZeroPadding,
    /// The declared block length differs from the bytes present, or part of the block is unused.
    LengthMismatch {
        /// The length in the header.
        declared: u32,
        /// The bytes supplied.
        actual: u64,
    },
    /// The buffer does not start on an 8-byte boundary.
    Misaligned,
    /// The host is not little-endian.
    HostEndianness,
    /// A table index at or past the end of the offset table.
    EntryOutOfRange {
        /// The index asked for.
        index: u32,
        /// Entries in the table.
        entries: u32,
    },
    /// An entry whose `start + len` passes the arena, or overflows.
    RangeOutOfArena {
        /// The entry's start.
        start: u32,
        /// The entry's length.
        len: u32,
        /// The arena length in elements.
        arena_len: u32,
    },
    /// An accessor used on a block of another payload kind.
    WrongPayload {
        /// The kind the block holds.
        held: PayloadKind,
    },
    /// The knot count is not `control points + degree + 1`.
    KnotCount {
        /// Knots present.
        knots: usize,
        /// Control points present.
        control_points: usize,
        /// The degree.
        degree: usize,
    },
    /// The weight array is not as long as the control-point array, is missing on a rational curve, or
    /// is present on a non-rational one.
    WeightCount {
        /// Weights present.
        weights: usize,
        /// Control points present.
        control_points: usize,
    },
    /// The x and y control-point columns have different lengths.
    ControlPointColumns {
        /// Length of x.
        x: usize,
        /// Length of y.
        y: usize,
    },
    /// The periodic flag is set; the profile has no wrapped periodic form.
    PeriodicUnsupported,
    /// Flag bits this version does not define are set, or a flag that the kind cannot have.
    InvalidFlags {
        /// The flags as read.
        bits: u16,
    },
    /// A record field that its kind does not use is not empty, or one that it needs is.
    RecordFields {
        /// The record.
        index: usize,
    },
    /// Record ids are not strictly ascending, or an id is at or above the NP-18 limit.
    IdsNotAscending {
        /// Position of the offending id.
        index: usize,
    },
    /// A segment that names a record that does not exist. A segment that names its own composite is not
    /// this error: it is a [`ArenaError::CompositeCycle`].
    SegmentOutOfRange {
        /// The composite record.
        record: usize,
        /// The index in its segment list.
        segment: u32,
    },
    /// A composite segment list that reaches itself.
    CompositeCycle {
        /// The record where the cycle was found.
        record: usize,
    },
    /// Composites nested deeper than NP-18 allows.
    NestingTooDeep {
        /// The record whose depth is too great.
        record: usize,
    },
    /// The trim array is not two numbers per segment.
    TrimCount {
        /// Trim values present.
        values: usize,
        /// Segments present.
        segments: usize,
    },
    /// A numeric policy rule refused a value or a count.
    Numeric(NumericError),
    /// A block with no records.
    EmptyBlock,
    /// (builder) the same entity id twice.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// (builder) a composite names an entity that was not pushed.
    UnknownSegment {
        /// The entity id.
        id: u32,
    },
    /// (builder) the block would not fit 32-bit lengths.
    TooLarge,
}

impl ArenaError {
    /// The rule identifier, `ARN-nn`, as listed in chapter 3 of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            ArenaError::Truncated { .. } => "ARN-01",
            ArenaError::BadMagic => "ARN-02",
            ArenaError::UnsupportedVersion { .. } => "ARN-03",
            ArenaError::UnknownKind { .. } => "ARN-04",
            ArenaError::NonZeroReserved => "ARN-05",
            ArenaError::NonZeroPadding => "ARN-06",
            ArenaError::LengthMismatch { .. } => "ARN-07",
            ArenaError::Misaligned => "ARN-08",
            ArenaError::HostEndianness => "ARN-09",
            ArenaError::EntryOutOfRange { .. } => "ARN-10",
            ArenaError::RangeOutOfArena { .. } => "ARN-11",
            ArenaError::WrongPayload { .. } => "ARN-12",
            ArenaError::KnotCount { .. } => "ARN-13",
            ArenaError::WeightCount { .. } => "ARN-14",
            ArenaError::ControlPointColumns { .. } => "ARN-15",
            ArenaError::PeriodicUnsupported => "ARN-16",
            ArenaError::InvalidFlags { .. } => "ARN-17",
            ArenaError::RecordFields { .. } => "ARN-18",
            ArenaError::IdsNotAscending { .. } => "ARN-19",
            ArenaError::SegmentOutOfRange { .. } => "ARN-20",
            ArenaError::CompositeCycle { .. } => "ARN-21",
            ArenaError::NestingTooDeep { .. } => "ARN-22",
            ArenaError::TrimCount { .. } => "ARN-23",
            ArenaError::Numeric(_) => "ARN-24",
            ArenaError::EmptyBlock => "ARN-25",
            ArenaError::DuplicateId { .. } => "ARN-26",
            ArenaError::UnknownSegment { .. } => "ARN-27",
            ArenaError::TooLarge => "ARN-28",
        }
    }
}

impl core::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            ArenaError::Truncated { needed, got } => {
                write!(f, "{rule}: the block needs {needed} bytes but only {got} are present")
            }
            ArenaError::BadMagic => write!(f, "{rule}: the block does not start with its magic bytes"),
            ArenaError::UnsupportedVersion { found } => {
                write!(f, "{rule}: version {found} is not one this reader implements")
            }
            ArenaError::UnknownKind { id } => {
                write!(f, "{rule}: kind id {id} is unknown or in the wrong slot")
            }
            ArenaError::NonZeroReserved => write!(f, "{rule}: a reserved field is not zero"),
            ArenaError::NonZeroPadding => write!(f, "{rule}: a padding byte is not zero"),
            ArenaError::LengthMismatch { declared, actual } => {
                write!(f, "{rule}: the header declares {declared} bytes but {actual} are present")
            }
            ArenaError::Misaligned => write!(f, "{rule}: the buffer does not start on an 8-byte boundary"),
            ArenaError::HostEndianness => write!(f, "{rule}: the host is not little-endian"),
            ArenaError::EntryOutOfRange { index, entries } => {
                write!(f, "{rule}: index {index} is past the end of a table of {entries} entries")
            }
            ArenaError::RangeOutOfArena { start, len, arena_len } => write!(
                f,
                "{rule}: the entry (start {start}, len {len}) does not fit in an arena of {arena_len} elements"
            ),
            ArenaError::WrongPayload { held } => {
                write!(f, "{rule}: this accessor does not apply to a {} arena", held.name())
            }
            ArenaError::KnotCount { knots, control_points, degree } => write!(
                f,
                "{rule}: {knots} knots do not match {control_points} control points at degree {degree}"
            ),
            ArenaError::WeightCount { weights, control_points } => {
                write!(f, "{rule}: {weights} weights for {control_points} control points")
            }
            ArenaError::ControlPointColumns { x, y } => {
                write!(f, "{rule}: the control-point columns have {x} x values and {y} y values")
            }
            ArenaError::PeriodicUnsupported => {
                write!(f, "{rule}: the periodic flag is set, and this profile has no wrapped periodic form")
            }
            ArenaError::InvalidFlags { bits } => write!(f, "{rule}: flags {bits:#06x} are not valid here"),
            ArenaError::RecordFields { index } => {
                write!(f, "{rule}: record {index} sets a field its kind does not use or lacks one it needs")
            }
            ArenaError::IdsNotAscending { index } => {
                write!(f, "{rule}: the id of record {index} does not ascend or is out of range")
            }
            ArenaError::SegmentOutOfRange { record, segment } => {
                write!(f, "{rule}: composite record {record} names record {segment}, which does not exist")
            }
            ArenaError::CompositeCycle { record } => {
                write!(f, "{rule}: record {record} is part of a composite cycle")
            }
            ArenaError::NestingTooDeep { record } => {
                write!(f, "{rule}: record {record} nests composites deeper than the NP-18 limit")
            }
            ArenaError::TrimCount { values, segments } => {
                write!(f, "{rule}: {values} trim values for {segments} segments, expected two per segment")
            }
            ArenaError::Numeric(n) => write!(f, "{rule}: the numeric policy refused a value ({})", n.rule()),
            ArenaError::EmptyBlock => write!(f, "{rule}: the block has no records"),
            ArenaError::DuplicateId { id } => write!(f, "{rule}: entity {id} was pushed twice"),
            ArenaError::UnknownSegment { id } => {
                write!(f, "{rule}: a composite names entity {id}, which was not pushed")
            }
            ArenaError::TooLarge => write!(f, "{rule}: the blocks would not fit 32-bit lengths"),
        }
    }
}

impl std::error::Error for ArenaError {}

/// The rules a reader refuses on, for the document: identifier and meaning.
pub const ARENA_RULES: &[(&str, &str)] = &[
    ("ARN-01", "the block is shorter than its structure requires"),
    ("ARN-02", "bad magic"),
    ("ARN-03", "unsupported version"),
    ("ARN-04", "an unknown payload or record kind id, or an arena of the wrong kind in its slot"),
    ("ARN-05", "a reserved field is not zero"),
    ("ARN-06", "a padding byte is not zero"),
    ("ARN-07", "the declared length differs from the bytes present, or part of the block is unused"),
    ("ARN-08", "the buffer does not start on an 8-byte boundary"),
    ("ARN-09", "the host is not little-endian, so payloads cannot be borrowed in place"),
    ("ARN-10", "an offset-table index or a record index at or past the end of its table"),
    ("ARN-11", "an entry whose start plus len passes the arena length, or overflows"),
    ("ARN-12", "an accessor used on an arena of another payload kind"),
    ("ARN-13", "the knot count is not control points plus degree plus one"),
    ("ARN-14", "weights that are missing, extra, or not one per control point (never padded)"),
    ("ARN-15", "control-point x and y columns of different lengths"),
    ("ARN-16", "the periodic flag is set; this profile has no wrapped periodic form"),
    ("ARN-17", "flag bits that are not defined, or a flag the record kind cannot have"),
    ("ARN-18", "a record field its kind does not use is set, or one it needs is empty"),
    ("ARN-19", "record ids that are not strictly ascending, or at or above the NP-18 limit"),
    ("ARN-20", "a segment that names a record that does not exist"),
    ("ARN-21", "a composite whose segment lists lead back to itself"),
    ("ARN-22", "composites nested deeper than NP-18 allows"),
    ("ARN-23", "trim values that are not two per segment"),
    ("ARN-24", "a numeric policy rule (NP-nn) refused a count or a value; the error carries it"),
    ("ARN-25", "a block with no records"),
    ("ARN-26", "(builder) the same entity id twice"),
    ("ARN-27", "(builder) a composite names an entity that was not pushed"),
    ("ARN-28", "(builder) the blocks would not fit 32-bit lengths"),
];

/// One offset-table entry: where a curve's payload starts in the arena and how many elements it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OffsetEntry {
    /// First element, from the start of the arena.
    pub start: u32,
    /// Number of elements.
    pub len: u32,
}

impl OffsetEntry {
    /// The element range, or `None` when `start + len` overflows or passes `arena_len`.
    pub fn range(self, arena_len: u32) -> Option<core::ops::Range<usize>> {
        let end = self.start.checked_add(self.len)?;
        (end <= arena_len).then_some(self.start as usize..end as usize)
    }
}

/// Observes reads, so a test can count them. Every `slice` call is one table read and one payload read.
pub trait ReadProbe {
    /// An offset-table entry was read.
    fn table_read(&self, kind: PayloadKind);
    /// An arena payload run was read.
    fn payload_read(&self, kind: PayloadKind);
}

/// Collects payloads of one kind and writes them as an arena block.
#[derive(Clone, Debug)]
pub struct ArenaBuilder {
    kind: PayloadKind,
    table: Vec<(u32, u32)>,
    /// The `f64` columns (one, or two for control points) or, for segments, empty.
    columns: Vec<Vec<f64>>,
    words: Vec<u32>,
    /// Payload bits to table index, so an identical payload is stored once. Only ever looked up, never
    /// iterated, so it cannot make the output depend on hash order.
    seen: HashMap<Vec<u64>, u32>,
}

impl ArenaBuilder {
    /// An empty builder for `kind`.
    pub fn new(kind: PayloadKind) -> ArenaBuilder {
        ArenaBuilder {
            kind,
            table: Vec::new(),
            columns: vec![Vec::new(); kind.f64_columns()],
            words: Vec::new(),
            seen: HashMap::new(),
        }
    }

    /// The kind.
    pub fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Distinct payloads added so far.
    pub fn entries(&self) -> usize {
        self.table.len()
    }

    fn wrong(&self) -> ArenaError {
        ArenaError::WrongPayload { held: self.kind }
    }

    /// Record an entry for `len` elements keyed by `key`, or return the entry that already holds them.
    fn entry_for(&mut self, key: Vec<u64>, len: usize) -> Result<Result<u32, u32>, ArenaError> {
        if len == 0 {
            return Err(ArenaError::Numeric(NumericError::Limit));
        }
        if let Some(&i) = self.seen.get(&key) {
            return Ok(Err(i));
        }
        let index = u32::try_from(self.table.len()).map_err(|_| ArenaError::TooLarge)?;
        if index == NO_ENTRY {
            return Err(ArenaError::TooLarge);
        }
        let start = match self.kind {
            PayloadKind::Segments => self.words.len(),
            _ => self.columns[0].len(),
        };
        let start = u32::try_from(start).map_err(|_| ArenaError::TooLarge)?;
        let len = u32::try_from(len).map_err(|_| ArenaError::TooLarge)?;
        start.checked_add(len).ok_or(ArenaError::TooLarge)?;
        self.table.push((start, len));
        self.seen.insert(key, index);
        Ok(Ok(index))
    }

    /// Add a one-column `f64` payload (knots, weights or trims) and return its table index. A payload
    /// equal bit for bit to one already added is not stored again: its index is returned.
    pub fn push_f64(&mut self, values: &[f64]) -> Result<u32, ArenaError> {
        if !self.kind.is_f64() || self.kind.f64_columns() != 1 {
            return Err(self.wrong());
        }
        let key: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
        match self.entry_for(key, values.len())? {
            Err(existing) => Ok(existing),
            Ok(index) => {
                self.columns[0].extend_from_slice(values);
                Ok(index)
            }
        }
    }

    /// Add a control-point payload: the `x` and `y` runs, of equal length.
    pub fn push_xy(&mut self, x: &[f64], y: &[f64]) -> Result<u32, ArenaError> {
        if self.kind != PayloadKind::ControlPoints {
            return Err(self.wrong());
        }
        if x.len() != y.len() {
            return Err(ArenaError::ControlPointColumns { x: x.len(), y: y.len() });
        }
        let key: Vec<u64> = x.iter().chain(y).map(|v| v.to_bits()).collect();
        match self.entry_for(key, x.len())? {
            Err(existing) => Ok(existing),
            Ok(index) => {
                self.columns[0].extend_from_slice(x);
                self.columns[1].extend_from_slice(y);
                Ok(index)
            }
        }
    }

    /// Add a segment list of record indices.
    pub fn push_u32(&mut self, values: &[u32]) -> Result<u32, ArenaError> {
        if self.kind != PayloadKind::Segments {
            return Err(self.wrong());
        }
        let key: Vec<u64> = values.iter().map(|&v| u64::from(v)).collect();
        match self.entry_for(key, values.len())? {
            Err(existing) => Ok(existing),
            Ok(index) => {
                self.words.extend_from_slice(values);
                Ok(index)
            }
        }
    }

    /// The block. An arena with no entries is valid: a file with no splines has an empty knots arena.
    pub fn finish(self) -> Result<AlignedBytes, ArenaError> {
        let elements =
            if self.kind == PayloadKind::Segments { self.words.len() } else { self.columns[0].len() };
        let layout = ArenaLayout::new(self.kind, self.table.len() as u64, elements as u64);
        let total = u32::try_from(layout.total).map_err(|_| ArenaError::TooLarge)?;
        let entries = u32::try_from(self.table.len()).map_err(|_| ArenaError::TooLarge)?;
        let elements = u32::try_from(elements).map_err(|_| ArenaError::TooLarge)?;
        let mut out: Vec<u8> = Vec::with_capacity(layout.total as usize);
        out.extend_from_slice(&ARENA_MAGIC);
        out.extend_from_slice(&ARENA_VERSION.to_le_bytes());
        out.extend_from_slice(&self.kind.id().to_le_bytes());
        out.extend_from_slice(&entries.to_le_bytes());
        out.extend_from_slice(&elements.to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
        out.resize(ARENA_HEADER_LEN, 0);
        for &(start, len) in &self.table {
            out.extend_from_slice(&start.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
        }
        for col in &self.columns {
            for v in col {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for w in &self.words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out.resize(layout.total as usize, 0);
        Ok(AlignedBytes::from_bytes(&out))
    }
}

/// Where the parts of an arena of `entries` table entries and `elements` elements per column lie.
/// Computed in `u64` from two `u32` counts, so nothing here overflows.
struct ArenaLayout {
    table: u64,
    payload: u64,
    /// End of the used payload bytes, where the zero padding of a segment arena starts.
    payload_end: u64,
    total: u64,
}

impl ArenaLayout {
    fn new(kind: PayloadKind, entries: u64, elements: u64) -> ArenaLayout {
        let table = ARENA_HEADER_LEN as u64;
        let payload = table + OFFSET_ENTRY_LEN as u64 * entries;
        let payload_end = if kind.is_f64() {
            payload + 8 * kind.f64_columns() as u64 * elements
        } else {
            payload + 4 * elements
        };
        ArenaLayout { table, payload, payload_end, total: payload_end.div_ceil(8) * 8 }
    }
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A parsed arena block: the offset table and the payload, both borrowed from the buffer.
#[derive(Clone, Copy)]
pub struct ArenaView<'a> {
    kind: PayloadKind,
    probe: Option<&'a dyn ReadProbe>,
    /// `start, len` pairs, two words per entry.
    table: &'a [u32],
    arena_len: u32,
    /// The `f64` columns: one for knots, weights and trims, two for control points, none for segments.
    columns: [&'a [f64]; 2],
    words: &'a [u32],
}

impl core::fmt::Debug for ArenaView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaView")
            .field("kind", &self.kind)
            .field("entries", &self.entries())
            .field("arena_len", &self.arena_len)
            .finish()
    }
}

impl<'a> ArenaView<'a> {
    /// Decode `bytes` in place. The buffer must start on an 8-byte boundary.
    ///
    /// The header, the counts and the padding are checked against the bytes present before a column is
    /// cut. The offset-table entries are checked one at a time, as they are used, so opening an arena
    /// costs nothing in the number of entries.
    pub fn parse(bytes: &'a [u8]) -> Result<ArenaView<'a>, ArenaError> {
        ArenaView::parse_probed(bytes, None)
    }

    /// As [`ArenaView::parse`], reporting every table and payload read to `probe`.
    pub fn parse_probed(
        bytes: &'a [u8],
        probe: Option<&'a dyn ReadProbe>,
    ) -> Result<ArenaView<'a>, ArenaError> {
        if cfg!(target_endian = "big") {
            return Err(ArenaError::HostEndianness);
        }
        if (bytes.as_ptr() as usize) % 8 != 0 {
            return Err(ArenaError::Misaligned);
        }
        if bytes.len() < ARENA_HEADER_LEN {
            return Err(ArenaError::Truncated { needed: ARENA_HEADER_LEN as u64, got: bytes.len() as u64 });
        }
        if bytes[..4] != ARENA_MAGIC {
            return Err(ArenaError::BadMagic);
        }
        let version = le_u16(bytes, 4);
        if version != ARENA_VERSION {
            return Err(ArenaError::UnsupportedVersion { found: version });
        }
        let kind_id = le_u16(bytes, 6);
        let kind = PayloadKind::from_id(kind_id).ok_or(ArenaError::UnknownKind { id: kind_id })?;
        if bytes[20..ARENA_HEADER_LEN].iter().any(|&b| b != 0) {
            return Err(ArenaError::NonZeroReserved);
        }
        let entries = le_u32(bytes, 8);
        let arena_len = le_u32(bytes, 12);
        let declared = le_u32(bytes, 16);
        if u64::from(declared) != bytes.len() as u64 {
            return Err(ArenaError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let layout = ArenaLayout::new(kind, u64::from(entries), u64::from(arena_len));
        if layout.total > bytes.len() as u64 {
            return Err(ArenaError::Truncated { needed: layout.total, got: bytes.len() as u64 });
        }
        if layout.total < bytes.len() as u64 {
            return Err(ArenaError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let at = |x: u64| x as usize;
        if bytes[at(layout.payload_end)..at(layout.total)].iter().any(|&b| b != 0) {
            return Err(ArenaError::NonZeroPadding);
        }
        let table = u32s(&bytes[at(layout.table)..at(layout.payload)]).ok_or(ArenaError::Misaligned)?;
        let mut columns: [&'a [f64]; 2] = [&[], &[]];
        let mut words: &'a [u32] = &[];
        let n = arena_len as usize;
        if kind.is_f64() {
            for (c, slot) in columns.iter_mut().enumerate().take(kind.f64_columns()) {
                let from = at(layout.payload) + c * 8 * n;
                *slot = f64s(&bytes[from..from + 8 * n]).ok_or(ArenaError::Misaligned)?;
            }
        } else {
            words =
                u32s(&bytes[at(layout.payload)..at(layout.payload) + 4 * n]).ok_or(ArenaError::Misaligned)?;
        }
        Ok(ArenaView { kind, probe, table, arena_len, columns, words })
    }

    /// The payload kind.
    pub fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Entries in the offset table.
    pub fn entries(&self) -> usize {
        self.table.len() / 2
    }

    /// Elements in each payload column.
    pub fn arena_len(&self) -> usize {
        self.arena_len as usize
    }

    /// Offset-table entry `index`, checked to lie inside the arena. One table read.
    pub fn entry(&self, index: u32) -> Result<OffsetEntry, ArenaError> {
        self.entry_range(index).map(|(e, _)| e)
    }

    fn entry_range(&self, index: u32) -> Result<(OffsetEntry, core::ops::Range<usize>), ArenaError> {
        let entries = self.table.len() / 2;
        if index as usize >= entries {
            return Err(ArenaError::EntryOutOfRange { index, entries: entries as u32 });
        }
        if let Some(p) = self.probe {
            p.table_read(self.kind);
        }
        let entry =
            OffsetEntry { start: self.table[2 * index as usize], len: self.table[2 * index as usize + 1] };
        let range = entry.range(self.arena_len).ok_or(ArenaError::RangeOutOfArena {
            start: entry.start,
            len: entry.len,
            arena_len: self.arena_len,
        })?;
        Ok((entry, range))
    }

    fn payload_read(&self) {
        if let Some(p) = self.probe {
            p.payload_read(self.kind);
        }
    }

    /// The `f64` payload of entry `index`, borrowed from the arena: one table read and one payload read.
    /// For knots, weights and trims.
    pub fn slice(&self, index: u32) -> Result<&'a [f64], ArenaError> {
        if !self.kind.is_f64() || self.kind.f64_columns() != 1 {
            return Err(ArenaError::WrongPayload { held: self.kind });
        }
        let (_, range) = self.entry_range(index)?;
        self.payload_read();
        Ok(&self.columns[0][range])
    }

    /// The `x` and `y` runs of control-point entry `index`: one table read and one payload read.
    pub fn slice_xy(&self, index: u32) -> Result<(&'a [f64], &'a [f64]), ArenaError> {
        if self.kind != PayloadKind::ControlPoints {
            return Err(ArenaError::WrongPayload { held: self.kind });
        }
        let (_, range) = self.entry_range(index)?;
        self.payload_read();
        Ok((&self.columns[0][range.clone()], &self.columns[1][range]))
    }

    /// The record indices of segment-list entry `index`: one table read and one payload read.
    pub fn slice_u32(&self, index: u32) -> Result<&'a [u32], ArenaError> {
        if self.kind != PayloadKind::Segments {
            return Err(ArenaError::WrongPayload { held: self.kind });
        }
        let (_, range) = self.entry_range(index)?;
        self.payload_read();
        Ok(&self.words[range])
    }
}
