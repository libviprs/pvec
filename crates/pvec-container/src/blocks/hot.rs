//! Hot type-batched immediate geometry blocks (issue P4.2). Red-phase stub: nothing is decoded yet.

use crate::blocks::aligned::AlignedBytes;
use crate::spec::FieldSpec;
use pvec_entity::EntityId;

/// The four magic bytes of a hot block.
pub const HOT_MAGIC: [u8; 4] = *b"PVH1";
/// The version this module reads and writes.
pub const HOT_VERSION: u16 = 1;
/// The fixed header of a hot block, in bytes.
pub const HOT_HEADER_LEN: usize = 32;

const fn field(
    name: &'static str,
    offset: usize,
    size: usize,
    encoding: &'static str,
    meaning: &'static str,
) -> FieldSpec {
    FieldSpec { name, offset, size, encoding, meaning }
}

/// The fixed header fields, for `docs/CONTAINER.md` chapter 2.
pub const HOT_HEADER_FIELDS: &[FieldSpec] = &[
    field("magic", 0, 4, "bytes", "`50 56 48 31` (`PVH1`)"),
    field("version", 4, 2, "u16", "1; a reader refuses any other"),
    field("kind", 6, 2, "u16", "geometry type, from the table in section 2.2"),
    field("entity_count", 8, 4, "u32", "entities in the block, 1 to 2^28 - 1"),
    field("vertex_count", 12, 4, "u32", "total vertices of a polyline block; zero for every other kind"),
    field("block_length", 16, 4, "u32", "length of the whole block in bytes, equal to the bytes present"),
    field("reserved", 20, 12, "bytes", "zero"),
];

/// The geometry type a block holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotKind {
    /// Line segments.
    Line,
    /// Polyline runs.
    Polyline,
    /// Circles and circular arcs.
    CircleArc,
    /// Ellipse arcs.
    EllipseArc,
}

impl HotKind {
    /// Every kind.
    pub const ALL: [HotKind; 4] = [HotKind::Line, HotKind::Polyline, HotKind::CircleArc, HotKind::EllipseArc];

    /// The id stored in the block header.
    pub fn id(self) -> u16 {
        match self {
            HotKind::Line => 1,
            HotKind::Polyline => 2,
            HotKind::CircleArc => 3,
            HotKind::EllipseArc => 4,
        }
    }

    /// The kind for an id.
    pub fn from_id(id: u16) -> Option<HotKind> {
        HotKind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// The names of the `f64` columns, in the order they are written.
    pub fn f64_columns(self) -> &'static [&'static str] {
        match self {
            HotKind::Line => &["x0", "y0", "x1", "y1"],
            HotKind::Polyline => &["x", "y"],
            HotKind::CircleArc => &["cx", "cy", "radius", "start", "sweep"],
            HotKind::EllipseArc => &["cx", "cy", "rx", "ry", "rotation", "start", "sweep"],
        }
    }
}

/// Why a block was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HotError {
    /// The block is not decoded yet.
    NotImplemented,
    /// The block ends before its structure does.
    Truncated {
        /// Bytes needed.
        needed: u64,
        /// Bytes supplied.
        got: u64,
    },
    /// The first four bytes are not the hot block magic.
    BadMagic,
    /// A version this reader does not implement.
    UnsupportedVersion {
        /// The version in the block.
        found: u16,
    },
    /// A kind id that is not in the table.
    UnknownKind {
        /// The id.
        id: u16,
    },
    /// A reserved header byte is not zero.
    NonZeroReserved,
    /// A padding byte is not zero.
    NonZeroPadding,
    /// The declared block length differs from the bytes present.
    LengthMismatch {
        /// The length in the header.
        declared: u32,
        /// The bytes supplied.
        actual: u64,
    },
    /// A block with no entities.
    EmptyBlock,
    /// More entities than NP-18 allows.
    TooManyEntities {
        /// The count in the header.
        count: u32,
    },
    /// More polyline vertices than a block may hold.
    TooManyVertices {
        /// The count in the header.
        count: u32,
    },
    /// An entity id at or above the NP-18 limit.
    EntityIdOutOfRange {
        /// The id.
        id: u32,
    },
    /// The ids are not strictly ascending at this position.
    IdsNotAscending {
        /// Position of the first id that is not above its predecessor.
        index: usize,
    },
    /// A run offset is smaller than the one before it.
    RunOffsetsNotMonotonic {
        /// Position of the offending offset.
        index: usize,
    },
    /// The first run offset is not zero, or the last is not the vertex count.
    RunOffsetsBadBounds,
    /// A polyline run with fewer than two vertices, or more than a run may hold.
    BadRunLength {
        /// The run.
        index: usize,
    },
    /// A coordinate that is NaN or infinite.
    NonFinite {
        /// The column.
        column: &'static str,
        /// The position in it.
        index: usize,
    },
    /// The buffer does not start on an 8-byte boundary, so the columns cannot be borrowed in place.
    Misaligned,
    /// The host is not little-endian, so a column cannot be borrowed in place.
    HostEndianness,
    /// An entity was pushed into a builder of another kind.
    WrongKind {
        /// The builder's kind.
        block: HotKind,
        /// The kind that was pushed.
        pushed: HotKind,
    },
    /// The same entity id was pushed twice.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// The block would not fit 32-bit lengths.
    TooLarge,
}

impl HotError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        "HOT-00"
    }
}

impl core::fmt::Display for HotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for HotError {}

/// Collects entities of one kind and writes them as columns.
#[derive(Clone, Debug)]
pub struct HotBlockBuilder {
    kind: HotKind,
}

impl HotBlockBuilder {
    /// An empty builder for `kind`.
    pub fn new(kind: HotKind) -> HotBlockBuilder {
        HotBlockBuilder { kind }
    }

    /// The kind.
    pub fn kind(&self) -> HotKind {
        self.kind
    }

    /// Entities pushed so far.
    pub fn len(&self) -> usize {
        0
    }

    /// Whether nothing was pushed.
    pub fn is_empty(&self) -> bool {
        true
    }

    /// Add a line segment from `a` to `b`.
    pub fn push_line(
        &mut self,
        _id: EntityId,
        _style: u32,
        _a: [f64; 2],
        _b: [f64; 2],
    ) -> Result<(), HotError> {
        Err(HotError::NotImplemented)
    }

    /// Add a polyline run.
    pub fn push_polyline(
        &mut self,
        _id: EntityId,
        _style: u32,
        _points: &[[f64; 2]],
    ) -> Result<(), HotError> {
        Err(HotError::NotImplemented)
    }

    /// Add a circle or circular arc; `start_sweep` is the start angle and the signed sweep, in radians.
    pub fn push_circle_arc(
        &mut self,
        _id: EntityId,
        _style: u32,
        _centre: [f64; 2],
        _radius: f64,
        _start_sweep: [f64; 2],
    ) -> Result<(), HotError> {
        Err(HotError::NotImplemented)
    }

    /// Add an ellipse arc; `angles` is the rotation of the major axis, the start angle and the signed sweep.
    pub fn push_ellipse_arc(
        &mut self,
        _id: EntityId,
        _style: u32,
        _centre: [f64; 2],
        _radii: [f64; 2],
        _angles: [f64; 3],
    ) -> Result<(), HotError> {
        Err(HotError::NotImplemented)
    }

    /// The block, entities ordered by `EntityId`.
    pub fn finish(self) -> Result<AlignedBytes, HotError> {
        Err(HotError::NotImplemented)
    }
}

/// A decoded block: borrowed columns, nothing owned.
#[derive(Clone, Copy, Debug)]
pub struct HotBlockView<'a> {
    kind: HotKind,
    empty: &'a [f64],
}

impl<'a> HotBlockView<'a> {
    /// Decode `bytes` in place.
    pub fn parse(_bytes: &'a [u8]) -> Result<HotBlockView<'a>, HotError> {
        Err(HotError::NotImplemented)
    }

    /// The kind.
    pub fn kind(&self) -> HotKind {
        self.kind
    }

    /// Number of entities.
    pub fn len(&self) -> usize {
        0
    }

    /// Whether the block has no entities (a parsed block never is).
    pub fn is_empty(&self) -> bool {
        true
    }

    /// The entity ids, ascending.
    pub fn entity_ids(&self) -> &'a [u32] {
        &[]
    }

    /// One style reference per entity.
    pub fn styles(&self) -> &'a [u32] {
        &[]
    }

    /// The run offsets of a polyline block (`len() + 1` of them); empty for the other kinds.
    pub fn run_offsets(&self) -> &'a [u32] {
        &[]
    }

    /// The `f64` columns in written order.
    pub fn f64_columns(&self) -> &[&'a [f64]] {
        core::slice::from_ref(&self.empty)
    }

    /// An `f64` column by name.
    pub fn column(&self, _name: &str) -> Option<&'a [f64]> {
        None
    }

    /// The x and y slices of the polyline at `index`.
    pub fn polyline(&self, _index: usize) -> Option<(&'a [f64], &'a [f64])> {
        None
    }
}
