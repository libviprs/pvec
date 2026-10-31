//! The deterministic container writer (issue P4.4).
//!
//! The same semantic input gives the same bytes: whatever order the entities, capabilities and
//! component streams were handed over in, whichever thread pushed them, whichever process or
//! machine ran the write, and at whatever time. That is what lets an artifact be cached by digest,
//! diffed in review and used as the baseline of a fidelity report.
//!
//! Three things make it so, and `docs/CONTAINER.md` chapter 4 is the contract for all of them:
//!
//! * the canonical order of blocks, entities, capabilities and streams is decided here, by keys
//!   the caller never sees, and never by the order of arrival ([`SortKey`]);
//! * every scalar is written with `to_le_bytes`, so the host byte order is not consulted;
//! * nothing ambient reaches the output: no clock, path, host name, hash-map iteration or address.
//!   The producer identity is whatever the caller put in [`WriterConfig`].

use crate::blocks::advanced::{AdvancedBuilder, SplineSpec};
use crate::blocks::arena::{ArenaError, PayloadKind};
use crate::blocks::hot::{HotBlockBuilder, HotError, HotKind};
use crate::blocks::raster::{self, IndexError, TileIn};
use crate::compress::CodecPolicy;
use crate::compress::pack;
use crate::directory::{
    BLOCK_ALIGN, BlockEntry, BlockKind, ENTRY_LEN, FLAG_HAS_BOUNDS, FLAG_MUST_UNDERSTAND,
};
use crate::error::ContainerError;
use crate::header::{ChecksumAlgorithm, Header};
use crate::integrity::{Integrity, artifact_digest, block_checksum};
use crate::limits::Limits;
use crate::manifest::{Bounds, ByteRange, DirectoryPointer, LengthUnit, Manifest, Metadata};
use crate::spec::{cap, dir};
use core::cmp::Ordering;
use core::fmt;
use pvec_capabilities::{CapabilityId, Declared, Registry, Requirement, Version};
use pvec_components::{
    Placement, RASTER_CAPABILITY, RasterError, RasterId, Stream, StreamError, decode_rasters, encode_streams,
};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, GeometryError, Point2};
use pvec_numeric::check_value;
use std::collections::BTreeMap;

/// A key of the canonical ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Blocks by block kind.
    BlockKind,
    /// Blocks by spatial cell id.
    SpatialCell,
    /// Blocks by geometry type.
    GeometryType,
    /// Entities in a block by `EntityId`.
    EntityId,
    /// Manifest capabilities by capability id.
    CapabilityId,
    /// Component streams by stream id.
    StreamId,
}

impl SortKey {
    /// Every key.
    pub const ALL: [SortKey; 6] = [
        SortKey::BlockKind,
        SortKey::SpatialCell,
        SortKey::GeometryType,
        SortKey::EntityId,
        SortKey::CapabilityId,
        SortKey::StreamId,
    ];
}

/// What the caller decides about an artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct WriterConfig {
    /// Unit of length of source space.
    pub unit: LengthUnit,
    /// Producer identity and corpus revision, as the caller supplies them.
    pub metadata: Metadata,
    /// Source bounds; the union of the entity bounds when `None`.
    pub bounds: Option<Bounds>,
    /// Capabilities to declare, in any order.
    pub capabilities: Vec<CapabilityId>,
}

/// How an artifact is compressed and sealed. The default is the one P8.6 settled on (`CodecPolicy::default`:
/// deflate for hot blocks, LZ4 for the advanced records and arenas, deflate for Tier 0 whatever the policy)
/// and writes both the block checksum table and the artifact digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct WriterOptions {
    /// What is recorded for integrity.
    pub integrity: Integrity,
    /// Which codec each kind of block is written with.
    pub codecs: CodecPolicy,
}

/// A block ready to be placed by [`assemble`]: the descriptor fields that are the caller's, and the bytes
/// as they are stored. The offset and the stored length are the assembler's to decide.
#[derive(Clone, Debug, PartialEq)]
pub struct RawBlock {
    /// The kind id.
    pub kind: u16,
    /// The subtype.
    pub subtype: u16,
    /// The spatial cell.
    pub cell: u32,
    /// The count.
    pub count: u32,
    /// The bounds of the entities, when the block has any.
    pub bounds: Option<Bounds>,
    /// The flag bits.
    pub flags: u8,
    /// The codec id.
    pub codec: u8,
    /// The capability id, or 0.
    pub capability: u16,
    /// The length once decoded.
    pub uncompressed_length: u32,
    /// The bytes as stored.
    pub stored: Vec<u8>,
}

/// Lays out an artifact from a manifest and blocks that are already encoded: header, manifest, the blocks
/// each on a multiple of 8, the checksum table when `integrity` asks for one, and the directory last, with
/// zero padding between and nothing else. The manifest's directory pointer, its checksum table pointer
/// and its digest are set here, whatever the manifest it was given held. This is what the writer itself
/// uses, and it is how a tool that must carry a block it does not understand byte for byte (or a test that
/// needs a file no writer would make) builds one: it checks the lengths fit and nothing else about the
/// blocks.
pub fn assemble(
    mut manifest: Manifest,
    blocks: &[RawBlock],
    integrity: Integrity,
) -> Result<Vec<u8>, WriteError> {
    let sealed = integrity != Integrity::None;
    manifest.directories =
        vec![DirectoryPointer { kind: DirectoryPointer::KIND_BLOCK, offset: 0, length: 0 }];
    manifest.block_checksums = sealed.then_some(ByteRange { offset: 0, length: 0 });
    manifest.artifact_digest = (integrity == Integrity::Full).then_some([0u8; 32]);
    // the manifest's length depends on counts, not on the offsets it holds, so it is known now
    let manifest_len = manifest.encode()?.len() as u64;
    let mut at = Header::LEN as u64 + manifest_len;
    let mut entries: Vec<BlockEntry> = Vec::with_capacity(blocks.len());
    for b in blocks {
        let stored_length = u32::try_from(b.stored.len()).map_err(|_| WriteError::TooLarge)?;
        at = align8(at);
        entries.push(BlockEntry {
            offset: at,
            stored_length,
            uncompressed_length: b.uncompressed_length,
            count: b.count,
            cell: b.cell,
            bounds: b.bounds,
            kind: b.kind,
            subtype: b.subtype,
            flags: b.flags,
            codec: b.codec,
            capability: b.capability,
        });
        at += u64::from(stored_length);
    }
    let table_offset = align8(at);
    let table_length = if sealed { 8 * blocks.len() as u64 } else { 0 };
    let directory_offset = align8(if sealed { table_offset + table_length } else { at });
    let directory_length = (entries.len() * ENTRY_LEN) as u64;
    let total = directory_offset.checked_add(directory_length).ok_or(WriteError::TooLarge)?;
    let total = usize::try_from(total).map_err(|_| WriteError::TooLarge)?;
    manifest.directories[0] = DirectoryPointer {
        kind: DirectoryPointer::KIND_BLOCK,
        offset: directory_offset,
        length: directory_length,
    };
    if sealed {
        manifest.block_checksums = Some(ByteRange { offset: table_offset, length: table_length });
    }
    let manifest_bytes = manifest.encode()?;
    let header = Header {
        major: Header::MAJOR,
        minor: Header::MINOR,
        header_len: Header::LEN as u16,
        block_checksum: ChecksumAlgorithm::Xxh3_64,
        artifact_checksum: ChecksumAlgorithm::Sha256,
        manifest_offset: Header::LEN as u64,
        manifest_length: manifest_len,
        directory_table_offset: Header::LEN as u64
            + Manifest::FIXED_LEN as u64
            + (cap::SIZE * manifest.capabilities.len()) as u64,
        directory_table_length: dir::SIZE as u64,
    };

    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(&manifest_bytes);
    for (b, e) in blocks.iter().zip(&entries) {
        out.resize(e.offset as usize, 0);
        out.extend_from_slice(&b.stored);
    }
    if sealed {
        out.resize(table_offset as usize, 0);
        for b in blocks {
            out.extend_from_slice(&block_checksum(&b.stored).to_le_bytes());
        }
    }
    out.resize(directory_offset as usize, 0);
    for e in &entries {
        out.extend_from_slice(&e.encode());
    }
    debug_assert_eq!(out.len(), total);
    if integrity == Integrity::Full {
        // the digest covers everything with its own 32 bytes read as zero, so it is computed over the file
        // as it stands (they are zero now) and written into the place it was left out of
        let at = Header::LEN + crate::spec::m::ARTIFACT_DIGEST;
        let digest = artifact_digest(&out, at);
        out[at..at + 32].copy_from_slice(&digest);
    }
    Ok(out)
}

/// What every entity carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityMeta {
    /// The entity id.
    pub id: EntityId,
    /// Opaque style reference.
    pub style: u32,
    /// Spatial cell id.
    pub cell: u32,
    /// Bounds of the entity in source space.
    pub bounds: Bounds,
}

/// Why writing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError {
    /// There is no entity to write.
    Empty,
    /// An entity id is used twice.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// An advanced curve was given a spatial cell.
    CellOnAdvanced {
        /// The id.
        id: u32,
    },
    /// A capability id the registry does not know.
    UnknownCapability {
        /// The id.
        id: u32,
    },
    /// Two component streams of one capability.
    DuplicateStream {
        /// The capability id.
        capability: u32,
    },
    /// Bounds that are not finite, in range and ordered.
    BadBounds {
        /// The entity, or `None` for the source bounds.
        id: Option<u32>,
    },
    /// A block or the file does not fit its length fields.
    TooLarge,
    /// A curve the writer cannot store.
    Unsupported {
        /// What it was.
        what: &'static str,
    },
    /// A hot block builder refused.
    Hot(HotError),
    /// An advanced block builder refused.
    Arena(ArenaError),
    /// The component stream encoder refused.
    Stream(StreamError),
    /// The manifest refused.
    Container(ContainerError),
    /// The semantic model refused.
    Geometry(GeometryError),
    /// A Tier 0 block in cell 0, in a straddler payload cell, or a second one for a cell.
    Tier0Cell {
        /// The cell.
        cell: u32,
    },
    /// Raster tiles the container cannot store (the error carries its `RASTER-nn` or `LIMIT-14` rule).
    Raster(IndexError),
    /// The raster placement stream is refused (the error carries its rule).
    Placement(RasterError),
}

impl WriteError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            WriteError::Empty => "WRT-01",
            WriteError::DuplicateId { .. } => "WRT-02",
            WriteError::CellOnAdvanced { .. } => "WRT-03",
            WriteError::UnknownCapability { .. } => "WRT-04",
            WriteError::DuplicateStream { .. } => "WRT-05",
            WriteError::BadBounds { .. } => "WRT-06",
            WriteError::TooLarge => "WRT-07",
            WriteError::Unsupported { .. } => "WRT-08",
            WriteError::Hot(_) => "WRT-09",
            WriteError::Arena(_) => "WRT-10",
            WriteError::Stream(_) => "WRT-11",
            WriteError::Container(_) => "WRT-12",
            WriteError::Geometry(_) => "WRT-13",
            WriteError::Tier0Cell { .. } => "WRT-14",
            WriteError::Raster(_) => "WRT-15",
            WriteError::Placement(_) => "WRT-16",
        }
    }
}

/// The refusals, rule by rule.
pub const WRITE_RULES: &[(&str, &str)] = &[
    ("WRT-01", "there is no entity to write (component streams alone are not an artifact)"),
    ("WRT-02", "an entity id is used by two entities, whatever their geometry types"),
    ("WRT-03", "an advanced curve was given a spatial cell; cells belong to hot blocks"),
    ("WRT-04", "a capability id the registry does not know"),
    ("WRT-05", "two component streams of one capability"),
    ("WRT-06", "bounds that are not finite, in range and ordered, on an entity or as the source bounds"),
    ("WRT-07", "a block or the artifact does not fit its 32-bit length fields"),
    ("WRT-08", "a curve the container cannot store exactly and deterministically"),
    ("WRT-09", "a hot block builder refused (the error carries its HOT rule)"),
    ("WRT-10", "an advanced block builder refused (the error carries its ARN rule)"),
    ("WRT-11", "the component stream encoder refused"),
    ("WRT-12", "the manifest refused (the error carries its container error)"),
    ("WRT-13", "the semantic model refused"),
    (
        "WRT-14",
        "a Tier 0 block in cell 0 or in a straddler payload cell, or a second Tier 0 block for one cell",
    ),
    (
        "WRT-15",
        "raster tiles the container cannot store: tiles for a raster the placement stream does not place, a tile outside its grid or given twice, a tile longer than a chunk, a grid over the ceiling (the error carries its RASTER rule)",
    ),
    ("WRT-16", "the raster placement stream is refused (the error carries its RASTER rule)"),
];

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            WriteError::Empty => write!(f, "{rule}: there is no entity to write"),
            WriteError::DuplicateId { id } => write!(f, "{rule}: entity id {id} is used twice"),
            WriteError::CellOnAdvanced { id } => {
                write!(f, "{rule}: advanced curve {id} was given a spatial cell")
            }
            WriteError::UnknownCapability { id } => write!(f, "{rule}: the registry has no capability {id}"),
            WriteError::DuplicateStream { capability } => {
                write!(f, "{rule}: two component streams of capability {capability}")
            }
            WriteError::BadBounds { id: Some(id) } => write!(f, "{rule}: entity {id} has invalid bounds"),
            WriteError::BadBounds { id: None } => write!(f, "{rule}: the source bounds are invalid"),
            WriteError::TooLarge => {
                write!(f, "{rule}: a block or the artifact does not fit its length fields")
            }
            WriteError::Unsupported { what } => write!(f, "{rule}: cannot store a {what}"),
            WriteError::Hot(e) => write!(f, "{rule}: {e}"),
            WriteError::Arena(e) => write!(f, "{rule}: {e}"),
            WriteError::Stream(e) => write!(f, "{rule}: {e}"),
            WriteError::Container(e) => write!(f, "{rule}: {e}"),
            WriteError::Geometry(e) => write!(f, "{rule}: {e}"),
            WriteError::Tier0Cell { cell } => {
                write!(f, "{rule}: a Tier 0 block cannot be in cell {cell}, or twice")
            }
            WriteError::Raster(e) => write!(f, "{rule}: {e}"),
            WriteError::Placement(e) => write!(f, "{rule}: {e}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<HotError> for WriteError {
    fn from(e: HotError) -> WriteError {
        WriteError::Hot(e)
    }
}

impl From<ArenaError> for WriteError {
    fn from(e: ArenaError) -> WriteError {
        WriteError::Arena(e)
    }
}

impl From<StreamError> for WriteError {
    fn from(e: StreamError) -> WriteError {
        WriteError::Stream(e)
    }
}

impl From<ContainerError> for WriteError {
    fn from(e: ContainerError) -> WriteError {
        WriteError::Container(e)
    }
}

impl From<GeometryError> for WriteError {
    fn from(e: GeometryError) -> WriteError {
        WriteError::Geometry(e)
    }
}

/// The geometry of a hot entity, as it was pushed.
#[derive(Clone, Debug)]
enum HotShape {
    Line([f64; 2], [f64; 2]),
    Polyline(Vec<[f64; 2]>),
    CircleArc { centre: [f64; 2], radius: f64, start_sweep: [f64; 2] },
    EllipseArc { centre: [f64; 2], radii: [f64; 2], angles: [f64; 3] },
}

impl HotShape {
    fn kind(&self) -> HotKind {
        match self {
            HotShape::Line(..) => HotKind::Line,
            HotShape::Polyline(_) => HotKind::Polyline,
            HotShape::CircleArc { .. } => HotKind::CircleArc,
            HotShape::EllipseArc { .. } => HotKind::EllipseArc,
        }
    }
}

#[derive(Clone, Debug)]
struct HotItem {
    seq: usize,
    meta: EntityMeta,
    shape: HotShape,
}

#[derive(Clone, Debug)]
enum AdvancedShape {
    Spline {
        degree: usize,
        knots: Vec<f64>,
        x: Vec<f64>,
        y: Vec<f64>,
        weights: Option<Vec<f64>>,
        closed: bool,
    },
    Composite {
        segments: Vec<EntityId>,
        trims: Option<Vec<[f64; 2]>>,
        closed: bool,
    },
}

#[derive(Clone, Debug)]
struct AdvancedItem {
    seq: usize,
    meta: EntityMeta,
    shape: AdvancedShape,
}

/// The default size of one chunk of the advanced curves, in bytes of records and arena data
/// (`docs/STREAMING.md` section 2.8). It was 64 KiB until P8.6, and is 256 KiB since (`docs/bench/TUNING.md`): a
/// viewport reads only the chunks it touches (`ReadPlan::advanced_chunks`), so a smaller chunk moves fewer bytes,
/// and below 256 KiB the file and its block directory grow faster than the bytes shrink.
pub const ADVANCED_CHUNK_BYTES: u32 = 256 * 1024;

/// The most bytes of tiles one kind 10 block holds by default: the block ceiling of the browser preset, so a file
/// opens under both presets. A level that would pass it is split into chunks.
pub const RASTER_CHUNK_BYTES: u32 = 1 << 26;

/// The bytes of the coarsest raster levels the index blocks hold inline by default, over every raster of the
/// artifact: the Tier 0 share of the rasters (ADR 0018 proposes 256 KiB, and issue 220 measures it).
pub const RASTER_INLINE_BYTES: u64 = 256 * 1024;

/// A block ready to be placed.
struct Planned {
    first_seen: usize,
    kind: BlockKind,
    cell: u32,
    subtype: u16,
    count: u32,
    bounds: Option<Bounds>,
    bytes: Vec<u8>,
}

/// The writer.
///
/// Push entities and streams in any order, then [`ContainerWriter::finish`]. The order of the pushes
/// is never visible in the output.
#[derive(Debug)]
pub struct ContainerWriter {
    config: WriterConfig,
    options: WriterOptions,
    skipped: Vec<SortKey>,
    seq: usize,
    hot: Vec<HotItem>,
    advanced: Vec<AdvancedItem>,
    advanced_chunk: u32,
    streams: Vec<(usize, Stream)>,
    spatial: Option<SpatialItem>,
    derived: Vec<Derived>,
    cell_refs: Option<RefsItem>,
    straddlers: Option<SpatialItem>,
    tier0: Vec<Tier0Item>,
    rasters: BTreeMap<u32, Vec<TileIn>>,
    raster_chunk: u32,
    raster_inline: u64,
}

/// A stored number the caller did not hand over: the writer derived it, within `bound` of the exact
/// value (radians), with the deterministic arctangent of `pvec_numeric::detmath`.
#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    /// The entity the number belongs to.
    pub entity: EntityId,
    /// What was derived and how.
    pub what: &'static str,
    /// A bound on the distance from the exact value, in radians.
    pub bound: f64,
}

/// The Tier 0 block of one leaf as the caller built it.
#[derive(Debug)]
struct Tier0Item {
    seq: usize,
    cell: u32,
    count: u32,
    bounds: Bounds,
    bytes: Vec<u8>,
}

/// The spatial directory block as the caller built it.
#[derive(Clone, Debug)]
struct SpatialItem {
    seq: usize,
    nodes: u32,
    bounds: Bounds,
    bytes: Vec<u8>,
}

/// Half the spacing of the doubles at one (2^-53): the relative size of one rounding.
const UNIT: f64 = f64::from_bits(0x3ca0_0000_0000_0000);

/// The reference arrays block as the caller built it.
#[derive(Clone, Debug)]
struct RefsItem {
    seq: usize,
    cells: u32,
    bytes: Vec<u8>,
}

fn normal(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: [normal(a.min[0].min(b.min[0])), normal(a.min[1].min(b.min[1]))],
        max: [normal(a.max[0].max(b.max[0])), normal(a.max[1].max(b.max[1]))],
    }
}

/// `bounds` if every value is finite and in range under the numeric policy and the box is ordered.
fn checked_bounds(bounds: Bounds) -> Option<Bounds> {
    let mut v = [0.0f64; 4];
    for (slot, x) in v.iter_mut().zip([bounds.min[0], bounds.min[1], bounds.max[0], bounds.max[1]]) {
        *slot = check_value(x).ok()?;
    }
    if v[0] > v[2] || v[1] > v[3] {
        return None;
    }
    Some(Bounds { min: [v[0], v[1]], max: [v[2], v[3]] })
}

/// The box of some points, from comparisons only.
fn boxed(points: &[[f64; 2]]) -> Bounds {
    let first = points.first().copied().unwrap_or([0.0, 0.0]);
    points.iter().fold(Bounds { min: first, max: first }, |b, p| union(b, Bounds { min: *p, max: *p }))
}

fn hull(x: &[f64], y: &[f64]) -> Bounds {
    let points: Vec<[f64; 2]> = x.iter().zip(y).map(|(a, b)| [*a, *b]).collect();
    boxed(&points)
}

/// The box of an axis-aligned ellipse (or circle) around `centre`, from additions only.
fn around(centre: [f64; 2], rx: f64, ry: f64) -> Bounds {
    Bounds { min: [centre[0] - rx, centre[1] - ry], max: [centre[0] + rx, centre[1] + ry] }
}

fn xy(p: Point2) -> [f64; 2] {
    [p.x, p.y]
}

fn align8(n: u64) -> u64 {
    n.div_ceil(BLOCK_ALIGN) * BLOCK_ALIGN
}

impl ContainerWriter {
    /// A writer with nothing in it.
    pub fn new(config: WriterConfig) -> ContainerWriter {
        ContainerWriter::with_options(config, WriterOptions::default())
    }

    /// A writer that compresses and seals as `options` say.
    pub fn with_options(config: WriterConfig, options: WriterOptions) -> ContainerWriter {
        ContainerWriter {
            config,
            options,
            skipped: Vec::new(),
            seq: 0,
            hot: Vec::new(),
            advanced: Vec::new(),
            advanced_chunk: ADVANCED_CHUNK_BYTES,
            streams: Vec::new(),
            spatial: None,
            derived: Vec::new(),
            cell_refs: None,
            straddlers: None,
            tier0: Vec::new(),
            rasters: BTreeMap::new(),
            raster_chunk: RASTER_CHUNK_BYTES,
            raster_inline: RASTER_INLINE_BYTES,
        }
    }

    /// Switches one sort off, so a test can show that the determinism suite notices.
    ///
    /// With a sort off the writer falls back to the order of arrival for that key, which is exactly
    /// the bug the key exists to prevent. Nothing but the mutation tests calls it.
    #[doc(hidden)]
    pub fn skip_sort(&mut self, key: SortKey) {
        if !self.skipped.contains(&key) {
            self.skipped.push(key);
        }
    }

    fn sorts(&self, key: SortKey) -> bool {
        !self.skipped.contains(&key)
    }

    fn next_seq(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    fn checked_meta(meta: EntityMeta) -> Result<EntityMeta, WriteError> {
        match checked_bounds(meta.bounds) {
            Some(bounds) => Ok(EntityMeta { bounds, ..meta }),
            None => Err(WriteError::BadBounds { id: Some(meta.id.index()) }),
        }
    }

    fn push_hot(&mut self, meta: EntityMeta, shape: HotShape) -> Result<(), WriteError> {
        let meta = ContainerWriter::checked_meta(meta)?;
        let seq = self.next_seq();
        self.hot.push(HotItem { seq, meta, shape });
        Ok(())
    }

    /// Adds a line segment from `a` to `b`.
    pub fn push_line(&mut self, meta: EntityMeta, a: [f64; 2], b: [f64; 2]) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::Line(a, b))
    }

    /// Adds a polyline of 2 to 2^20 vertices.
    pub fn push_polyline(&mut self, meta: EntityMeta, points: &[[f64; 2]]) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::Polyline(points.to_vec()))
    }

    /// Adds a circle or circular arc: the centre, the radius, and the start angle and signed sweep in
    /// radians.
    pub fn push_circle_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radius: f64,
        start_sweep: [f64; 2],
    ) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::CircleArc { centre, radius, start_sweep })
    }

    /// Adds an ellipse or elliptical arc: the centre, the two semi-axes, and the rotation, start angle
    /// and signed sweep in radians.
    pub fn push_ellipse_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radii: [f64; 2],
        angles: [f64; 3],
    ) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::EllipseArc { centre, radii, angles })
    }

    fn push_advanced(&mut self, meta: EntityMeta, shape: AdvancedShape) -> Result<(), WriteError> {
        let meta = ContainerWriter::checked_meta(meta)?;
        if meta.cell != 0 {
            return Err(WriteError::CellOnAdvanced { id: meta.id.index() });
        }
        let seq = self.next_seq();
        self.advanced.push(AdvancedItem { seq, meta, shape });
        Ok(())
    }

    /// Sets the size, in bytes of record and arena data, at which the advanced curves are cut into
    /// chunks (default [`ADVANCED_CHUNK_BYTES`]). A composite and every curve it names always share a
    /// chunk, so one component bigger than this is still one chunk.
    pub fn advanced_chunk_bytes(&mut self, bytes: u32) {
        self.advanced_chunk = bytes;
    }

    /// Adds a B-spline. Its cell must be 0: the advanced curves have no spatial cell.
    pub fn push_spline(&mut self, meta: EntityMeta, spec: SplineSpec<'_>) -> Result<(), WriteError> {
        let shape = AdvancedShape::Spline {
            degree: spec.degree,
            knots: spec.knots.to_vec(),
            x: spec.x.to_vec(),
            y: spec.y.to_vec(),
            weights: spec.weights.map(<[f64]>::to_vec),
            closed: spec.closed,
        };
        if spec.periodic {
            return Err(WriteError::Arena(ArenaError::PeriodicUnsupported));
        }
        self.push_advanced(meta, shape)
    }

    /// Adds a composite curve naming other advanced curves by id, with optional `(t1, t2)` trims.
    /// Its cell must be 0.
    pub fn push_composite(
        &mut self,
        meta: EntityMeta,
        segments: &[EntityId],
        trims: Option<&[[f64; 2]]>,
        closed: bool,
    ) -> Result<(), WriteError> {
        let shape = AdvancedShape::Composite {
            segments: segments.to_vec(),
            trims: trims.map(<[[f64; 2]]>::to_vec),
            closed,
        };
        self.push_advanced(meta, shape)
    }

    /// Adds a curve of the semantic model (P3.1).
    ///
    /// Polylines, B-splines, rational B-splines and Bezier curves are stored as they are; a trimmed
    /// line is the segment between its end points; a circle, an ellipse or an arc of either is stored
    /// whatever its frame. A rotated frame is stored with the angle of its x axis by
    /// `pvec_numeric::detmath::atan2` (within one ulp, the same on every machine) and listed by
    /// [`ContainerWriter::derived`]. What cannot be stored exactly and the same on every machine is
    /// refused, never approximated: an unbounded line, a composite (its segments need ids, so push
    /// them with [`ContainerWriter::push_composite`]) and a trimmed spline.
    ///
    /// The bounds are computed here from basic arithmetic only and are conservative: the box of the
    /// vertices, the control points, the end points, or, for a circle, an ellipse or any arc of
    /// either, the whole circle's or ellipse's box. The model's own bounds are not used because for an
    /// arc they go through `sin_cos` and `atan2`, which would put a libm result into the artifact.
    pub fn push_curve(
        &mut self,
        id: EntityId,
        style: u32,
        cell: u32,
        curve: &Curve,
    ) -> Result<(), WriteError> {
        let unsupported = |what| Err(WriteError::Unsupported { what });
        let meta = |bounds| EntityMeta { id, style, cell, bounds };
        match curve {
            Curve::Line(_) => unsupported("unbounded line"),
            Curve::Composite(_) => unsupported("composite curve"),
            Curve::Polyline(p) => {
                let points: Vec<[f64; 2]> = p.vertices().iter().map(|v| xy(*v)).collect();
                self.push_polyline(meta(boxed(&points)), &points)
            }
            Curve::BSpline(b) => {
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let spec = SplineSpec {
                    degree: b.degree(),
                    knots: b.knots(),
                    x: &x,
                    y: &y,
                    weights: None,
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta(hull(&x, &y)), spec)
            }
            Curve::RationalBSpline(b) => {
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let spec = SplineSpec {
                    degree: b.degree(),
                    knots: b.knots(),
                    x: &x,
                    y: &y,
                    weights: Some(b.weights()),
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta(hull(&x, &y)), spec)
            }
            Curve::Bezier(b) => {
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let degree = b.degree();
                let mut knots = vec![0.0; degree + 1];
                knots.extend(std::iter::repeat_n(1.0, degree + 1));
                let spec = SplineSpec {
                    degree,
                    knots: &knots,
                    x: &x,
                    y: &y,
                    weights: None,
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta(hull(&x, &y)), spec)
            }
            Curve::Circle(c) => {
                let (o, rot) = self.frame(id, c.placement())?;
                let (start, sweep) = shifted(rot, 0.0, core::f64::consts::TAU);
                self.push_circle_arc(meta(around(o, c.radius(), c.radius())), o, c.radius(), [start, sweep])
            }
            Curve::Ellipse(e) => {
                let (o, rot) = self.frame(id, e.placement())?;
                let radii = [e.semi_axis_1(), e.semi_axis_2()];
                let r = if rot == 0.0 { radii } else { [radii[0].max(radii[1]); 2] };
                self.push_ellipse_arc(
                    meta(around(o, r[0], r[1])),
                    o,
                    radii,
                    [rot, 0.0, core::f64::consts::TAU],
                )
            }
            Curve::Trimmed(t) => {
                let trim = t.trim();
                let (start, sweep) = if trim.reversed {
                    (trim.start + trim.span, -trim.span)
                } else {
                    (trim.start, trim.span)
                };
                match t.basis() {
                    Curve::Line(_) => {
                        let d = curve.domain();
                        let (a, b) = (xy(curve.eval(d.lo())?), xy(curve.eval(d.hi())?));
                        self.push_line(meta(boxed(&[a, b])), a, b)
                    }
                    Curve::Circle(c) => {
                        let (o, rot) = self.frame(id, c.placement())?;
                        let (start, sweep) = shifted(rot, start, sweep);
                        self.push_circle_arc(
                            meta(around(o, c.radius(), c.radius())),
                            o,
                            c.radius(),
                            [start, sweep],
                        )
                    }
                    Curve::Ellipse(e) => {
                        let (o, rot) = self.frame(id, e.placement())?;
                        let radii = [e.semi_axis_1(), e.semi_axis_2()];
                        let r = if rot == 0.0 { radii } else { [radii[0].max(radii[1]); 2] };
                        self.push_ellipse_arc(meta(around(o, r[0], r[1])), o, radii, [rot, start, sweep])
                    }
                    _ => unsupported("trimmed spline"),
                }
            }
            _ => unsupported("curve of a kind this writer does not know"),
        }
    }

    /// What the writer derived instead of being handed it: the rotation of each rotated frame, in the
    /// order the curves were pushed. A frame along x derives nothing.
    pub fn derived(&self) -> &[Derived] {
        &self.derived
    }

    /// The origin and the rotation of a frame. The rotation of a frame along x is exactly zero; any
    /// other is the angle of its axis by `detmath::atan2` (within one ulp), and is recorded.
    fn frame(&mut self, id: EntityId, p: &pvec_geometry::Placement2) -> Result<([f64; 2], f64), WriteError> {
        let x = p.x_axis();
        let rot = if x.x == 1.0 && x.y == 0.0 {
            0.0
        } else {
            let r = pvec_numeric::detmath::atan2(x.y, x.x);
            self.derived.push(Derived {
                entity: id,
                what: "the rotation of the frame: the angle of its x axis, by the deterministic arctangent (within one ulp)",
                bound: UNIT * 2.0 * r.abs(),
            });
            r
        };
        Ok((xy(p.origin()), rot))
    }

    /// The box this writer derived for each hot entity pushed so far (a segment, a polyline, a circle or
    /// an ellipse, or an arc of either), in push order. The advanced curves are not here. A spatial
    /// directory built over these boxes holds every hot block's bounds, which the directory requires
    /// (`SPD-10`), where a box computed from the curve model would not: the model's bounds for an arc
    /// are tighter than the conservative ones this writer stores.
    pub fn hot_boxes(&self) -> Vec<(EntityId, Bounds)> {
        self.hot.iter().map(|h| (h.meta.id, h.meta.bounds)).collect()
    }

    /// Adds a component stream. Its capability is declared in the manifest as the registry declares it.
    pub fn push_stream(&mut self, stream: Stream) -> Result<(), WriteError> {
        let seq = self.next_seq();
        self.streams.push((seq, stream));
        Ok(())
    }

    /// Sets the spatial directory block (`pvec-stream`'s `build_directory` makes it): `nodes` is its node
    /// count, which becomes the entry's count, `bounds` is the box of the whole tree and `bytes` is the
    /// block. A second call replaces the first. The block is optional (a reader that does not know it
    /// skips it) and is stored as it is, whatever codec the policy names. The writer does not parse it:
    /// that the cells the hot blocks name are the cells the directory has is checked by
    /// `SpatialDirectory::open`.
    pub fn set_spatial_directory(
        &mut self,
        nodes: u32,
        bounds: Bounds,
        bytes: Vec<u8>,
    ) -> Result<(), WriteError> {
        let bounds = checked_bounds(bounds).ok_or(WriteError::BadBounds { id: None })?;
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        let seq = self.next_seq();
        self.spatial = Some(SpatialItem { seq, nodes, bounds, bytes });
        Ok(())
    }

    /// Sets the reference block (`pvec-stream`'s `CellRefs::encode` makes it): `cells` is the number of
    /// cells that have references, which becomes the entry's count, and `bytes` is the block. A second
    /// call replaces the first. The block is optional, is stored as it is and is not a must-understand
    /// block, so a reader that does not know it skips it; it sorts after every other kind, so adding it
    /// moves no block id and the references it holds stay valid. The writer does not parse it: that the
    /// count is the cell count of the bytes is checked when `pvec-stream` reads it.
    pub fn set_cell_refs(&mut self, cells: u32, bytes: Vec<u8>) -> Result<(), WriteError> {
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        let seq = self.next_seq();
        self.cell_refs = Some(RefsItem { seq, cells, bytes });
        Ok(())
    }

    /// Sets the straddler lists block (`pvec-stream`'s `build_straddlers` makes it): `entries` is the
    /// number of listed entities, which becomes the entry's count, `bounds` is the box of all of them and
    /// `bytes` is the block. A second call replaces the first. Like the spatial directory it is optional
    /// (a reader that does not know it skips it) and stored as it is. The payloads it points at are hot
    /// blocks the caller pushed in cells at or above [`crate::directory::STRADDLER_CELL_FIRST`]; the
    /// writer does not parse the block, and `StraddlerIndex::open` checks the references.
    pub fn set_straddler_lists(
        &mut self,
        entries: u32,
        bounds: Bounds,
        bytes: Vec<u8>,
    ) -> Result<(), WriteError> {
        let bounds = checked_bounds(bounds).ok_or(WriteError::BadBounds { id: None })?;
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        let seq = self.next_seq();
        self.straddlers = Some(SpatialItem { seq, nodes: entries, bounds, bytes });
        Ok(())
    }

    /// Adds the Tier 0 block of one index leaf (`pvec-stream`'s `tier0_blocks` makes them): `cell` is the
    /// leaf id, `count` the entities it covers, `bounds` the leaf's box and `bytes` the block. Optional
    /// like the spatial directory, and the writer does not parse it; `Tier0Leaves` checks the block against
    /// the directory when a reader opens it. Cell 0, the straddler payload cells and a second block for one
    /// cell are refused (`WRT-14`).
    pub fn push_tier0_leaf(
        &mut self,
        cell: u32,
        count: u32,
        bounds: Bounds,
        bytes: Vec<u8>,
    ) -> Result<(), WriteError> {
        if cell == 0
            || cell >= crate::directory::STRADDLER_CELL_FIRST
            || self.tier0.iter().any(|t| t.cell == cell)
        {
            return Err(WriteError::Tier0Cell { cell });
        }
        let bounds = checked_bounds(bounds).ok_or(WriteError::BadBounds { id: None })?;
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        let seq = self.next_seq();
        self.tier0.push(Tier0Item { seq, cell, count, bounds, bytes });
        Ok(())
    }

    /// Sets the bytes at which a level of raster tiles is cut into chunks (default [`RASTER_CHUNK_BYTES`]). A
    /// tile is never split, so one longer than this is refused when the file is finished.
    pub fn raster_chunk_bytes(&mut self, bytes: u32) {
        self.raster_chunk = bytes;
    }

    /// Sets how many bytes of the coarsest levels the index blocks hold inline, over every raster (default
    /// [`RASTER_INLINE_BYTES`]). The levels are taken coarsest first, across the rasters, each raster's in order,
    /// while they fit; a level that does not fit stops that raster. Nothing about the order they were pushed in
    /// is used.
    pub fn raster_inline_bytes(&mut self, bytes: u64) {
        self.raster_inline = bytes;
    }

    /// Adds encoded tiles of raster `raster`, whose placement is in the raster stream pushed with
    /// [`ContainerWriter::push_stream`] (capability 21). Each tile carries its level, column and row and the
    /// bytes of its PNG, JPEG or WebP, which are stored as they are; a tile with no bytes is a tile that is not
    /// stored. The tiles of one raster can come in any number of calls and in any order, and the file does not
    /// depend on either. What the writer cannot store is refused by [`ContainerWriter::finish`]: tiles for a
    /// raster the stream does not place, a tile outside its level or given twice, a tile longer than a chunk.
    pub fn push_raster_tiles(&mut self, raster: RasterId, tiles: Vec<TileIn>) -> Result<(), WriteError> {
        self.rasters.entry(raster.index()).or_default().extend(tiles);
        Ok(())
    }

    /// The capabilities to declare: the caller's and those of the streams, each once, in id order.
    ///
    /// The minimum reader is the highest `min_reader` over the required capabilities only: an optional one is
    /// skippable by definition, and raising the version for it would make an older reader refuse the file at
    /// `NEGOTIATE-03` before it ever reached the skip path.
    fn declarations(&self, registry: &Registry) -> Result<(Vec<Declared>, Version), WriteError> {
        let mut wanted: Vec<CapabilityId> = Vec::new();
        for id in
            self.config.capabilities.iter().copied().chain(self.streams.iter().map(|(_, s)| s.capability))
        {
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
        if self.sorts(SortKey::CapabilityId) {
            wanted.sort_unstable();
        }
        let mut min_reader = Version::new(1, 0, 0);
        let mut out = Vec::with_capacity(wanted.len());
        for id in wanted {
            let c = registry.get(id).ok_or(WriteError::UnknownCapability { id: id.get() })?;
            if c.requirement == Requirement::Required {
                min_reader = min_reader.max(c.min_reader);
            }
            out.push(Declared { id, requirement: c.requirement, version: c.version });
        }
        Ok((out, min_reader))
    }

    fn hot_blocks(&self) -> Result<Vec<Planned>, WriteError> {
        let mut index: BTreeMap<(u32, u16), usize> = BTreeMap::new();
        let mut groups: Vec<(HotKind, u32, Vec<&HotItem>)> = Vec::new();
        for item in &self.hot {
            let (kind, key) = (item.shape.kind(), (item.meta.cell, item.shape.kind().id()));
            let at = match index.get(&key) {
                Some(at) => *at,
                None => {
                    groups.push((kind, item.meta.cell, Vec::new()));
                    index.insert(key, groups.len() - 1);
                    groups.len() - 1
                }
            };
            groups[at].2.push(item);
        }
        let mut out = Vec::with_capacity(groups.len());
        for (kind, cell, mut items) in groups {
            let first_seen = items.first().map_or(0, |i| i.seq);
            if self.sorts(SortKey::EntityId) {
                items.sort_by_key(|i| i.meta.id);
            }
            let mut builder = HotBlockBuilder::new(kind);
            for i in &items {
                let (id, style) = (i.meta.id, i.meta.style);
                match &i.shape {
                    HotShape::Line(a, b) => builder.push_line(id, style, *a, *b)?,
                    HotShape::Polyline(points) => builder.push_polyline(id, style, points)?,
                    HotShape::CircleArc { centre, radius, start_sweep } => {
                        builder.push_circle_arc(id, style, *centre, *radius, *start_sweep)?
                    }
                    HotShape::EllipseArc { centre, radii, angles } => {
                        builder.push_ellipse_arc(id, style, *centre, *radii, *angles)?
                    }
                }
            }
            let block =
                if self.sorts(SortKey::EntityId) { builder.finish()? } else { builder.finish_unsorted()? };
            let bounds = items.iter().map(|i| i.meta.bounds).reduce(union);
            out.push(Planned {
                first_seen,
                kind: BlockKind::Hot,
                cell,
                subtype: kind.id(),
                count: u32::try_from(items.len()).map_err(|_| WriteError::TooLarge)?,
                bounds,
                bytes: block.as_bytes().to_vec(),
            });
        }
        Ok(out)
    }

    /// The advanced curves as chunks. A composite and everything it names are one component and never
    /// split; components are ordered by their lowest id and packed into a chunk until it holds
    /// `advanced_chunk` bytes. Chunk `k` is a record block and five arenas in cell `k`, rows ascending by id.
    fn advanced_blocks(&self) -> Result<Vec<Planned>, WriteError> {
        if self.advanced.is_empty() {
            return Ok(Vec::new());
        }
        let mut items: Vec<&AdvancedItem> = self.advanced.iter().collect();
        items.sort_by_key(|i| i.meta.id);
        let at = |id: EntityId| items.binary_search_by_key(&id, |i| i.meta.id).ok();
        // union-find over the sorted items; the root of a component is its lowest index
        let mut parent: Vec<usize> = (0..items.len()).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for (i, item) in items.iter().enumerate() {
            if let AdvancedShape::Composite { segments, .. } = &item.shape {
                for s in segments.iter().filter_map(|s| at(*s)) {
                    let (a, b) = (root(&mut parent, i), root(&mut parent, s));
                    parent[a.max(b)] = a.min(b);
                }
            }
        }
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); items.len()];
        for i in 0..items.len() {
            let r = root(&mut parent, i);
            members[r].push(i);
        }
        let size = |i: usize| -> u64 {
            32 + match &items[i].shape {
                AdvancedShape::Spline { knots, x, weights, .. } => {
                    8 * (knots.len() + weights.as_ref().map_or(0, Vec::len)) as u64 + 16 * x.len() as u64
                }
                AdvancedShape::Composite { segments, trims, .. } => {
                    4 * segments.len() as u64 + 16 * trims.as_ref().map_or(0, Vec::len) as u64
                }
            }
        };
        let mut chunks: Vec<Vec<usize>> = Vec::new();
        let mut filled = 0u64;
        for component in members.iter().filter(|m| !m.is_empty()) {
            let bytes: u64 = component.iter().map(|i| size(*i)).sum();
            match chunks.last_mut() {
                Some(last) if filled + bytes <= u64::from(self.advanced_chunk) => {
                    last.extend(component);
                    filled += bytes;
                }
                _ => {
                    chunks.push(component.clone());
                    filled = bytes;
                }
            }
        }
        let mut out = Vec::with_capacity(6 * chunks.len());
        for (k, mut chunk) in chunks.into_iter().enumerate() {
            chunk.sort_unstable();
            let cell = u32::try_from(k).map_err(|_| WriteError::TooLarge)?;
            let mut builder = AdvancedBuilder::new();
            for i in &chunk {
                let (id, style) = (items[*i].meta.id, items[*i].meta.style);
                match &items[*i].shape {
                    AdvancedShape::Spline { degree, knots, x, y, weights, closed } => builder.push_spline(
                        id,
                        style,
                        SplineSpec {
                            degree: *degree,
                            knots,
                            x,
                            y,
                            weights: weights.as_deref(),
                            closed: *closed,
                            periodic: false,
                        },
                    )?,
                    AdvancedShape::Composite { segments, trims, closed } => {
                        builder.push_composite(id, style, segments, trims.as_deref(), *closed)?
                    }
                }
            }
            let blocks = builder.finish()?;
            let s = blocks.slices();
            let bounds = chunk.iter().map(|i| items[*i].meta.bounds).reduce(union);
            let first_seen = chunk.iter().map(|i| items[*i].seq).min().unwrap_or(0);
            let parts: [(BlockKind, u16, &[u8], Option<Bounds>); 6] = [
                (BlockKind::Records, 0, s.records, bounds),
                (BlockKind::Arena, PayloadKind::Knots.id(), s.knots, None),
                (BlockKind::Arena, PayloadKind::Weights.id(), s.weights, None),
                (BlockKind::Arena, PayloadKind::ControlPoints.id(), s.control_points, None),
                (BlockKind::Arena, PayloadKind::Segments.id(), s.segments, None),
                (BlockKind::Arena, PayloadKind::Trims.id(), s.trims, None),
            ];
            for (kind, subtype, bytes, bounds) in parts {
                // every block of the six starts with its magic, version and kind, then the entry count
                let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                out.push(Planned { first_seen, kind, cell, subtype, count, bounds, bytes: bytes.to_vec() });
            }
        }
        Ok(out)
    }

    fn stream_block(&self) -> Result<Option<Planned>, WriteError> {
        let Some((first_seen, _)) = self.streams.first() else {
            return Ok(None);
        };
        let mut seen: Vec<u32> = self.streams.iter().map(|(_, s)| s.capability.get()).collect();
        seen.sort_unstable();
        if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
            return Err(WriteError::DuplicateStream { capability: w[0] });
        }
        let mut streams: Vec<Stream> = self.streams.iter().map(|(_, s)| s.clone()).collect();
        if self.sorts(SortKey::StreamId) {
            streams.sort_by_key(|s| s.capability);
        }
        let bytes = encode_streams(&streams)?;
        let count = u32::try_from(streams.len()).map_err(|_| WriteError::TooLarge)?;
        Ok(Some(Planned {
            first_seen: *first_seen,
            kind: BlockKind::Components,
            cell: 0,
            subtype: 0,
            count,
            bounds: None,
            bytes,
        }))
    }

    /// The kind 9 and kind 10 blocks of the rasters pushed, in the canonical order: every index by raster, then
    /// every chunk by raster and subtype. They are codec 0 and optional, name capability 21, and carry the
    /// footprint of the visible box (index) or of the tiles of the chunk (chunk) in source space.
    fn raster_blocks(&mut self) -> Result<Vec<RawBlock>, WriteError> {
        let rasters = std::mem::take(&mut self.rasters);
        let Some(&first) = rasters.keys().next() else { return Ok(Vec::new()) };
        let stream = self
            .streams
            .iter()
            .map(|(_, s)| s)
            .find(|s| s.capability == RASTER_CAPABILITY)
            .ok_or(WriteError::Raster(IndexError::NoIndex { raster: first }))?;
        let placements = decode_rasters(&stream.payload).map_err(WriteError::Placement)?;
        let mut plans: Vec<(u32, &Placement, Vec<(u32, u32)>)> = Vec::with_capacity(rasters.len());
        for &id in rasters.keys() {
            let p =
                placements.get(id as usize).ok_or(WriteError::Raster(IndexError::NoIndex { raster: id }))?;
            let n = u32::from(p.levels);
            let grid =
                (0..n).map(|l| raster::grid_of(p.width, p.height, u32::from(p.tile_edge), n, l)).collect();
            plans.push((id, p, grid));
        }
        // LIMIT-14 over the artifact, at the default ceiling: the entries of every tile table
        let mut left = u64::from(Limits::DEFAULT.max_raster_tiles);
        // which levels are inline: the coarsest first across the rasters, each raster's in order, while they fit
        let mut inline: BTreeMap<u32, u32> = plans.iter().map(|(id, _, _)| (*id, 0)).collect();
        let mut candidates: Vec<(u32, u32, u64)> = Vec::new();
        for (id, _, grid) in &plans {
            let mut bytes = vec![0u64; grid.len()];
            for t in rasters.get(id).into_iter().flatten() {
                if let Some(b) = bytes.get_mut(t.level as usize) {
                    *b += t.bytes.len() as u64;
                }
            }
            candidates.extend(bytes.into_iter().enumerate().map(|(l, b)| (l as u32, *id, b)));
        }
        candidates.sort_unstable();
        let (mut budget, mut closed) = (self.raster_inline, Vec::<u32>::new());
        for (level, id, bytes) in candidates {
            if closed.contains(&id) || inline.get(&id) != Some(&level) {
                continue;
            }
            if bytes <= budget {
                budget -= bytes;
                inline.insert(id, level + 1);
            } else {
                closed.push(id);
            }
        }
        let mut indexes = Vec::new();
        let mut chunks = Vec::new();
        let mut rasters = rasters;
        for (id, p, grid) in plans {
            let tiles = rasters.remove(&id).unwrap_or_default();
            let total: u64 = grid.iter().map(|(c, r)| u64::from(*c) * u64::from(*r)).sum();
            let built =
                raster::build(&grid, tiles, inline.get(&id).copied().unwrap_or(0), self.raster_chunk, left)
                    .map_err(WriteError::Raster)?;
            left -= total;
            let footprint = pixel_box(p, p.visible_box.map(u64::from))?;
            let encoded = built.index.encode();
            indexes.push(raster_block(raster::KIND_INDEX, id, 0, built.index.stored(), footprint, encoded)?);
            for c in built.chunks {
                // a chunk's tiles are cells of its level, each `tile_edge` pixels of the level wide, and a pixel of
                // the level is `2^(levels - 1 - level)` pixels of the image
                let scale = 1u64 << (u32::from(p.levels) - 1 - c.level);
                let edge = u64::from(p.tile_edge) * scale;
                let area = [
                    u64::from(c.cols[0]) * edge,
                    u64::from(c.rows[0]) * edge,
                    ((u64::from(c.cols[1]) + 1) * edge).min(u64::from(p.width)),
                    ((u64::from(c.rows[1]) + 1) * edge).min(u64::from(p.height)),
                ];
                let st = raster::subtype(c.level, c.chunk).ok_or(WriteError::TooLarge)?;
                chunks.push(raster_block(raster::KIND_TILES, id, st, c.count, pixel_box(p, area)?, c.bytes)?);
            }
        }
        indexes.extend(chunks);
        indexes.sort_by_key(|b| (b.kind, b.cell, b.subtype));
        Ok(indexes)
    }

    /// The artifact.
    ///
    /// Header, manifest, the blocks in canonical order each on a multiple of 8 bytes, then the block
    /// directory, and nothing else. Every padding byte is zero.
    pub fn finish(mut self) -> Result<Vec<u8>, WriteError> {
        if self.hot.is_empty() && self.advanced.is_empty() {
            return Err(WriteError::Empty);
        }
        let mut ids: Vec<u32> = self
            .hot
            .iter()
            .map(|i| i.meta.id.index())
            .chain(self.advanced.iter().map(|i| i.meta.id.index()))
            .collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(WriteError::DuplicateId { id: w[0] });
        }
        let (capabilities, min_reader) = self.declarations(&Registry::v1())?;
        let source_bounds = match self.config.bounds {
            Some(b) => checked_bounds(b).ok_or(WriteError::BadBounds { id: None })?,
            None => self
                .hot
                .iter()
                .map(|i| i.meta.bounds)
                .chain(self.advanced.iter().map(|i| i.meta.bounds))
                .reduce(union)
                .ok_or(WriteError::Empty)?,
        };

        let mut blocks = self.hot_blocks()?;
        blocks.extend(self.advanced_blocks()?);
        blocks.extend(self.stream_block()?);
        if let Some(s) = &self.spatial {
            blocks.push(Planned {
                first_seen: s.seq,
                kind: BlockKind::SpatialDirectory,
                cell: 0,
                subtype: 0,
                count: s.nodes,
                bounds: Some(s.bounds),
                bytes: s.bytes.clone(),
            });
        }
        if let Some(r) = &self.cell_refs {
            blocks.push(Planned {
                first_seen: r.seq,
                kind: BlockKind::CellRefs,
                cell: 0,
                subtype: 0,
                count: r.cells,
                bounds: None,
                bytes: r.bytes.clone(),
            });
        }
        if let Some(s) = &self.straddlers {
            blocks.push(Planned {
                first_seen: s.seq,
                kind: BlockKind::Straddlers,
                cell: 0,
                subtype: 0,
                count: s.nodes,
                bounds: Some(s.bounds),
                bytes: s.bytes.clone(),
            });
        }
        for t in &self.tier0 {
            blocks.push(Planned {
                first_seen: t.seq,
                kind: BlockKind::Tier0,
                cell: t.cell,
                subtype: 0,
                count: t.count,
                bounds: Some(t.bounds),
                bytes: t.bytes.clone(),
            });
        }
        // arrival order first, so that a key that is switched off falls back to it and nothing else
        blocks.sort_by_key(|b| b.first_seen);
        let by = |key: SortKey, a: Ordering| if self.sorts(key) { a } else { Ordering::Equal };
        blocks.sort_by(|a, b| {
            by(SortKey::BlockKind, BlockKind::group(a.kind.id()).cmp(&BlockKind::group(b.kind.id())))
                .then(by(SortKey::SpatialCell, a.cell.cmp(&b.cell)))
                .then(by(SortKey::BlockKind, a.kind.id().cmp(&b.kind.id())))
                .then(by(SortKey::GeometryType, a.subtype.cmp(&b.subtype)))
        });

        let policy = self.options.codecs;
        let mut raw: Vec<RawBlock> = Vec::with_capacity(blocks.len());
        for b in blocks {
            let uncompressed_length = u32::try_from(b.bytes.len()).map_err(|_| WriteError::TooLarge)?;
            // a block is stored compressed only if that is smaller, and an uncompressed block is legal
            let (codec, stored) = pack(policy.for_kind(b.kind), b.bytes);
            let mut flags = if matches!(
                b.kind,
                BlockKind::Components
                    | BlockKind::SpatialDirectory
                    | BlockKind::CellRefs
                    | BlockKind::Straddlers
                    | BlockKind::Tier0
            ) {
                0
            } else {
                FLAG_MUST_UNDERSTAND
            };
            if b.bounds.is_some() {
                flags |= FLAG_HAS_BOUNDS;
            }
            raw.push(RawBlock {
                kind: b.kind.id(),
                subtype: b.subtype,
                cell: b.cell,
                count: b.count,
                bounds: b.bounds,
                flags,
                codec: codec.id(),
                capability: 0,
                uncompressed_length,
                stored,
            });
        }
        // kinds 9 and 10 sort after every kind the format defines, so they close the directory in their own order
        raw.extend(self.raster_blocks()?);
        let manifest = Manifest {
            unit: self.config.unit,
            bounds: source_bounds,
            min_reader,
            capabilities,
            directories: Vec::new(),
            block_checksums: None,
            artifact_digest: None,
            metadata: self.config.metadata.clone(),
        };
        assemble(manifest, &raw, self.options.integrity)
    }
}

/// The box in source space of a rectangle of pixels `[x0 y0 x1 y1]` of a raster, from its four corners through the
/// placement's map.
fn pixel_box(p: &Placement, rect: [u64; 4]) -> Result<Bounds, WriteError> {
    let [a, b, c, d, tx, ty] = p.pixel_to_source;
    let corners = [(rect[0], rect[1]), (rect[2], rect[1]), (rect[2], rect[3]), (rect[0], rect[3])];
    let points: Vec<[f64; 2]> = corners
        .iter()
        .map(|&(x, y)| {
            let (x, y) = (x as f64, y as f64);
            [normal(a * x + b * y + tx), normal(c * x + d * y + ty)]
        })
        .collect();
    checked_bounds(boxed(&points)).ok_or(WriteError::BadBounds { id: None })
}

/// One raster block: optional, optional-codec-free (codec 0), owned by capability 21, with its footprint.
fn raster_block(
    kind: u16,
    raster: u32,
    subtype: u16,
    count: u32,
    bounds: Bounds,
    stored: Vec<u8>,
) -> Result<RawBlock, WriteError> {
    let uncompressed_length = u32::try_from(stored.len()).map_err(|_| WriteError::TooLarge)?;
    Ok(RawBlock {
        kind,
        subtype,
        cell: raster,
        count,
        bounds: Some(bounds),
        flags: FLAG_HAS_BOUNDS,
        codec: crate::directory::CODEC_NONE,
        capability: raster::CAPABILITY,
        uncompressed_length,
        stored,
    })
}

/// The start and sweep of an arc of a circle whose frame is turned by `rot`: the start moves by the
/// rotation and is brought back into `[0, tau)`. A frame along x leaves both as they are.
fn shifted(rot: f64, start: f64, sweep: f64) -> (f64, f64) {
    if rot == 0.0 {
        return (start, sweep);
    }
    let mut s = (rot + start).rem_euclid(core::f64::consts::TAU);
    if s >= core::f64::consts::TAU {
        s = 0.0;
    }
    (s, sweep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvec_capabilities::Capability;

    /// The v1 registry with the line capability's `min_reader` raised and its requirement set, so the hoist
    /// can be seen on a required and on an optional capability at a higher version.
    fn registry_with(line: Requirement, min_reader: Version) -> Registry {
        let v1 = Registry::v1();
        let entries: Vec<Capability> = v1
            .entries()
            .iter()
            .cloned()
            .map(|mut c| {
                if c.id == pvec_capabilities::ids::LINE {
                    c.requirement = line;
                    c.min_reader = min_reader;
                }
                c
            })
            .collect();
        Registry::new(entries, v1.retired().to_vec()).expect("a valid registry")
    }

    fn declared_min_reader(registry: &Registry) -> Version {
        let config = WriterConfig {
            unit: LengthUnit::Millimetre,
            metadata: Metadata {
                producer_id: "test".to_string(),
                producer_version: "0".to_string(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: vec![pvec_capabilities::ids::LINE],
        };
        ContainerWriter::new(config).declarations(registry).expect("declares").1
    }

    #[test]
    fn a_required_capability_at_a_higher_version_raises_the_minimum_reader() {
        let r = registry_with(Requirement::Required, Version::new(1, 1, 0));
        assert_eq!(declared_min_reader(&r), Version::new(1, 1, 0));
    }

    #[test]
    fn an_optional_capability_at_a_higher_version_leaves_the_minimum_reader_at_1_0_0() {
        let r = registry_with(Requirement::Optional, Version::new(1, 1, 0));
        assert_eq!(declared_min_reader(&r), Version::new(1, 0, 0));
    }
}
