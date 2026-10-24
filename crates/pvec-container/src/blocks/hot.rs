//! Hot type-batched immediate geometry blocks (issue P4.2).
//!
//! One block holds one geometry type as parallel columns (structure of arrays, layout B of ADR 0004):
//! `u32` entity ids, one `u32` style reference per entity, a run-offset column for polylines, and one
//! `f64` column per coordinate. [`HotBlockView`] borrows every column out of the block bytes, so
//! reading a viewport is a bounded set of slices and nothing is copied or allocated per entity.
//! The byte layout is chapter 2 of `docs/CONTAINER.md`, rendered from the constants here.

use crate::blocks::aligned::{AlignedBytes, f64s, u32s};
use crate::spec::FieldSpec;
use pvec_entity::EntityId;
use pvec_numeric::MAX_ENTITIES;

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

    /// The name used in the document.
    pub fn name(self) -> &'static str {
        match self {
            HotKind::Line => "Line",
            HotKind::Polyline => "Polyline",
            HotKind::CircleArc => "CircleArc",
            HotKind::EllipseArc => "EllipseArc",
        }
    }

    /// What the `f64` columns hold, for the document.
    fn meaning(self) -> &'static str {
        match self {
            HotKind::Line => "start point (`x0`, `y0`) and end point (`x1`, `y1`), one entry per segment",
            HotKind::Polyline => {
                "all x, then all y, concatenated run by run; run `i` is `offsets[i]..offsets[i+1]`"
            }
            HotKind::CircleArc => {
                "centre, radius, start angle and signed sweep in radians; a full circle sweeps 2 pi"
            }
            HotKind::EllipseArc => {
                "centre, the two semi-axes, the rotation of the first axis, start angle and signed sweep in radians"
            }
        }
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
    /// A reserved header field is not zero.
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
    /// A block with no entities.
    EmptyBlock,
    /// More entities than NP-18 allows.
    TooManyEntities {
        /// The count in the header.
        count: u32,
    },
    /// More polyline vertices than the entity count allows, or vertices in a block that has no runs.
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
    /// The rule identifier, `HOT-nn`, as listed in chapter 2 of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            HotError::Truncated { .. } => "HOT-01",
            HotError::BadMagic => "HOT-02",
            HotError::UnsupportedVersion { .. } => "HOT-03",
            HotError::UnknownKind { .. } => "HOT-04",
            HotError::NonZeroReserved => "HOT-05",
            HotError::NonZeroPadding => "HOT-06",
            HotError::LengthMismatch { .. } => "HOT-07",
            HotError::EmptyBlock => "HOT-08",
            HotError::TooManyEntities { .. } => "HOT-09",
            HotError::TooManyVertices { .. } => "HOT-10",
            HotError::EntityIdOutOfRange { .. } => "HOT-11",
            HotError::IdsNotAscending { .. } | HotError::DuplicateId { .. } => "HOT-12",
            HotError::RunOffsetsNotMonotonic { .. } => "HOT-13",
            HotError::RunOffsetsBadBounds => "HOT-14",
            HotError::BadRunLength { .. } => "HOT-15",
            HotError::NonFinite { .. } => "HOT-16",
            HotError::Misaligned => "HOT-17",
            HotError::HostEndianness => "HOT-18",
            HotError::WrongKind { .. } => "HOT-19",
            HotError::TooLarge => "HOT-20",
        }
    }
}

impl core::fmt::Display for HotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {self:?}", self.rule())
    }
}

impl std::error::Error for HotError {}

/// The rules a reader refuses on, for the document: identifier and meaning.
pub const HOT_RULES: &[(&str, &str)] = &[
    ("HOT-01", "the block is shorter than its structure requires"),
    ("HOT-02", "bad magic"),
    ("HOT-03", "unsupported version"),
    ("HOT-04", "unknown kind id"),
    ("HOT-05", "a reserved header field is not zero"),
    ("HOT-06", "a padding byte is not zero"),
    ("HOT-07", "the declared length differs from the bytes present, or part of the block is unused"),
    ("HOT-08", "no entities"),
    ("HOT-09", "more entities than NP-18 allows"),
    (
        "HOT-10",
        "a vertex count that the entity count cannot account for, or vertices in a block with no runs",
    ),
    ("HOT-11", "an entity id at or above the NP-18 limit"),
    ("HOT-12", "ids that are not strictly ascending (the builder reports a repeated id here too)"),
    ("HOT-13", "run offsets that decrease"),
    ("HOT-14", "run offsets that do not start at zero or do not end at the vertex count"),
    ("HOT-15", "a polyline run of fewer than 2 or more than 2^20 vertices"),
    ("HOT-16", "a NaN or infinite coordinate (NP-01)"),
    ("HOT-17", "the buffer does not start on an 8-byte boundary"),
    ("HOT-18", "the host is not little-endian, so columns cannot be borrowed in place"),
    ("HOT-19", "(builder) an entity of another kind than the block"),
    ("HOT-20", "(builder) the block would not fit 32-bit lengths"),
];

/// The most vertices one polyline run holds.
pub const MAX_RUN_VERTICES: u32 = 1 << 20;

/// Size of an aligned column element and of the first `f64` column's alignment.
const F64: usize = 8;
const U32: usize = 4;

fn align8(x: u64) -> u64 {
    x.div_ceil(8) * 8
}

/// Where the columns of a block of `n` entities and `m` polyline vertices lie. All values are bytes
/// from the first byte of the block. Computed in `u64` from counts already bounded (`n` below 2^28 and
/// `m` below 2^32), so nothing here overflows.
struct Layout {
    ids: u64,
    styles: u64,
    offsets: u64,
    /// End of the `u32` columns, where the zero padding starts.
    u32_end: u64,
    /// Where the first `f64` column starts, a multiple of 8.
    f64_start: u64,
    /// Elements in each `f64` column.
    f64_len: u64,
    total: u64,
}

impl Layout {
    fn new(kind: HotKind, n: u64, m: u64) -> Layout {
        let ids = HOT_HEADER_LEN as u64;
        let styles = ids + U32 as u64 * n;
        let offsets = styles + U32 as u64 * n;
        let u32_end = offsets + if kind == HotKind::Polyline { U32 as u64 * (n + 1) } else { 0 };
        let f64_start = align8(u32_end);
        let f64_len = if kind == HotKind::Polyline { m } else { n };
        let total = f64_start + F64 as u64 * kind.f64_columns().len() as u64 * f64_len;
        Layout { ids, styles, offsets, u32_end, f64_start, f64_len, total }
    }
}

/// Collects entities of one kind and writes them as columns, ordered by `EntityId`.
#[derive(Clone, Debug)]
pub struct HotBlockBuilder {
    kind: HotKind,
    ids: Vec<u32>,
    styles: Vec<u32>,
    /// Vertex index where each polyline run starts, plus the total at the end; empty for other kinds.
    runs: Vec<u32>,
    columns: Vec<Vec<f64>>,
}

impl HotBlockBuilder {
    /// An empty builder for `kind`.
    pub fn new(kind: HotKind) -> HotBlockBuilder {
        let runs = if kind == HotKind::Polyline { vec![0] } else { Vec::new() };
        HotBlockBuilder {
            kind,
            ids: Vec::new(),
            styles: Vec::new(),
            runs,
            columns: vec![Vec::new(); kind.f64_columns().len()],
        }
    }

    /// The kind.
    pub fn kind(&self) -> HotKind {
        self.kind
    }

    /// Entities pushed so far.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether nothing was pushed.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    fn check_kind(&self, pushed: HotKind) -> Result<(), HotError> {
        if self.kind == pushed { Ok(()) } else { Err(HotError::WrongKind { block: self.kind, pushed }) }
    }

    fn check_finite(&self, values: &[f64]) -> Result<(), HotError> {
        for (i, v) in values.iter().enumerate() {
            if !v.is_finite() {
                let column = self.kind.f64_columns()[i % self.kind.f64_columns().len()];
                return Err(HotError::NonFinite { column, index: self.ids.len() });
            }
        }
        Ok(())
    }

    fn push_row(&mut self, id: EntityId, style: u32, values: &[f64]) -> Result<(), HotError> {
        self.check_finite(values)?;
        self.ids.push(id.index());
        self.styles.push(style);
        for (col, v) in self.columns.iter_mut().zip(values) {
            col.push(*v);
        }
        Ok(())
    }

    /// Add a line segment from `a` to `b`.
    pub fn push_line(&mut self, id: EntityId, style: u32, a: [f64; 2], b: [f64; 2]) -> Result<(), HotError> {
        self.check_kind(HotKind::Line)?;
        self.push_row(id, style, &[a[0], a[1], b[0], b[1]])
    }

    /// Add a polyline run of 2 to 2^20 vertices.
    pub fn push_polyline(&mut self, id: EntityId, style: u32, points: &[[f64; 2]]) -> Result<(), HotError> {
        self.check_kind(HotKind::Polyline)?;
        if points.len() < 2 || points.len() > MAX_RUN_VERTICES as usize {
            return Err(HotError::BadRunLength { index: self.ids.len() });
        }
        for p in points {
            if !p[0].is_finite() || !p[1].is_finite() {
                let column = if p[0].is_finite() { "y" } else { "x" };
                return Err(HotError::NonFinite { column, index: self.ids.len() });
            }
        }
        let end = u64::from(*self.runs.last().unwrap_or(&0)) + points.len() as u64;
        let end = u32::try_from(end).map_err(|_| HotError::TooLarge)?;
        self.ids.push(id.index());
        self.styles.push(style);
        for p in points {
            self.columns[0].push(p[0]);
            self.columns[1].push(p[1]);
        }
        self.runs.push(end);
        Ok(())
    }

    /// Add a circle or circular arc; `start_sweep` is the start angle and the signed sweep, in radians.
    pub fn push_circle_arc(
        &mut self,
        id: EntityId,
        style: u32,
        centre: [f64; 2],
        radius: f64,
        start_sweep: [f64; 2],
    ) -> Result<(), HotError> {
        self.check_kind(HotKind::CircleArc)?;
        self.push_row(id, style, &[centre[0], centre[1], radius, start_sweep[0], start_sweep[1]])
    }

    /// Add an ellipse arc; `angles` is the rotation of the major axis, the start angle and the signed sweep.
    pub fn push_ellipse_arc(
        &mut self,
        id: EntityId,
        style: u32,
        centre: [f64; 2],
        radii: [f64; 2],
        angles: [f64; 3],
    ) -> Result<(), HotError> {
        self.check_kind(HotKind::EllipseArc)?;
        self.push_row(id, style, &[centre[0], centre[1], radii[0], radii[1], angles[0], angles[1], angles[2]])
    }

    /// The block, entities ordered by `EntityId` so the same set of entities always gives the same bytes.
    pub fn finish(self) -> Result<AlignedBytes, HotError> {
        self.finish_ordered(true)
    }

    /// The block with the entities in the order they were pushed, which breaks the canonical order.
    ///
    /// The writer's mutation check uses it to show that its own sort by `EntityId` is load-bearing;
    /// nothing else should.
    #[doc(hidden)]
    pub fn finish_unsorted(self) -> Result<AlignedBytes, HotError> {
        self.finish_ordered(false)
    }

    fn finish_ordered(self, sorted: bool) -> Result<AlignedBytes, HotError> {
        let n = self.ids.len();
        if n == 0 {
            return Err(HotError::EmptyBlock);
        }
        if n >= MAX_ENTITIES {
            return Err(HotError::TooManyEntities { count: u32::try_from(n).unwrap_or(u32::MAX) });
        }
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_unstable_by_key(|&i| self.ids[i]);
        for w in order.windows(2) {
            if self.ids[w[0]] == self.ids[w[1]] {
                return Err(HotError::DuplicateId { id: self.ids[w[0]] });
            }
        }
        if !sorted {
            order = (0..n).collect();
        }
        let is_poly = self.kind == HotKind::Polyline;
        let m = if is_poly { self.columns[0].len() as u64 } else { 0 };
        let layout = Layout::new(self.kind, n as u64, m);
        let total = u32::try_from(layout.total).map_err(|_| HotError::TooLarge)?;

        let mut out: Vec<u8> = Vec::with_capacity(layout.total as usize);
        out.extend_from_slice(&HOT_MAGIC);
        out.extend_from_slice(&HOT_VERSION.to_le_bytes());
        out.extend_from_slice(&self.kind.id().to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&(m as u32).to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
        out.resize(HOT_HEADER_LEN, 0);
        for &i in &order {
            out.extend_from_slice(&self.ids[i].to_le_bytes());
        }
        for &i in &order {
            out.extend_from_slice(&self.styles[i].to_le_bytes());
        }
        if is_poly {
            let mut at = 0u32;
            out.extend_from_slice(&at.to_le_bytes());
            for &i in &order {
                at += self.runs[i + 1] - self.runs[i];
                out.extend_from_slice(&at.to_le_bytes());
            }
        }
        out.resize(layout.f64_start as usize, 0);
        if is_poly {
            for col in &self.columns {
                for &i in &order {
                    let (lo, hi) = (self.runs[i] as usize, self.runs[i + 1] as usize);
                    for v in &col[lo..hi] {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        } else {
            for col in &self.columns {
                for &i in &order {
                    out.extend_from_slice(&col[i].to_le_bytes());
                }
            }
        }
        debug_assert_eq!(out.len() as u64, layout.total);
        Ok(AlignedBytes::from_bytes(&out))
    }
}

/// A decoded block: borrowed columns, nothing owned.
///
/// Every slice points into the buffer the block was parsed from. The view is `Copy`, and copying it
/// copies a handful of fat pointers, never any of the data.
#[derive(Clone, Copy, Debug)]
pub struct HotBlockView<'a> {
    kind: HotKind,
    ids: &'a [u32],
    styles: &'a [u32],
    offsets: &'a [u32],
    columns: [&'a [f64]; 7],
    column_count: usize,
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl<'a> HotBlockView<'a> {
    /// Decode `bytes` in place. The buffer must start on an 8-byte boundary (an [`AlignedBytes`] does).
    ///
    /// Every count is checked against the bytes present before a column is sliced, nothing is
    /// allocated, and a block that fails any rule is refused whole.
    pub fn parse(bytes: &'a [u8]) -> Result<HotBlockView<'a>, HotError> {
        if cfg!(target_endian = "big") {
            return Err(HotError::HostEndianness);
        }
        if (bytes.as_ptr() as usize) % F64 != 0 {
            return Err(HotError::Misaligned);
        }
        if bytes.len() < HOT_HEADER_LEN {
            return Err(HotError::Truncated { needed: HOT_HEADER_LEN as u64, got: bytes.len() as u64 });
        }
        if bytes[..4] != HOT_MAGIC {
            return Err(HotError::BadMagic);
        }
        let version = le_u16(bytes, 4);
        if version != HOT_VERSION {
            return Err(HotError::UnsupportedVersion { found: version });
        }
        let kind_id = le_u16(bytes, 6);
        let kind = HotKind::from_id(kind_id).ok_or(HotError::UnknownKind { id: kind_id })?;
        if bytes[20..HOT_HEADER_LEN].iter().any(|&b| b != 0) {
            return Err(HotError::NonZeroReserved);
        }
        let count = le_u32(bytes, 8);
        let vertices = le_u32(bytes, 12);
        let declared = le_u32(bytes, 16);
        if count == 0 {
            return Err(HotError::EmptyBlock);
        }
        if count as usize >= MAX_ENTITIES {
            return Err(HotError::TooManyEntities { count });
        }
        let n = u64::from(count);
        let is_poly = kind == HotKind::Polyline;
        let vertex_limit = if is_poly { n * u64::from(MAX_RUN_VERTICES) } else { 0 };
        if u64::from(vertices) > vertex_limit {
            return Err(HotError::TooManyVertices { count: vertices });
        }
        if u64::from(declared) != bytes.len() as u64 {
            return Err(HotError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let layout = Layout::new(kind, n, u64::from(vertices));
        if layout.total > bytes.len() as u64 {
            return Err(HotError::Truncated { needed: layout.total, got: bytes.len() as u64 });
        }
        if layout.total < bytes.len() as u64 {
            return Err(HotError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let n = count as usize;
        let at = |x: u64| x as usize;
        let u32_col =
            |from: u64, len: usize| u32s(&bytes[at(from)..at(from) + 4 * len]).ok_or(HotError::Misaligned);
        let ids = u32_col(layout.ids, n)?;
        let styles = u32_col(layout.styles, n)?;
        let offsets = if is_poly { u32_col(layout.offsets, n + 1)? } else { &[][..] };
        if bytes[at(layout.u32_end)..at(layout.f64_start)].iter().any(|&b| b != 0) {
            return Err(HotError::NonZeroPadding);
        }

        for (i, &id) in ids.iter().enumerate() {
            if id as usize >= MAX_ENTITIES {
                return Err(HotError::EntityIdOutOfRange { id });
            }
            if i > 0 && id <= ids[i - 1] {
                return Err(HotError::IdsNotAscending { index: i });
            }
        }
        if is_poly {
            if offsets[0] != 0 {
                return Err(HotError::RunOffsetsBadBounds);
            }
            for i in 1..offsets.len() {
                if offsets[i] < offsets[i - 1] {
                    return Err(HotError::RunOffsetsNotMonotonic { index: i });
                }
            }
            if offsets[n] != vertices {
                return Err(HotError::RunOffsetsBadBounds);
            }
            for (i, w) in offsets.windows(2).enumerate() {
                let len = w[1] - w[0];
                if !(2..=MAX_RUN_VERTICES).contains(&len) {
                    return Err(HotError::BadRunLength { index: i });
                }
            }
        }

        let names = kind.f64_columns();
        let mut columns: [&'a [f64]; 7] = [&[]; 7];
        let len = at(layout.f64_len);
        for (c, slot) in columns.iter_mut().enumerate().take(names.len()) {
            let from = at(layout.f64_start) + c * F64 * len;
            let col = f64s(&bytes[from..from + F64 * len]).ok_or(HotError::Misaligned)?;
            if let Some(index) = col.iter().position(|v| !v.is_finite()) {
                return Err(HotError::NonFinite { column: names[c], index });
            }
            *slot = col;
        }
        Ok(HotBlockView { kind, ids, styles, offsets, columns, column_count: names.len() })
    }

    /// The kind.
    pub fn kind(&self) -> HotKind {
        self.kind
    }

    /// Number of entities.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Whether the block has no entities (a parsed block never is).
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The entity ids, strictly ascending.
    pub fn entity_ids(&self) -> &'a [u32] {
        self.ids
    }

    /// One style reference per entity: an opaque index the style component stream resolves.
    pub fn styles(&self) -> &'a [u32] {
        self.styles
    }

    /// The run offsets of a polyline block (`len() + 1` of them); empty for the other kinds.
    pub fn run_offsets(&self) -> &'a [u32] {
        self.offsets
    }

    /// The `f64` columns in written order, named by [`HotKind::f64_columns`].
    pub fn f64_columns(&self) -> &[&'a [f64]] {
        &self.columns[..self.column_count]
    }

    /// An `f64` column by name.
    pub fn column(&self, name: &str) -> Option<&'a [f64]> {
        let at = self.kind.f64_columns().iter().position(|c| *c == name)?;
        Some(self.columns[at])
    }

    /// The x and y slices of the polyline at `index`, in entity order.
    pub fn polyline(&self, index: usize) -> Option<(&'a [f64], &'a [f64])> {
        if self.kind != HotKind::Polyline {
            return None;
        }
        let lo = *self.offsets.get(index)? as usize;
        let hi = *self.offsets.get(index + 1)? as usize;
        Some((self.columns[0].get(lo..hi)?, self.columns[1].get(lo..hi)?))
    }
}

/// Chapter 2 of `docs/CONTAINER.md`, rendered from the constants in this module.
pub fn render_chapter() -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "## 2. Hot geometry blocks\n\n\
Simple geometry that a viewport reads over and over lives in hot blocks: one block per geometry type, \
holding parallel columns (structure of arrays, layout B of ADR 0004) and not one serialised record per \
entity. A reader decodes a block by slicing its columns out of the bytes, so a viewport costs a bounded \
number of column copies and no per-entity pointer chase or branch. Which spatial partition a block \
belongs to is decided by the spatial index (P5.1, P5.2), not here.\n\n\
### 2.1 Block header\n\n\
The header is {HOT_HEADER_LEN} bytes, little-endian, and the first byte of the block must be 8-aligned in \
memory (the reader refuses a buffer that is not, rather than copying it). Offsets are from the first byte \
of the block.\n\n"
    ));
    s.push_str(&crate::spec::table(HOT_HEADER_FIELDS));
    s.push_str(
        "\n### 2.2 Kinds and columns\n\n\
Coordinates are `f64` end to end: there is no quantisation or rounding in this layer, and a value comes \
back with the bits it went in with. They are finite (`NP-01`); a NaN or an infinity refuses the block. \
These are the immediate display forms (a line is its two end points); the exact curve parameters of \
ADR 0011 stay in the codec sections of P3.2.\n\n\
| id | kind | `f64` columns, in order | meaning |\n|---|---|---|---|\n",
    );
    for k in HotKind::ALL {
        s.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            k.id(),
            k.name(),
            k.f64_columns().iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", "),
            k.meaning()
        ));
    }
    s.push_str(&format!(
        "\n### 2.3 Column order and alignment\n\n\
After the header the columns follow in this order: the entity ids (`u32`, `n` of them), the style \
references (`u32`, `n`), for a polyline block the run offsets (`u32`, `n + 1`), zero padding up to a \
multiple of 8, then the `f64` columns of the kind, each `n` long (a polyline block: the vertex count) with \
no padding between them. Every column starts at its natural alignment: a `u32` column on a multiple of 4 \
and an `f64` column on a multiple of 8, from the first byte of the block. Because the block itself \
starts on 8, a decoded block is directly loadable, including in WASM linear memory, where a `Float64Array` \
over the memory throws on a byte offset that is not a multiple of 8. The only padding a block can contain is the gap before \
the first `f64` column (0 or 4 bytes), the block is a multiple of 8 bytes long, and the header's \
`block_length` is that length.\n\n\
Padding is zero and a reader refuses a block where it is not, which keeps the encoding canonical: \
decoding and re-encoding gives the same bytes.\n\n\
### 2.4 Order, ids and styles\n\n\
Entities are stored in strictly ascending `EntityId` order (P3.6), so the same set of entities always \
encodes to the same bytes whatever order the writer saw them in (P4.4). An id is below `2^28` (NP-18) \
and appears once. The style reference is an opaque `u32` per entity that the style component stream \
(P3.7) resolves; this layer stores it and does not interpret it. ADR 0011 left references out of the \
geometry codec because the component streams point at entities by id, but a viewport that draws from \
these blocks needs the style next to the coordinates, and four bytes per entity is the price.\n\n\
### 2.5 Polyline runs\n\n\
The run offsets index the `x` and `y` columns. The first is 0, they never decrease, the last is the \
vertex count in the header, and every run has between 2 and {MAX_RUN_VERTICES} vertices. A block whose run \
offsets break any of these is refused whole and never read. The vertex count is at most \
`entity_count * {MAX_RUN_VERTICES}`, so a lying count cannot size anything before the bytes are checked.\n\n\
### 2.6 Refusals\n\n\
Each refusal is a typed `HotError` with a rule identifier. Every count is compared with the bytes left \
before it is used, and decoding allocates nothing.\n\n\
| rule | meaning |\n|---|---|\n",
    ));
    for (rule, meaning) in HOT_RULES {
        s.push_str(&format!("| {rule} | {meaning} |\n"));
    }
    s
}
