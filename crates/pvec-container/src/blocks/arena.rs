//! Arena blocks for variable-length advanced geometry (issue P4.3).
//!
//! One arena block holds one payload kind (knots, weights, control points, composite segment lists,
//! trim domains) for every curve in a file. A curve does not carry its numbers: it carries an index
//! into the block's offset table, and the table entry is `(start, len)` into the arena, so a curve's
//! payload is located by two integer reads and is one contiguous run. The layout is chapter 3 of
//! `docs/CONTAINER.md`, rendered from the constants here and in [`crate::blocks::advanced`].

use crate::blocks::aligned::AlignedBytes;
use crate::spec::FieldSpec;
use pvec_numeric::NumericError;

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
pub const ARENA_HEADER_FIELDS: &[FieldSpec] = &[];

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
        0
    }

    /// The kind for an id.
    pub fn from_id(_id: u16) -> Option<PayloadKind> {
        None
    }

    /// The name used in the document.
    pub fn name(self) -> &'static str {
        ""
    }

    /// The names of the payload columns, in the order they are written.
    pub fn columns(self) -> &'static [&'static str] {
        &[]
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
    /// A segment that names a record that does not exist, or the record itself.
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
        "ARN-00"
    }
}

impl core::fmt::Display for ArenaError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {self:?}", self.rule())
    }
}

impl std::error::Error for ArenaError {}

/// The rules a reader refuses on, for the document: identifier and meaning.
pub const ARENA_RULES: &[(&str, &str)] = &[];

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
    pub fn range(self, _arena_len: u32) -> Option<core::ops::Range<usize>> {
        None
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
#[allow(dead_code)]
pub struct ArenaBuilder {
    kind: PayloadKind,
}

impl ArenaBuilder {
    /// An empty builder for `kind`.
    pub fn new(kind: PayloadKind) -> ArenaBuilder {
        ArenaBuilder { kind }
    }

    /// The kind.
    pub fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Add a one-column `f64` payload (knots, weights or trims) and return its table index. A payload
    /// equal bit for bit to one already added is not stored again: its index is returned.
    pub fn push_f64(&mut self, _values: &[f64]) -> Result<u32, ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// Add a control-point payload: the `x` and `y` runs, of equal length.
    pub fn push_xy(&mut self, _x: &[f64], _y: &[f64]) -> Result<u32, ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// Add a segment list of record indices.
    pub fn push_u32(&mut self, _values: &[u32]) -> Result<u32, ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// The block.
    pub fn finish(self) -> Result<AlignedBytes, ArenaError> {
        Err(ArenaError::TooLarge)
    }
}

/// A parsed arena block: the offset table and the payload, both borrowed from the buffer.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct ArenaView<'a> {
    kind: PayloadKind,
    probe: Option<&'a dyn ReadProbe>,
    _bytes: &'a [u8],
}

impl core::fmt::Debug for ArenaView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ArenaView").field("kind", &self.kind).finish()
    }
}

impl<'a> ArenaView<'a> {
    /// Decode `bytes` in place. The buffer must start on an 8-byte boundary.
    pub fn parse(bytes: &'a [u8]) -> Result<ArenaView<'a>, ArenaError> {
        ArenaView::parse_probed(bytes, None)
    }

    /// As [`ArenaView::parse`], reporting every table and payload read to `probe`.
    pub fn parse_probed(
        bytes: &'a [u8],
        probe: Option<&'a dyn ReadProbe>,
    ) -> Result<ArenaView<'a>, ArenaError> {
        let _ = (bytes, probe);
        Err(ArenaError::BadMagic)
    }

    /// The payload kind.
    pub fn kind(&self) -> PayloadKind {
        self.kind
    }

    /// Entries in the offset table.
    pub fn entries(&self) -> usize {
        0
    }

    /// Elements in each payload column.
    pub fn arena_len(&self) -> usize {
        0
    }

    /// Offset-table entry `index`, checked to lie inside the arena. One table read.
    pub fn entry(&self, _index: u32) -> Result<OffsetEntry, ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// The `f64` payload of entry `index`, borrowed from the arena: one table read and one payload read.
    /// For knots, weights and trims.
    pub fn slice(&self, _index: u32) -> Result<&'a [f64], ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// The `x` and `y` runs of control-point entry `index`: one table read and one payload read.
    pub fn slice_xy(&self, _index: u32) -> Result<(&'a [f64], &'a [f64]), ArenaError> {
        Err(ArenaError::TooLarge)
    }

    /// The record indices of segment-list entry `index`: one table read and one payload read.
    pub fn slice_u32(&self, _index: u32) -> Result<&'a [u32], ArenaError> {
        Err(ArenaError::TooLarge)
    }
}
