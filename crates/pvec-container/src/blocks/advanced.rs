//! Advanced curve records and the six blocks that hold them (issue P4.3).
//!
//! An [`AdvancedCurveRecord`] is a fixed 32-byte row: degree, flags and one offset-table index per
//! arena, and nothing else, so there is no per-entity byte blob. The numbers live in the arenas of
//! [`crate::blocks::arena`]. A composite curve is a record whose segment list is an index list naming
//! other records, so a segment two composites share is stored once.

use crate::blocks::aligned::{AlignedBytes, u32s};
use crate::blocks::arena::{
    ARENA_HEADER_FIELDS, ARENA_HEADER_LEN, ARENA_RULES, ArenaBuilder, ArenaError, ArenaView, NO_ENTRY,
    OFFSET_ENTRY_LEN, PayloadKind, ReadProbe,
};
use crate::spec::FieldSpec;
use pvec_entity::EntityId;
use pvec_numeric::{
    MAX_COMPOSITE_SEGMENTS, MAX_DEGREE, MAX_ENTITIES, MAX_NESTING_DEPTH, NumericError, Policy, check_value,
};
use std::collections::{HashMap, HashSet};

const fn field(
    name: &'static str,
    offset: usize,
    size: usize,
    encoding: &'static str,
    meaning: &'static str,
) -> FieldSpec {
    FieldSpec { name, offset, size, encoding, meaning }
}

/// The four magic bytes of a record block.
pub const RECORD_MAGIC: [u8; 4] = *b"PVR1";
/// The version this module reads and writes.
pub const RECORD_VERSION: u16 = 1;
/// The fixed header of a record block, in bytes.
pub const RECORD_HEADER_LEN: usize = 32;
/// Bytes of one record.
pub const RECORD_LEN: usize = 32;

/// Flag bit: the curve is rational (it has a weight array).
pub const FLAG_RATIONAL: u16 = 1;
/// Flag bit: the curve ends where it starts (NP-20).
pub const FLAG_CLOSED: u16 = 2;
/// Flag bit: periodic. Defined, and refused by this version of the profile.
pub const FLAG_PERIODIC: u16 = 4;

/// The record block header fields, for `docs/CONTAINER.md` chapter 3.
pub const RECORD_HEADER_FIELDS: &[FieldSpec] = &[
    field("magic", 0, 4, "bytes", "`50 56 52 31` (`PVR1`)"),
    field("version", 4, 2, "u16", "1; a reader refuses any other"),
    field("reserved_a", 6, 2, "u16", "zero"),
    field("record_count", 8, 4, "u32", "records in the block, 1 to 2^28 - 1"),
    field("block_length", 12, 4, "u32", "length of the whole block in bytes, equal to the bytes present"),
    field("reserved_b", 16, 16, "bytes", "zero"),
];
/// The record fields, for `docs/CONTAINER.md` chapter 3.
pub const RECORD_FIELDS: &[FieldSpec] = &[
    field("id", 0, 4, "u32", "entity id, below 2^28; records are in strictly ascending id order"),
    field("style", 4, 4, "u32", "opaque style reference, as in the hot blocks"),
    field("kind", 8, 1, "u8", "1 spline, 2 composite"),
    field("degree", 9, 1, "u8", "1 to 16 for a spline, 0 for a composite"),
    field("flags", 10, 2, "u16", "bit 0 rational, bit 1 closed, bit 2 periodic (refused); the rest zero"),
    field("knots", 12, 4, "u32", "index into the knots offset table; `ffffffff` for a composite"),
    field("weights", 16, 4, "u32", "index into the weights offset table; `ffffffff` unless rational"),
    field(
        "control_points",
        20,
        4,
        "u32",
        "index into the control-points offset table; `ffffffff` for a composite",
    ),
    field("segments", 24, 4, "u32", "index into the segments offset table; `ffffffff` for a spline"),
    field("trims", 28, 4, "u32", "index into the trims offset table; `ffffffff` for no trims"),
];

/// What a record is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    /// A B-spline, rational when [`FLAG_RATIONAL`] is set.
    Spline,
    /// A composite curve: a list of other records.
    Composite,
}

/// One advanced curve: degree, flags and arena indices. [`NO_ENTRY`](crate::blocks::arena::NO_ENTRY)
/// in an index means the curve has none of that payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdvancedCurveRecord {
    /// The entity id.
    pub id: u32,
    /// Opaque style reference, as in the hot blocks.
    pub style: u32,
    /// Spline or composite.
    pub kind: RecordKind,
    /// Degree of a spline (1 to 16); zero for a composite.
    pub degree: u8,
    /// [`FLAG_RATIONAL`], [`FLAG_CLOSED`], [`FLAG_PERIODIC`].
    pub flags: u16,
    /// Index into the knots offset table.
    pub knots: u32,
    /// Index into the weights offset table.
    pub weights: u32,
    /// Index into the control-points offset table.
    pub control_points: u32,
    /// Index into the segments offset table.
    pub segments: u32,
    /// Index into the trims offset table.
    pub trims: u32,
}

/// A B-spline to write.
#[derive(Clone, Copy, Debug)]
pub struct SplineSpec<'a> {
    /// Degree, 1 to 16.
    pub degree: usize,
    /// The knot vector.
    pub knots: &'a [f64],
    /// Control point x values.
    pub x: &'a [f64],
    /// Control point y values.
    pub y: &'a [f64],
    /// Weights, one per control point, for a rational curve.
    pub weights: Option<&'a [f64]>,
    /// The curve ends where it starts.
    pub closed: bool,
    /// Periodic. Always refused; it is here so the refusal is a tested rule.
    pub periodic: bool,
}

/// A spline read out of the arenas: every array is borrowed.
#[derive(Clone, Copy, Debug)]
pub struct SplineView<'a> {
    /// The entity id.
    pub id: u32,
    /// The style reference.
    pub style: u32,
    /// The degree.
    pub degree: usize,
    /// The closed flag.
    pub closed: bool,
    /// The knot vector.
    pub knots: &'a [f64],
    /// The weights of a rational curve.
    pub weights: Option<&'a [f64]>,
    /// Control point x values.
    pub x: &'a [f64],
    /// Control point y values.
    pub y: &'a [f64],
}

/// A composite curve read out of the arenas.
#[derive(Clone, Copy, Debug)]
pub struct CompositeView<'a> {
    /// The entity id.
    pub id: u32,
    /// The style reference.
    pub style: u32,
    /// The closed flag.
    pub closed: bool,
    /// Record indices of the segments, in order.
    pub segments: &'a [u32],
    /// `(t1, t2)` per segment, flattened, when the composite trims its segments.
    pub trims: Option<&'a [f64]>,
}

/// An advanced curve, borrowed from the blocks.
#[derive(Clone, Copy, Debug)]
pub enum CurveView<'a> {
    /// A B-spline.
    Spline(SplineView<'a>),
    /// A composite curve.
    Composite(CompositeView<'a>),
}

/// The six blocks of an advanced geometry section, written.
#[derive(Clone, Debug, Default)]
pub struct AdvancedBlocks {
    /// The record block.
    pub records: AlignedBytes,
    /// The knots arena.
    pub knots: AlignedBytes,
    /// The weights arena.
    pub weights: AlignedBytes,
    /// The control-points arena.
    pub control_points: AlignedBytes,
    /// The segments arena.
    pub segments: AlignedBytes,
    /// The trims arena.
    pub trims: AlignedBytes,
}

/// The six blocks as byte slices, each starting on an 8-byte boundary.
#[derive(Clone, Copy, Debug)]
pub struct BlockSlices<'a> {
    /// The record block.
    pub records: &'a [u8],
    /// The knots arena.
    pub knots: &'a [u8],
    /// The weights arena.
    pub weights: &'a [u8],
    /// The control-points arena.
    pub control_points: &'a [u8],
    /// The segments arena.
    pub segments: &'a [u8],
    /// The trims arena.
    pub trims: &'a [u8],
}

impl AdvancedBlocks {
    /// The blocks as slices.
    pub fn slices(&self) -> BlockSlices<'_> {
        BlockSlices {
            records: self.records.as_bytes(),
            knots: self.knots.as_bytes(),
            weights: self.weights.as_bytes(),
            control_points: self.control_points.as_bytes(),
            segments: self.segments.as_bytes(),
            trims: self.trims.as_bytes(),
        }
    }
}

fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A parsed record block: fixed 32-byte rows borrowed from the buffer.
#[derive(Clone, Copy, Debug)]
pub struct RecordsView<'a> {
    words: &'a [u32],
}

const WORDS_PER_RECORD: usize = RECORD_LEN / 4;

impl<'a> RecordsView<'a> {
    /// Decode a record block in place. The buffer must start on an 8-byte boundary. Every row is
    /// checked: ids ascending and below NP-18's limit, kinds, flags and the fields a kind may not use.
    pub fn parse(bytes: &'a [u8]) -> Result<RecordsView<'a>, ArenaError> {
        if cfg!(target_endian = "big") {
            return Err(ArenaError::HostEndianness);
        }
        if (bytes.as_ptr() as usize) % 8 != 0 {
            return Err(ArenaError::Misaligned);
        }
        if bytes.len() < RECORD_HEADER_LEN {
            return Err(ArenaError::Truncated { needed: RECORD_HEADER_LEN as u64, got: bytes.len() as u64 });
        }
        if bytes[..4] != RECORD_MAGIC {
            return Err(ArenaError::BadMagic);
        }
        let version = le_u16(bytes, 4);
        if version != RECORD_VERSION {
            return Err(ArenaError::UnsupportedVersion { found: version });
        }
        if bytes[6..8].iter().any(|&b| b != 0) || bytes[16..RECORD_HEADER_LEN].iter().any(|&b| b != 0) {
            return Err(ArenaError::NonZeroReserved);
        }
        let count = le_u32(bytes, 8);
        let declared = le_u32(bytes, 12);
        if count == 0 {
            return Err(ArenaError::EmptyBlock);
        }
        if count as usize >= MAX_ENTITIES {
            return Err(ArenaError::Numeric(NumericError::Limit));
        }
        if u64::from(declared) != bytes.len() as u64 {
            return Err(ArenaError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let total = RECORD_HEADER_LEN as u64 + RECORD_LEN as u64 * u64::from(count);
        if total > bytes.len() as u64 {
            return Err(ArenaError::Truncated { needed: total, got: bytes.len() as u64 });
        }
        if total < bytes.len() as u64 {
            return Err(ArenaError::LengthMismatch { declared, actual: bytes.len() as u64 });
        }
        let words = u32s(&bytes[RECORD_HEADER_LEN..]).ok_or(ArenaError::Misaligned)?;
        let view = RecordsView { words };
        let mut previous: Option<u32> = None;
        for index in 0..count as usize {
            let w = &words[index * WORDS_PER_RECORD..(index + 1) * WORDS_PER_RECORD];
            let id = w[0];
            if id as usize >= MAX_ENTITIES || previous.is_some_and(|p| id <= p) {
                return Err(ArenaError::IdsNotAscending { index });
            }
            previous = Some(id);
            view.decode(index)?;
        }
        Ok(view)
    }

    /// Decode and check one row.
    fn decode(&self, index: usize) -> Result<AdvancedCurveRecord, ArenaError> {
        let w = &self.words[index * WORDS_PER_RECORD..(index + 1) * WORDS_PER_RECORD];
        let kind = match w[2] & 0xff {
            1 => RecordKind::Spline,
            2 => RecordKind::Composite,
            other => return Err(ArenaError::UnknownKind { id: other as u16 }),
        };
        let degree = ((w[2] >> 8) & 0xff) as u8;
        let flags = (w[2] >> 16) as u16;
        if flags & !(FLAG_RATIONAL | FLAG_CLOSED | FLAG_PERIODIC) != 0 {
            return Err(ArenaError::InvalidFlags { bits: flags });
        }
        if flags & FLAG_PERIODIC != 0 {
            return Err(ArenaError::PeriodicUnsupported);
        }
        let r = AdvancedCurveRecord {
            id: w[0],
            style: w[1],
            kind,
            degree,
            flags,
            knots: w[3],
            weights: w[4],
            control_points: w[5],
            segments: w[6],
            trims: w[7],
        };
        let rational = flags & FLAG_RATIONAL != 0;
        let bad = ArenaError::RecordFields { index };
        match kind {
            RecordKind::Spline => {
                if r.knots == NO_ENTRY
                    || r.control_points == NO_ENTRY
                    || (r.weights == NO_ENTRY) == rational
                    || r.segments != NO_ENTRY
                    || r.trims != NO_ENTRY
                {
                    return Err(bad);
                }
            }
            RecordKind::Composite => {
                if rational {
                    return Err(ArenaError::InvalidFlags { bits: flags });
                }
                if degree != 0
                    || r.knots != NO_ENTRY
                    || r.weights != NO_ENTRY
                    || r.control_points != NO_ENTRY
                    || r.segments == NO_ENTRY
                {
                    return Err(bad);
                }
            }
        }
        Ok(r)
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.words.len() / WORDS_PER_RECORD
    }

    /// Whether there are no records (a parsed block never has none).
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Record `index`.
    pub fn get(&self, index: usize) -> Option<AdvancedCurveRecord> {
        if index >= self.len() {
            return None;
        }
        self.decode(index).ok()
    }
}

/// NP-14, NP-15, NP-20 and the counts, for one spline: the one place both the writer and the reader
/// decide whether the arrays belong together.
fn validate_spline(
    degree: usize,
    closed: bool,
    knots: &[f64],
    weights: Option<&[f64]>,
    x: &[f64],
    y: &[f64],
) -> Result<(), ArenaError> {
    let policy = Policy::V1;
    if !(1..=MAX_DEGREE).contains(&degree) {
        return Err(ArenaError::Numeric(NumericError::Limit));
    }
    if x.len() != y.len() {
        return Err(ArenaError::ControlPointColumns { x: x.len(), y: y.len() });
    }
    let n = x.len();
    if let Some(w) = weights {
        if w.len() != n {
            return Err(ArenaError::WeightCount { weights: w.len(), control_points: n });
        }
    }
    if knots.len() != n.saturating_add(degree).saturating_add(1) {
        return Err(ArenaError::KnotCount { knots: knots.len(), control_points: n, degree });
    }
    policy.validate_knots(degree, n, knots).map_err(ArenaError::Numeric)?;
    if let Some(w) = weights {
        policy.validate_weights(w).map_err(ArenaError::Numeric)?;
    }
    for &v in x.iter().chain(y) {
        check_value(v).map_err(ArenaError::Numeric)?;
    }
    if closed {
        let extent = |c: &[f64]| {
            let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
            let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            hi - lo
        };
        let scale = extent(x).max(extent(y));
        policy
            .check_closed_flag(true, [x[0], y[0]], [x[n - 1], y[n - 1]], scale)
            .map_err(ArenaError::Numeric)?;
    }
    Ok(())
}

/// A curve waiting for `finish`.
#[derive(Clone, Debug)]
struct Pending {
    id: u32,
    style: u32,
    body: Body,
}

#[derive(Clone, Debug)]
enum Body {
    Spline { degree: u8, flags: u16, knots: Vec<f64>, weights: Option<Vec<f64>>, x: Vec<f64>, y: Vec<f64> },
    Composite { flags: u16, segments: Vec<u32>, trims: Option<Vec<f64>> },
}

/// Collects advanced curves and writes the six blocks, ordered by `EntityId`.
#[derive(Clone, Debug, Default)]
pub struct AdvancedBuilder {
    pending: Vec<Pending>,
}

impl AdvancedBuilder {
    /// An empty builder.
    pub fn new() -> AdvancedBuilder {
        AdvancedBuilder { pending: Vec::new() }
    }

    /// Curves pushed so far.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing was pushed.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Add a B-spline, refused unless it is consistent (counts, knots, weights, closure).
    pub fn push_spline(&mut self, id: EntityId, style: u32, spec: SplineSpec<'_>) -> Result<(), ArenaError> {
        if spec.periodic {
            return Err(ArenaError::PeriodicUnsupported);
        }
        validate_spline(spec.degree, spec.closed, spec.knots, spec.weights, spec.x, spec.y)?;
        let mut flags = 0;
        if spec.weights.is_some() {
            flags |= FLAG_RATIONAL;
        }
        if spec.closed {
            flags |= FLAG_CLOSED;
        }
        self.pending.push(Pending {
            id: id.index(),
            style,
            body: Body::Spline {
                degree: spec.degree as u8,
                flags,
                knots: spec.knots.to_vec(),
                weights: spec.weights.map(<[f64]>::to_vec),
                x: spec.x.to_vec(),
                y: spec.y.to_vec(),
            },
        });
        Ok(())
    }

    /// Add a composite curve whose segments are the entities in `segments`, in order, with optional
    /// `(t1, t2)` trims per segment.
    pub fn push_composite(
        &mut self,
        id: EntityId,
        style: u32,
        segments: &[EntityId],
        trims: Option<&[[f64; 2]]>,
        closed: bool,
    ) -> Result<(), ArenaError> {
        if segments.is_empty() || segments.len() > MAX_COMPOSITE_SEGMENTS {
            return Err(ArenaError::Numeric(NumericError::Limit));
        }
        let trims = match trims {
            None => None,
            Some(t) => {
                if t.len() != segments.len() {
                    return Err(ArenaError::TrimCount { values: 2 * t.len(), segments: segments.len() });
                }
                let flat: Vec<f64> = t.iter().flatten().copied().collect();
                for &v in &flat {
                    check_value(v).map_err(ArenaError::Numeric)?;
                }
                Some(flat)
            }
        };
        self.pending.push(Pending {
            id: id.index(),
            style,
            body: Body::Composite {
                flags: if closed { FLAG_CLOSED } else { 0 },
                segments: segments.iter().map(|s| s.index()).collect(),
                trims,
            },
        });
        Ok(())
    }

    /// The blocks. Identical payloads share one arena range; the same curves give the same bytes
    /// whatever order they were pushed in.
    pub fn finish(self) -> Result<AdvancedBlocks, ArenaError> {
        let n = self.pending.len();
        if n == 0 {
            return Err(ArenaError::EmptyBlock);
        }
        if n >= MAX_ENTITIES {
            return Err(ArenaError::TooLarge);
        }
        let mut pending = self.pending;
        pending.sort_unstable_by_key(|p| p.id);
        if let Some(w) = pending.windows(2).find(|w| w[0].id == w[1].id) {
            return Err(ArenaError::DuplicateId { id: w[0].id });
        }
        let ids: Vec<u32> = pending.iter().map(|p| p.id).collect();

        let mut knots = ArenaBuilder::new(PayloadKind::Knots);
        let mut weights = ArenaBuilder::new(PayloadKind::Weights);
        let mut cps = ArenaBuilder::new(PayloadKind::ControlPoints);
        let mut segments = ArenaBuilder::new(PayloadKind::Segments);
        let mut trims = ArenaBuilder::new(PayloadKind::Trims);
        let mut rows: Vec<AdvancedCurveRecord> = Vec::with_capacity(n);
        for p in &pending {
            let mut row = AdvancedCurveRecord {
                id: p.id,
                style: p.style,
                kind: RecordKind::Spline,
                degree: 0,
                flags: 0,
                knots: NO_ENTRY,
                weights: NO_ENTRY,
                control_points: NO_ENTRY,
                segments: NO_ENTRY,
                trims: NO_ENTRY,
            };
            match &p.body {
                Body::Spline { degree, flags, knots: k, weights: w, x, y } => {
                    row.degree = *degree;
                    row.flags = *flags;
                    row.knots = knots.push_f64(k)?;
                    if let Some(w) = w {
                        row.weights = weights.push_f64(w)?;
                    }
                    row.control_points = cps.push_xy(x, y)?;
                }
                Body::Composite { flags, segments: seg, trims: t } => {
                    row.kind = RecordKind::Composite;
                    row.flags = *flags;
                    let mut indices = Vec::with_capacity(seg.len());
                    for s in seg {
                        let at = ids.binary_search(s).map_err(|_| ArenaError::UnknownSegment { id: *s })?;
                        indices.push(at as u32);
                    }
                    row.segments = segments.push_u32(&indices)?;
                    if let Some(t) = t {
                        row.trims = trims.push_f64(t)?;
                    }
                }
            }
            rows.push(row);
        }

        let mut out: Vec<u8> = Vec::with_capacity(RECORD_HEADER_LEN + RECORD_LEN * n);
        out.extend_from_slice(&RECORD_MAGIC);
        out.extend_from_slice(&RECORD_VERSION.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        let total = u32::try_from(RECORD_HEADER_LEN + RECORD_LEN * n).map_err(|_| ArenaError::TooLarge)?;
        out.extend_from_slice(&total.to_le_bytes());
        out.resize(RECORD_HEADER_LEN, 0);
        for r in &rows {
            let kind = match r.kind {
                RecordKind::Spline => 1u32,
                RecordKind::Composite => 2,
            };
            let word2 = kind | u32::from(r.degree) << 8 | u32::from(r.flags) << 16;
            for w in [r.id, r.style, word2, r.knots, r.weights, r.control_points, r.segments, r.trims] {
                out.extend_from_slice(&w.to_le_bytes());
            }
        }
        let blocks = AdvancedBlocks {
            records: AlignedBytes::from_bytes(&out),
            knots: knots.finish()?,
            weights: weights.finish()?,
            control_points: cps.finish()?,
            segments: segments.finish()?,
            trims: trims.finish()?,
        };
        // A cycle or a chain that is too deep is a property of the whole set, so it is checked here, once,
        // over what was just written.
        AdvancedView::parse(blocks.slices())?.check_nesting()?;
        Ok(blocks)
    }
}

/// The record block and the five arenas, parsed and borrowed.
#[derive(Clone, Copy, Debug)]
pub struct AdvancedView<'a> {
    records: RecordsView<'a>,
    knots: ArenaView<'a>,
    weights: ArenaView<'a>,
    control_points: ArenaView<'a>,
    segments: ArenaView<'a>,
    trims: ArenaView<'a>,
}

impl<'a> AdvancedView<'a> {
    /// Parse the six blocks.
    pub fn parse(blocks: BlockSlices<'a>) -> Result<AdvancedView<'a>, ArenaError> {
        AdvancedView::parse_with(blocks, None)
    }

    /// As [`AdvancedView::parse`], reporting every table and payload read to `probe`.
    pub fn parse_probed(
        blocks: BlockSlices<'a>,
        probe: &'a dyn ReadProbe,
    ) -> Result<AdvancedView<'a>, ArenaError> {
        AdvancedView::parse_with(blocks, Some(probe))
    }

    fn parse_with(
        blocks: BlockSlices<'a>,
        probe: Option<&'a dyn ReadProbe>,
    ) -> Result<AdvancedView<'a>, ArenaError> {
        let arena = |bytes: &'a [u8], kind: PayloadKind| {
            let v = ArenaView::parse_probed(bytes, probe)?;
            if v.kind() == kind { Ok(v) } else { Err(ArenaError::UnknownKind { id: v.kind().id() }) }
        };
        Ok(AdvancedView {
            records: RecordsView::parse(blocks.records)?,
            knots: arena(blocks.knots, PayloadKind::Knots)?,
            weights: arena(blocks.weights, PayloadKind::Weights)?,
            control_points: arena(blocks.control_points, PayloadKind::ControlPoints)?,
            segments: arena(blocks.segments, PayloadKind::Segments)?,
            trims: arena(blocks.trims, PayloadKind::Trims)?,
        })
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether there are no records (a parsed block never has none).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Record `index`, in ascending entity id order. Reads nothing from the arenas.
    pub fn record(&self, index: usize) -> Option<AdvancedCurveRecord> {
        self.records.get(index)
    }

    fn record_or_err(&self, index: usize) -> Result<AdvancedCurveRecord, ArenaError> {
        self.records.get(index).ok_or(ArenaError::EntryOutOfRange {
            index: index.min(u32::MAX as usize) as u32,
            entries: self.len() as u32,
        })
    }

    /// The segment list of composite `index`, every entry checked to name a record.
    fn segment_list(&self, index: usize, rec: &AdvancedCurveRecord) -> Result<&'a [u32], ArenaError> {
        let list = self.segments.slice_u32(rec.segments)?;
        if list.is_empty() || list.len() > MAX_COMPOSITE_SEGMENTS {
            return Err(ArenaError::Numeric(NumericError::Limit));
        }
        if let Some(&bad) = list.iter().find(|&&s| s as usize >= self.len()) {
            return Err(ArenaError::SegmentOutOfRange { record: index, segment: bad });
        }
        Ok(list)
    }

    /// The curve at record `index`, with every array borrowed from the arenas: at most one offset-table
    /// read and one arena read per payload kind. A composite's segments are not followed; see
    /// [`AdvancedView::nesting_depth`].
    pub fn curve(&self, index: usize) -> Result<CurveView<'a>, ArenaError> {
        let rec = self.record_or_err(index)?;
        let closed = rec.flags & FLAG_CLOSED != 0;
        match rec.kind {
            RecordKind::Spline => {
                let knots = self.knots.slice(rec.knots)?;
                let (x, y) = self.control_points.slice_xy(rec.control_points)?;
                let weights = if rec.flags & FLAG_RATIONAL != 0 {
                    Some(self.weights.slice(rec.weights)?)
                } else {
                    None
                };
                validate_spline(usize::from(rec.degree), closed, knots, weights, x, y)?;
                Ok(CurveView::Spline(SplineView {
                    id: rec.id,
                    style: rec.style,
                    degree: usize::from(rec.degree),
                    closed,
                    knots,
                    weights,
                    x,
                    y,
                }))
            }
            RecordKind::Composite => {
                let segments = self.segment_list(index, &rec)?;
                let trims = if rec.trims == NO_ENTRY {
                    None
                } else {
                    let t = self.trims.slice(rec.trims)?;
                    if t.len() != 2 * segments.len() {
                        return Err(ArenaError::TrimCount { values: t.len(), segments: segments.len() });
                    }
                    for &v in t {
                        check_value(v).map_err(ArenaError::Numeric)?;
                    }
                    Some(t)
                };
                Ok(CurveView::Composite(CompositeView {
                    id: rec.id,
                    style: rec.style,
                    closed,
                    segments,
                    trims,
                }))
            }
        }
    }

    /// The nesting depth of the record (a spline is 1), refusing a cycle or a depth over NP-18's limit.
    /// Resolve a composite's segments only after this returns `Ok`.
    pub fn nesting_depth(&self, index: usize) -> Result<usize, ArenaError> {
        self.record_or_err(index)?;
        self.depth(index, 1, &mut Walk::default())
    }

    /// `nesting_depth` of every record, sharing the work between them: each record is read once, and each
    /// segment list is scanned once however many composites use it, so the cost is the size of the
    /// file and not the product of composites and list lengths.
    pub fn check_nesting(&self) -> Result<(), ArenaError> {
        let mut walk = Walk::default();
        for index in 0..self.len() {
            self.depth(index, 1, &mut walk)?;
        }
        Ok(())
    }

    /// Height of the record at `index` when it sits at `level` (the outermost curve is level 1).
    fn depth(&self, index: usize, level: usize, walk: &mut Walk) -> Result<usize, ArenaError> {
        if level > MAX_NESTING_DEPTH {
            return Err(ArenaError::NestingTooDeep { record: index });
        }
        if walk.open.contains(&index) {
            return Err(ArenaError::CompositeCycle { record: index });
        }
        if let Some(&h) = walk.done.get(&index) {
            if level + h - 1 > MAX_NESTING_DEPTH {
                return Err(ArenaError::NestingTooDeep { record: index });
            }
            return Ok(h);
        }
        let rec = self.record_or_err(index)?;
        let height = match rec.kind {
            RecordKind::Spline => 1,
            RecordKind::Composite => {
                walk.open.insert(index);
                let tallest = match walk.lists.get(&rec.segments) {
                    Some(&t) => t,
                    None => {
                        let list = self.segment_list(index, &rec)?;
                        let mut t = 0;
                        for &s in list {
                            t = t.max(self.depth(s as usize, level + 1, walk)?);
                        }
                        walk.lists.insert(rec.segments, t);
                        t
                    }
                };
                walk.open.remove(&index);
                1 + tallest
            }
        };
        if level + height - 1 > MAX_NESTING_DEPTH {
            return Err(ArenaError::NestingTooDeep { record: index });
        }
        walk.done.insert(index, height);
        Ok(height)
    }
}

/// What the nesting walk remembers: finished heights, the records on the current path, and the height of
/// each segment list already scanned (keyed by its offset-table index).
#[derive(Default)]
struct Walk {
    done: HashMap<usize, usize>,
    open: HashSet<usize>,
    lists: HashMap<u32, usize>,
}

/// Chapter 3 of `docs/CONTAINER.md`, rendered from the constants in this module and in `arena`.
pub fn render_chapter() -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "## 3. Advanced geometry arenas\n\n\
B-spline knot vectors, weight arrays, control-point sets, composite segment lists and trim domains have a \
different length for every entity, which the fixed columns of chapter 2 cannot express. They live in \
arenas: one block per payload kind, holding every entity's values back to back, with an offset table \
that says where each entity's run starts and how long it is. An advanced curve is a fixed record of \
integers, so the curve is located by two integer reads (its record, then the offset-table entry of the \
payload kind it needs) and each payload is one contiguous run. This is layout B of ADR 0004 and the \
panel's sixth finding: there is no nested per-entity byte blob to chase and no per-entity allocation. \
The exact curve parameters are those of ADR 0011 and `docs/GEOMETRY_ENCODING.md`; this chapter is the \
container's view of them.\n\n\
### 3.1 Arena block\n\n\
An arena block is a {ARENA_HEADER_LEN} byte header, the offset table, and the payload, all little-endian. \
The first byte of the block must be 8-aligned in memory (the reader refuses a buffer that is not, rather \
than copying it). Offsets are from the first byte of the block.\n\n"
    ));
    s.push_str(&crate::spec::table(ARENA_HEADER_FIELDS));
    s.push_str(&format!(
        "\nAfter the header come `entry_count` offset-table entries of {OFFSET_ENTRY_LEN} bytes each (see \
section 3.3), which leaves the payload on a multiple of 8, and then the payload columns, each \
`element_count` elements long, one after the other with no padding between them. Every column starts at \
its natural alignment: an `f64` column on a multiple of 8 and a `u32` column on a multiple of 4, from the \
first byte of the block, which is what a `Float64Array` over WASM linear memory needs. The block is a \
multiple of 8 bytes long, so the only padding it can contain is the 0 or 4 bytes after a segment arena \
with an odd element count. Padding is zero and a reader refuses a block where it is not, which keeps the \
encoding canonical.\n\n\
### 3.2 Payload kinds\n\n\
| id | kind | element | columns | one entry holds |\n|---|---|---|---|---|\n"
    ));
    for k in PayloadKind::ALL {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            k.id(),
            k.name(),
            k.element(),
            k.columns().iter().map(|c| format!("`{c}`")).collect::<Vec<_>>().join(", "),
            k.meaning()
        ));
    }
    s.push_str(
        "\nCoordinates, knots, weights and trims are `f64` end to end, so a value comes back with the bits it \
went in with. The two control-point columns have the same length and the entry's range applies to both.\n\n\
### 3.3 The offset table\n\n\
An entry is `start: u32` then `len: u32`, both counted in elements (not bytes) of the arena's columns. \
Entry `i` of the table is at `32 + 8 * i`. The entry names the run `start..start + len` of each column. \
Nothing requires entries to be ordered or disjoint: two entries may name the same range, and that is how \
equal payloads share storage (section 3.6). An index of `ffffffff` in a record means the curve has no \
payload of that kind; it is never a table index.\n\n\
A reader checks an entry in range before it hands out a slice: `start + len` is added with overflow \
checked, and the sum must not exceed `element_count`. An entry that fails is refused, never clamped. The \
table is checked entry by entry as it is used, so opening an arena does not cost anything in the number \
of entries.\n\n",
    );
    s.push_str(&format!(
        "### 3.4 Advanced curve records\n\n\
The record block is a {RECORD_HEADER_LEN} byte header followed by `record_count` records of {RECORD_LEN} bytes \
each, in strictly ascending entity id order (P3.6), so the same set of curves always encodes to the same \
bytes (P4.4). A record is degree, flags and one offset-table index per arena, and nothing else: there is \
no nested per-entity byte blob, and a record never holds a number of the curve.\n\n"
    ));
    s.push_str(&crate::spec::table(RECORD_HEADER_FIELDS));
    s.push_str("\nThe record:\n\n");
    s.push_str(&crate::spec::table(RECORD_FIELDS));
    s.push_str(
        "\nA spline (kind 1) needs knots and control points, and weights if and only if the rational flag is \
set; it has no segment list or trims. A composite (kind 2) has a degree of 0, a segment list, optionally \
trims, and no knots, weights or control points; it cannot be rational. The closed flag is a claim about the \
geometry that NP-20 checks for a spline (the end meets the start); for a composite it is carried and not \
checked, because checking it needs the segments evaluated, which is not this layer's job.\n\n\
The periodic flag is defined and refused. ISO_PROFILE section 11 and ADR 0011 say the profile has no \
wrapped periodic form (a closed curve is an ordinary clamped B-spline), so there is no periodic rule to \
apply, and a record that sets the bit makes the whole block unreadable rather than half-understood. \
A periodic spline is unwrapped before it is stored (`BSpline::from_periodic`, ADR 0011), so the writer never has a reason to set the bit. A later minor version can give the bit a meaning by lifting that refusal.\n\n\
### 3.5 Counts, weights and composites\n\n\
For a spline of degree `p` with `n` control points the knot vector has `n + p + 1` entries and a rational \
curve has exactly `n` weights. A weight array that is shorter is refused, not padded with ones, and so is \
one that is longer. Beyond the counts, the knots go through NP-14 (non-decreasing, multiplicities, no \
ambiguous pairs), the weights through NP-15, every coordinate through NP-01 and NP-02, and the closed \
flag through NP-20.\n\n\
A composite's segment list is a list of record indices: it names other records of the same block, so a \
segment that two composites use is one record and its geometry is stored once. The list has 1 to 65536 \
entries (NP-18) and, when the composite has trims, the trims entry has two numbers `t1`, `t2` per \
segment. This layer does not check a trim against the curve it trims (NP-12 needs that curve's \
domain), and it only references records of its own block: a segment that is a hot line or arc has to be \
written as a degree 1 spline here.\n\n\
Composites may nest up to the depth of NP-18, counting the outermost curve as 1, so a composite of \
composites of splines is depth 3. This is an allowance of the container, which stores a composite as \
references so that shared geometry is stored once. It is not a change to the Part 21 form of \
`docs/ISO_PROFILE.md` section 9, where a `composite_curve` segment's parent is never another composite: \
a writer of Part 21 (P6.2) expands nested references into one flat list of segments, as that section says, \
and ADR 0004 records the amendment. `curve` follows nothing. `nesting_depth` walks the references with \
each record read once, and refuses a cycle (a composite that leads back to itself, itself included) and a \
chain deeper than the limit before anything is resolved. `check_nesting` does the same for every record \
at once and scans each segment list once, however many composites share it, so its cost is the size of \
the file and not the product of composites and list lengths. The writer runs it over everything it wrote \
and refuses the set if it fails.\n\n\
### 3.6 Sharing and canonical order\n\n\
An arena builder adds a payload once: if the same bits were added before, it returns the existing table \
index. Two entities with the same knot vector therefore share one table entry and one arena range, and \
the record of each points at it. Entries are numbered in the order the writer first needs them, and the \
writer visits curves in ascending entity id order, so the bytes do not depend on the order curves were \
pushed in. Sharing compares bits, not values: vectors one ulp apart are two payloads.\n\n\
### 3.7 What a reader may assume and what it must verify\n\n\
A reader may assume, once the six blocks have parsed: every block is exactly as long as its header says, \
its padding is zero, and every column is inside the buffer and at its natural alignment; the record \
block's ids ascend and are below 2^28; every record has a known kind, only defined flags, no periodic \
flag, and exactly the index fields its kind uses; and each arena is the kind its slot expects.\n\n\
A reader must verify, as it uses them: that an offset-table index is below the entry count and the entry \
lies inside the arena (`ArenaView::slice` and its siblings do both); that the numbers are finite and \
satisfy the numeric policy (`AdvancedView::curve` does, for a spline); that a composite's segment \
indices name records and its trims are twice as many as its segments (`curve` does); and that nesting \
is acyclic and shallow enough (`nesting_depth`, which a reader calls before it resolves segments). It \
must also apply the ceilings of P4.8, which cite NP-18 and are not restated here, and it must not assume \
two entries are disjoint.\n\n\
Reading one curve costs at most one offset-table read and one arena read for each payload kind it has, \
and the number does not depend on how many curves are in the file; a test counts them with a probe.\n\n\
### 3.8 Refusals\n\n\
Each refusal is a typed `ArenaError` with a rule identifier. Every count is compared with the bytes \
present before it is used, and parsing allocates nothing.\n\n\
| rule | meaning |\n|---|---|\n",
    );
    for (rule, meaning) in ARENA_RULES {
        s.push_str(&format!("| {rule} | {meaning} |\n"));
    }
    s
}
