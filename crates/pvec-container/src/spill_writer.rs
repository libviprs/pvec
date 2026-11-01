//! The container writer that keeps its pushes in files (issue pvec#235).
//!
//! [`SpillWriter`] takes the same pushes as `writer::ContainerWriter` and writes the same artifact, but it
//! never holds more than a budget of them: hot entities, advanced curves, component streams and raster tiles
//! go to files in a [`Store`] the moment they are pushed, the hot entities are put in their blocks by an
//! external sort, every finished block goes to a block spool, and [`SpillWriter::finish_to`] lays the artifact
//! out from the spool onto any `Write`, with no `Seek`. What stays in memory is the bookkeeping that cannot be
//! avoided: a small record for each advanced curve (their chunks are planned from the whole set), a
//! directory entry for each block, a tile table for each raster, and, while a block is built, the entities of
//! that block. `docs/CONTAINER.md` chapter 4 has the bounds.
//!
//! The artifact is the one the in-memory writer makes for the same pushes. Both use the same functions to
//! decide what a curve becomes (`plan_curve`), to cut the advanced curves into chunks (`plan_chunks`), to
//! build a block, to compress it (`pack_planned`), to lay the file out (`plan_layout`) and to cut the rasters
//! (`raster_blocks_of`); what differs is where the pushes are kept and how the bytes reach the sink. A test
//! holds the two to each other on the whole corpus.
//!
//! An artifact whose build fails writes nothing to the sink: every block is built and every rule is checked
//! before the first byte is written, and the files are removed whether the build succeeded, failed or was
//! dropped.

use crate::blocks::advanced::SplineSpec;
use crate::blocks::hot::HotKind;
use crate::blocks::raster::{TileIn, TileMeta};
use crate::compress::CodecPolicy;
use crate::directory::BlockKind;
use crate::integrity::{DigestStream, Integrity, block_checksum};
use crate::manifest::{Bounds, Manifest};
use crate::spill::{Appender, ExternalSorter, Log, SpillError, SpillResult, Store};
use crate::writer::{
    AdvancedItem, AdvancedShape, BlockMeta, Derived, EntityMeta, HotGroup, HotItem, HotShape, Item, Planned, RasterFail,
    RasterRules, RawBlock, WriteError, WriterConfig, WriterOptions, advanced_chunk, advanced_size,
    checked_bounds, declare, hot_block, pack_planned, plan_chunks, plan_curve, plan_layout, raster_blocks_of,
    stream_block_of, union,
};
use crate::header::Header;
use crate::spec::m::ARTIFACT_DIGEST;
use pvec_capabilities::{CapabilityId, Registry, Version};
use pvec_components::{RASTER_CAPABILITY, RasterId, Stream};
use pvec_entity::EntityId;
use pvec_geometry::Curve;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

/// Why a push or the finish failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum FinishError {
    /// The container refuses the input (the rule is the writer's, as for the in-memory writer).
    Write(WriteError),
    /// A spill file could not be written or read.
    Spill(SpillError),
    /// The sink the artifact is written to failed.
    Output(SpillError),
}

impl fmt::Display for FinishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FinishError::Write(e) => write!(f, "{e}"),
            FinishError::Spill(e) => write!(f, "spill file: {e}"),
            FinishError::Output(e) => write!(f, "output: {e}"),
        }
    }
}

impl std::error::Error for FinishError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FinishError::Write(e) => Some(e),
            FinishError::Spill(e) | FinishError::Output(e) => Some(e),
        }
    }
}

impl From<WriteError> for FinishError {
    fn from(e: WriteError) -> FinishError {
        FinishError::Write(e)
    }
}

impl From<SpillError> for FinishError {
    /// Every store error that reaches a `?` inside the writer is a spill file's; the sink's are tagged where
    /// they are raised.
    fn from(e: SpillError) -> FinishError {
        FinishError::Spill(e)
    }
}

/// Where the artifact is written: a sink that takes bytes in order and needs no seek.
pub trait ByteSink {
    /// Takes the next bytes of the artifact.
    fn put(&mut self, bytes: &[u8]) -> Result<(), SpillError>;
}

// ----------------------------------------------------------------------------------- records in files

fn corrupt(what: &'static str) -> SpillError {
    SpillError::corrupt(what)
}

/// A reader over the bytes of one record, which refuses a record that ends early.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> SpillResult<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.bytes.len()).ok_or_else(|| corrupt("a record ends early"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }
    fn u8(&mut self) -> SpillResult<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> SpillResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f64(&mut self) -> SpillResult<f64> {
        let b = self.take(8)?;
        Ok(f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
    }
    fn f64s<const N: usize>(&mut self) -> SpillResult<[f64; N]> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = self.f64()?;
        }
        Ok(out)
    }
    /// A count that the rest of the record can hold at `each` bytes apiece.
    fn count(&mut self, each: usize) -> SpillResult<usize> {
        let n = self.u32()? as usize;
        if n.checked_mul(each).is_none_or(|b| b > self.bytes.len() - self.at) {
            return Err(corrupt("a count is longer than its record"));
        }
        Ok(n)
    }
    fn f64_vec(&mut self, n: usize) -> SpillResult<Vec<f64>> {
        (0..n).map(|_| self.f64()).collect()
    }
    fn done(&self) -> SpillResult<()> {
        if self.at == self.bytes.len() { Ok(()) } else { Err(corrupt("a record has bytes left over")) }
    }
}

fn put_f64s(out: &mut Vec<u8>, values: &[f64]) {
    for v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn put_meta(out: &mut Vec<u8>, m: &EntityMeta) {
    out.extend_from_slice(&m.id.index().to_le_bytes());
    out.extend_from_slice(&m.style.to_le_bytes());
    out.extend_from_slice(&m.cell.to_le_bytes());
    put_f64s(out, &[m.bounds.min[0], m.bounds.min[1], m.bounds.max[0], m.bounds.max[1]]);
}

fn get_meta(c: &mut Cursor<'_>) -> SpillResult<EntityMeta> {
    let id = EntityId::new(c.u32()?).map_err(|_| corrupt("an entity id out of range"))?;
    let (style, cell) = (c.u32()?, c.u32()?);
    let b = c.f64s::<4>()?;
    Ok(EntityMeta { id, style, cell, bounds: Bounds { min: [b[0], b[1]], max: [b[2], b[3]] } })
}

/// A hot entity as the spill keeps it: what the container needs to put it in its block, and `tag`, bytes the
/// caller attached to it that come back untouched (the facade keeps the entity's Tier 0 entry there).
#[derive(Clone, Debug)]
pub struct HotRecord {
    pub(crate) item: HotItem,
    /// What the caller attached to the entity.
    pub tag: Vec<u8>,
}

impl HotRecord {
    /// The entity, its style, its cell and its bounds.
    pub fn meta(&self) -> &EntityMeta {
        &self.item.meta
    }

    /// The same entity in another cell.
    pub fn in_cell(mut self, cell: u32) -> HotRecord {
        self.item.meta.cell = cell;
        self
    }

    /// The record as bytes, for a file or a sorter.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(96 + self.tag.len());
        put_meta(&mut out, &self.item.meta);
        match &self.item.shape {
            HotShape::Line(a, b) => {
                out.push(0);
                put_f64s(&mut out, &[a[0], a[1], b[0], b[1]]);
            }
            HotShape::Polyline(points) => {
                out.push(1);
                out.extend_from_slice(&(points.len() as u32).to_le_bytes());
                for p in points {
                    put_f64s(&mut out, p);
                }
            }
            HotShape::CircleArc { centre, radius, start_sweep } => {
                out.push(2);
                put_f64s(&mut out, &[centre[0], centre[1], *radius, start_sweep[0], start_sweep[1]]);
            }
            HotShape::EllipseArc { centre, radii, angles } => {
                out.push(3);
                put_f64s(&mut out, &[centre[0], centre[1], radii[0], radii[1], angles[0], angles[1], angles[2]]);
            }
        }
        out.extend_from_slice(&(self.tag.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.tag);
        out
    }

    /// A record from its bytes; an `InvalidData` error for bytes `encode` could not have made.
    pub fn decode(bytes: &[u8]) -> SpillResult<HotRecord> {
        let mut c = Cursor { bytes, at: 0 };
        let meta = get_meta(&mut c)?;
        let shape = match c.u8()? {
            0 => {
                let v = c.f64s::<4>()?;
                HotShape::Line([v[0], v[1]], [v[2], v[3]])
            }
            1 => {
                let n = c.count(16)?;
                let mut points = Vec::with_capacity(n);
                for _ in 0..n {
                    points.push(c.f64s::<2>()?);
                }
                HotShape::Polyline(points)
            }
            2 => {
                let v = c.f64s::<5>()?;
                HotShape::CircleArc { centre: [v[0], v[1]], radius: v[2], start_sweep: [v[3], v[4]] }
            }
            3 => {
                let v = c.f64s::<7>()?;
                HotShape::EllipseArc { centre: [v[0], v[1]], radii: [v[2], v[3]], angles: [v[4], v[5], v[6]] }
            }
            _ => return Err(corrupt("an unknown shape")),
        };
        let n = c.count(1)?;
        let tag = c.take(n)?.to_vec();
        c.done()?;
        Ok(HotRecord { item: HotItem { seq: 0, meta, shape }, tag })
    }
}

fn encode_advanced(item: &AdvancedItem) -> Vec<u8> {
    let mut out = Vec::new();
    put_meta(&mut out, &item.meta);
    match &item.shape {
        AdvancedShape::Spline { degree, knots, x, y, weights, closed } => {
            out.push(0);
            out.extend_from_slice(&(*degree as u32).to_le_bytes());
            out.push(u8::from(*closed));
            out.extend_from_slice(&(knots.len() as u32).to_le_bytes());
            put_f64s(&mut out, knots);
            out.extend_from_slice(&(x.len() as u32).to_le_bytes());
            put_f64s(&mut out, x);
            put_f64s(&mut out, y);
            match weights {
                Some(w) => {
                    out.push(1);
                    put_f64s(&mut out, w);
                }
                None => out.push(0),
            }
        }
        AdvancedShape::Composite { segments, trims, closed } => {
            out.push(1);
            out.push(u8::from(*closed));
            out.extend_from_slice(&(segments.len() as u32).to_le_bytes());
            for s in segments {
                out.extend_from_slice(&s.index().to_le_bytes());
            }
            match trims {
                Some(t) => {
                    out.push(1);
                    out.extend_from_slice(&(t.len() as u32).to_le_bytes());
                    for p in t {
                        put_f64s(&mut out, p);
                    }
                }
                None => out.push(0),
            }
        }
    }
    out
}

fn decode_advanced(bytes: &[u8]) -> SpillResult<AdvancedItem> {
    let mut c = Cursor { bytes, at: 0 };
    let meta = get_meta(&mut c)?;
    let shape = match c.u8()? {
        0 => {
            let degree = c.u32()? as usize;
            let closed = c.u8()? != 0;
            let nk = c.count(8)?;
            let knots = c.f64_vec(nk)?;
            let n = c.count(16)?;
            let x = c.f64_vec(n)?;
            let y = c.f64_vec(n)?;
            let weights = if c.u8()? != 0 { Some(c.f64_vec(n)?) } else { None };
            AdvancedShape::Spline { degree, knots, x, y, weights, closed }
        }
        1 => {
            let closed = c.u8()? != 0;
            let n = c.count(4)?;
            let mut segments = Vec::with_capacity(n);
            for _ in 0..n {
                segments.push(EntityId::new(c.u32()?).map_err(|_| corrupt("an entity id out of range"))?);
            }
            let trims = if c.u8()? != 0 {
                let t = c.count(16)?;
                let mut v = Vec::with_capacity(t);
                for _ in 0..t {
                    v.push(c.f64s::<2>()?);
                }
                Some(v)
            } else {
                None
            };
            AdvancedShape::Composite { segments, trims, closed }
        }
        _ => return Err(corrupt("an unknown advanced shape")),
    };
    c.done()?;
    Ok(AdvancedItem { seq: 0, meta, shape })
}

fn encode_stream(s: &Stream) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + s.payload.len());
    out.extend_from_slice(&s.capability.get().to_le_bytes());
    for v in [s.version.major, s.version.minor, s.version.patch] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&s.payload);
    out
}

fn decode_stream(bytes: &[u8]) -> SpillResult<Stream> {
    let mut c = Cursor { bytes, at: 0 };
    let capability = CapabilityId::new(c.u32()?);
    let mut v = [0u16; 3];
    for x in &mut v {
        let b = c.take(2)?;
        *x = u16::from_le_bytes([b[0], b[1]]);
    }
    let payload = c.take(bytes.len() - c.at)?.to_vec();
    Ok(Stream { capability, version: Version::new(v[0], v[1], v[2]), payload })
}

// ------------------------------------------------------------------------------------- the block spool

/// A block that is in the spool: how the directory describes it, and where its bytes are.
#[derive(Clone, Copy, Debug)]
struct Spooled {
    /// The canonical order: group, cell, kind, subtype.
    order: (u16, u32, u16, u16),
    meta: BlockMeta,
    offset: u64,
    checksum: u64,
}

/// The finished blocks, as stored, in a file, with what the layout needs to know about each.
#[derive(Debug)]
struct BlockSpool {
    store: Store,
    out: Appender,
    blocks: Vec<Spooled>,
}

impl BlockSpool {
    fn create(store: &Store) -> SpillResult<BlockSpool> {
        Ok(BlockSpool { store: Arc::clone(store), out: Appender::create(store)?, blocks: Vec::new() })
    }

    fn push(&mut self, raw: RawBlock) -> Result<(), FinishError> {
        let meta = BlockMeta::of(&raw)?;
        let order = (BlockKind::group(raw.kind), raw.cell, raw.kind, raw.subtype);
        let offset = self.out.end();
        self.out.write(&raw.stored)?;
        self.blocks.push(Spooled { order, meta, offset, checksum: block_checksum(&raw.stored) });
        Ok(())
    }

    fn push_planned(&mut self, policy: CodecPolicy, planned: Planned) -> Result<(), FinishError> {
        self.push(pack_planned(policy, planned)?)
    }
}

/// Where the bytes of an artifact go while it is laid out, twice when it carries a digest: once to a hash
/// (the digest is in the manifest, which comes first) and once to the sink.
trait Out {
    fn put(&mut self, bytes: &[u8]) -> Result<(), FinishError>;
}

struct Hash(DigestStream);

impl Out for Hash {
    fn put(&mut self, bytes: &[u8]) -> Result<(), FinishError> {
        self.0.update(bytes);
        Ok(())
    }
}

struct Sink<'a>(&'a mut dyn ByteSink);

impl Out for Sink<'_> {
    fn put(&mut self, bytes: &[u8]) -> Result<(), FinishError> {
        self.0.put(bytes).map_err(FinishError::Output)
    }
}

fn zeros(out: &mut dyn Out, mut n: u64) -> Result<(), FinishError> {
    const Z: [u8; 4096] = [0; 4096];
    while n > 0 {
        let k = n.min(Z.len() as u64) as usize;
        out.put(&Z[..k])?;
        n -= k as u64;
    }
    Ok(())
}

impl BlockSpool {
    /// Lays the artifact out onto `sink` and returns its length. Blocks go in the canonical order, each on a
    /// multiple of 8, so the output is what `writer::assemble` gives for the same blocks.
    fn assemble_to(
        &mut self,
        manifest: Manifest,
        integrity: Integrity,
        sink: &mut dyn ByteSink,
    ) -> Result<u64, FinishError> {
        self.out.flush()?;
        // the canonical order; ties cannot happen (a cell holds one block of a kind and subtype), and the
        // sort is stable anyway
        self.blocks.sort_by_key(|b| b.order);
        let metas: Vec<BlockMeta> = self.blocks.iter().map(|b| b.meta).collect();
        let lay = plan_layout(manifest, &metas, integrity)?;
        let digest_at = (Header::LEN + ARTIFACT_DIGEST) as u64;
        let mut manifest_bytes = lay.manifest_bytes.clone();
        if integrity == Integrity::Full {
            let mut hash = Hash(DigestStream::new(digest_at));
            self.emit(&lay, &manifest_bytes, &mut hash)?;
            let digest = hash.0.finalize();
            let at = ARTIFACT_DIGEST;
            manifest_bytes[at..at + 32].copy_from_slice(&digest);
        }
        self.emit(&lay, &manifest_bytes, &mut Sink(sink))
    }

    fn emit(&self, lay: &crate::writer::Layout, manifest_bytes: &[u8], out: &mut dyn Out) -> Result<u64, FinishError> {
        let mut at = 0u64;
        out.put(&lay.header.encode())?;
        out.put(manifest_bytes)?;
        at += Header::LEN as u64 + manifest_bytes.len() as u64;
        let mut buffer = vec![0u8; 64 * 1024];
        for (b, e) in self.blocks.iter().zip(&lay.entries) {
            zeros(out, e.offset - at)?;
            let (mut from, mut left) = (b.offset, u64::from(b.meta.stored_length));
            while left > 0 {
                let want = left.min(buffer.len() as u64) as usize;
                let n = self.store.read_at(self.out.file(), from, &mut buffer[..want])?;
                if n == 0 {
                    return Err(SpillError::corrupt("the block spool ends early").into());
                }
                out.put(&buffer[..n])?;
                from += n as u64;
                left -= n as u64;
            }
            at = e.offset + u64::from(b.meta.stored_length);
        }
        if lay.sealed {
            zeros(out, lay.table_offset - at)?;
            for b in &self.blocks {
                out.put(&b.checksum.to_le_bytes())?;
            }
            at = lay.table_offset + 8 * self.blocks.len() as u64;
        }
        zeros(out, lay.directory_offset - at)?;
        for e in &lay.entries {
            out.put(&e.encode())?;
        }
        at = lay.directory_offset + (lay.entries.len() * crate::directory::ENTRY_LEN) as u64;
        debug_assert_eq!(at, lay.total);
        Ok(at)
    }
}

// ------------------------------------------------------------------------------------- the writer

/// What the ids of everything pushed say, which the Tier 0 coverage rule and the duplicate rule need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdSummary {
    /// Ids pushed.
    pub count: u64,
    /// The highest id.
    pub max: Option<u32>,
    /// The lowest id that is below the highest and was not pushed.
    pub first_gap: Option<u32>,
    /// The lowest id that was pushed twice.
    pub duplicate: Option<u32>,
}

/// The skeleton of an advanced curve that stays in memory: enough to plan the chunks.
#[derive(Debug)]
struct AdvancedNote {
    id: EntityId,
    offset: u64,
    size: u64,
    segments: Vec<EntityId>,
}

/// Where a raster tile is in the tile log.
#[derive(Debug)]
struct TileNote {
    meta: TileMeta,
    offset: u64,
}

/// The writer. See the module.
#[derive(Debug)]
pub struct SpillWriter {
    config: WriterConfig,
    options: WriterOptions,
    store: Store,
    budget: usize,
    hot: Log,
    hot_count: u64,
    hot_emitted: bool,
    advanced: Log,
    notes: Vec<AdvancedNote>,
    advanced_chunk: u32,
    streams: Log,
    ids: Option<ExternalSorter>,
    summary: Option<IdSummary>,
    extent: Option<Bounds>,
    spatial: Option<Planned>,
    cell_refs: Option<Planned>,
    straddlers: Option<Planned>,
    tier0_cells: BTreeSet<u32>,
    spool: BlockSpool,
    tiles: Log,
    rasters: BTreeMap<u32, Vec<TileNote>>,
    raster_chunk: u32,
    raster_inline: u64,
    sniff_tiles: bool,
    derived: Vec<Derived>,
}

impl SpillWriter {
    /// A writer that keeps its files in `store` and sorts with at most about `budget` bytes of records in memory.
    pub fn new(config: WriterConfig, store: Store, budget: usize) -> SpillResult<SpillWriter> {
        SpillWriter::with_options(config, WriterOptions::default(), store, budget)
    }

    /// A writer that compresses and seals as `options` say.
    pub fn with_options(
        config: WriterConfig,
        options: WriterOptions,
        store: Store,
        budget: usize,
    ) -> SpillResult<SpillWriter> {
        Ok(SpillWriter {
            config,
            options,
            hot: Log::create(&store)?,
            hot_count: 0,
            hot_emitted: false,
            advanced: Log::create(&store)?,
            notes: Vec::new(),
            advanced_chunk: crate::writer::ADVANCED_CHUNK_BYTES,
            streams: Log::create(&store)?,
            ids: Some(ExternalSorter::new(&store, budget / 4)),
            summary: None,
            extent: None,
            spatial: None,
            cell_refs: None,
            straddlers: None,
            tier0_cells: BTreeSet::new(),
            spool: BlockSpool::create(&store)?,
            tiles: Log::create(&store)?,
            rasters: BTreeMap::new(),
            raster_chunk: crate::writer::RASTER_CHUNK_BYTES,
            raster_inline: crate::writer::RASTER_INLINE_BYTES,
            sniff_tiles: true,
            derived: Vec::new(),
            store,
            budget,
        })
    }

    /// The store, for a sorter of the caller's that should share its files.
    pub fn store(&self) -> Store {
        Arc::clone(&self.store)
    }

    /// The budget, in bytes of records held for sorting.
    pub fn budget(&self) -> usize {
        self.budget
    }

    /// Hot entities pushed so far.
    pub fn hot_count(&self) -> u64 {
        self.hot_count
    }

    /// Switches the check of tile headers off (the hostile-tile corpus needs it).
    #[doc(hidden)]
    pub fn skip_tile_checks(&mut self) {
        self.sniff_tiles = false;
    }

    /// Sets the size at which the advanced curves are cut into chunks (see
    /// [`crate::writer::ContainerWriter::advanced_chunk_bytes`]).
    pub fn advanced_chunk_bytes(&mut self, bytes: u32) {
        self.advanced_chunk = bytes;
    }

    /// Sets the bytes at which a level of raster tiles is cut into chunks.
    pub fn raster_chunk_bytes(&mut self, bytes: u32) {
        self.raster_chunk = bytes;
    }

    /// Sets how many bytes of the coarsest levels the raster index blocks hold inline.
    pub fn raster_inline_bytes(&mut self, bytes: u64) {
        self.raster_inline = bytes;
    }

    fn note(&mut self, meta: &EntityMeta) -> Result<(), FinishError> {
        if let Some(ids) = self.ids.as_mut() {
            ids.push(u128::from(meta.id.index()), Vec::new())?;
        }
        self.extent = Some(match self.extent {
            Some(e) => union(e, meta.bounds),
            None => meta.bounds,
        });
        Ok(())
    }

    fn push_hot(
        &mut self,
        meta: EntityMeta,
        shape: HotShape,
        tag: &mut dyn FnMut() -> Vec<u8>,
    ) -> Result<(), FinishError> {
        let meta = crate::writer::ContainerWriter::checked_meta(meta)?;
        self.note(&meta)?;
        let record = HotRecord { item: HotItem { seq: 0, meta, shape }, tag: tag() };
        self.hot.append(&record.encode())?;
        self.hot_count += 1;
        Ok(())
    }

    fn push_advanced(
        &mut self,
        meta: EntityMeta,
        shape: AdvancedShape,
        tag: &mut dyn FnMut() -> Vec<u8>,
    ) -> Result<(), FinishError> {
        let meta = crate::writer::ContainerWriter::checked_meta(meta)?;
        if meta.cell != 0 {
            return Err(WriteError::CellOnAdvanced { id: meta.id.index() }.into());
        }
        // an advanced curve is in no leaf, so its tag is not kept, but the caller still looks at the curve
        tag();
        self.note(&meta)?;
        let item = AdvancedItem { seq: 0, meta, shape };
        let offset = self.advanced.append(&encode_advanced(&item))?;
        let segments = match &item.shape {
            AdvancedShape::Composite { segments, .. } => segments.clone(),
            AdvancedShape::Spline { .. } => Vec::new(),
        };
        self.notes.push(AdvancedNote { id: meta.id, offset, size: advanced_size(&item.shape), segments });
        Ok(())
    }

    /// Adds a line segment from `a` to `b`.
    pub fn push_line(&mut self, meta: EntityMeta, a: [f64; 2], b: [f64; 2]) -> Result<(), FinishError> {
        self.push_hot(meta, HotShape::Line(a, b), &mut Vec::new)
    }

    /// Adds a polyline of 2 to 2^20 vertices.
    pub fn push_polyline(&mut self, meta: EntityMeta, points: &[[f64; 2]]) -> Result<(), FinishError> {
        self.push_hot(meta, HotShape::Polyline(points.to_vec()), &mut Vec::new)
    }

    /// Adds a circle or circular arc.
    pub fn push_circle_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radius: f64,
        start_sweep: [f64; 2],
    ) -> Result<(), FinishError> {
        self.push_hot(meta, HotShape::CircleArc { centre, radius, start_sweep }, &mut Vec::new)
    }

    /// Adds an ellipse or elliptical arc.
    pub fn push_ellipse_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radii: [f64; 2],
        angles: [f64; 3],
    ) -> Result<(), FinishError> {
        self.push_hot(meta, HotShape::EllipseArc { centre, radii, angles }, &mut Vec::new)
    }

    /// Adds a B-spline. Its cell must be 0.
    pub fn push_spline(&mut self, meta: EntityMeta, spec: SplineSpec<'_>) -> Result<(), FinishError> {
        if spec.periodic {
            return Err(WriteError::Arena(crate::blocks::arena::ArenaError::PeriodicUnsupported).into());
        }
        let shape = AdvancedShape::Spline {
            degree: spec.degree,
            knots: spec.knots.to_vec(),
            x: spec.x.to_vec(),
            y: spec.y.to_vec(),
            weights: spec.weights.map(<[f64]>::to_vec),
            closed: spec.closed,
        };
        self.push_advanced(meta, shape, &mut Vec::new)
    }

    /// Adds a composite curve naming other advanced curves by id. Its cell must be 0.
    pub fn push_composite(
        &mut self,
        meta: EntityMeta,
        segments: &[EntityId],
        trims: Option<&[[f64; 2]]>,
        closed: bool,
    ) -> Result<(), FinishError> {
        let shape = AdvancedShape::Composite {
            segments: segments.to_vec(),
            trims: trims.map(<[[f64; 2]]>::to_vec),
            closed,
        };
        self.push_advanced(meta, shape, &mut Vec::new)
    }

    /// Adds a curve of the semantic model, as [`crate::writer::ContainerWriter::push_curve`] does. Once the
    /// curve is accepted `tag` is called, and the bytes it returns are kept beside the entity when it is stored
    /// as a hot one (and come back with [`SpillWriter::hot_records`]); a curve the container refuses never
    /// reaches it.
    pub fn push_curve(
        &mut self,
        id: EntityId,
        style: u32,
        cell: u32,
        curve: &Curve,
        tag: &mut dyn FnMut() -> Vec<u8>,
    ) -> Result<(), FinishError> {
        // the rotations the writer derives are listed by the in-memory writer for a test to read; here they would
        // be one more thing that grows with the drawing, so they are dropped after each curve
        self.derived.clear();
        match plan_curve(id, style, cell, curve, &mut self.derived)? {
            Item::Hot(meta, shape) => self.push_hot(meta, shape, tag),
            Item::Advanced(meta, shape) => self.push_advanced(meta, shape, tag),
        }
    }

    /// Adds a component stream. Its capability is declared in the manifest as the registry declares it.
    pub fn push_stream(&mut self, stream: &Stream) -> Result<(), FinishError> {
        self.streams.append(&encode_stream(stream))?;
        Ok(())
    }

    /// Adds encoded tiles of raster `raster`, as [`crate::writer::ContainerWriter::push_raster_tiles`] does.
    pub fn push_raster_tiles(&mut self, raster: RasterId, tiles: Vec<TileIn>) -> Result<(), FinishError> {
        let list = self.rasters.entry(raster.index()).or_default();
        for t in tiles {
            let offset = if t.bytes.is_empty() { 0 } else { self.tiles.append(&t.bytes)? };
            list.push(TileNote {
                meta: TileMeta { level: t.level, col: t.col, row: t.row, len: t.bytes.len() as u64 },
                offset,
            });
        }
        Ok(())
    }

    /// Sets the spatial directory block, as [`crate::writer::ContainerWriter::set_spatial_directory`] does.
    pub fn set_spatial_directory(&mut self, nodes: u32, bounds: Bounds, bytes: Vec<u8>) -> Result<(), WriteError> {
        let bounds = checked_bounds(bounds).ok_or(WriteError::BadBounds { id: None })?;
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        self.spatial = Some(Planned {
            first_seen: 0,
            kind: BlockKind::SpatialDirectory,
            cell: 0,
            subtype: 0,
            count: nodes,
            bounds: Some(bounds),
            bytes,
        });
        Ok(())
    }

    /// Adds the Tier 0 block of one index leaf, as [`crate::writer::ContainerWriter::push_tier0_leaf`] does.
    /// The block is compressed and spooled at once.
    pub fn push_tier0_leaf(
        &mut self,
        cell: u32,
        count: u32,
        bounds: Bounds,
        bytes: Vec<u8>,
    ) -> Result<(), FinishError> {
        if cell == 0 || cell >= crate::directory::STRADDLER_CELL_FIRST || self.tier0_cells.contains(&cell) {
            return Err(WriteError::Tier0Cell { cell }.into());
        }
        let bounds = checked_bounds(bounds).ok_or(WriteError::BadBounds { id: None })?;
        u32::try_from(bytes.len()).map_err(|_| WriteError::TooLarge)?;
        self.tier0_cells.insert(cell);
        let planned = Planned {
            first_seen: 0,
            kind: BlockKind::Tier0,
            cell,
            subtype: 0,
            count,
            bounds: Some(bounds),
            bytes,
        };
        self.spool.push_planned(self.options.codecs, planned)
    }

    /// The hot entities in the order they were pushed, with their tags.
    pub fn hot_records(&mut self) -> SpillResult<impl Iterator<Item = SpillResult<HotRecord>> + use<>> {
        Ok(self.hot.records()?.map(|r| r.and_then(|bytes| HotRecord::decode(&bytes))))
    }

    /// Puts the hot entities `records`, all of one cell, in their blocks: one block for each geometry type, ascending
    /// by id. After the first call the writer takes the hot entities from its callers' calls and not from the
    /// pushes, so a caller that places entities in cells itself calls this once for every cell and leaves the
    /// pushes to be read with [`SpillWriter::hot_records`].
    pub fn emit_hot_cell(&mut self, cell: u32, records: Vec<HotRecord>) -> Result<(), FinishError> {
        self.hot_emitted = true;
        let mut by_kind: BTreeMap<u16, (HotKind, Vec<HotRecord>)> = BTreeMap::new();
        for r in records {
            let kind = r.item.shape.kind();
            by_kind.entry(kind.id()).or_insert_with(|| (kind, Vec::new())).1.push(r.in_cell(cell));
        }
        for (_, (kind, group)) in by_kind {
            let items: Vec<&HotItem> = group.iter().map(|r| &r.item).collect();
            let planned = hot_block(kind, cell, items, true)?;
            self.spool.push_planned(self.options.codecs, planned)?;
        }
        Ok(())
    }

    /// What the ids pushed so far say. It ends the id sort, so call it once the pushes are done; later calls
    /// answer the same.
    pub fn id_summary(&mut self) -> Result<IdSummary, FinishError> {
        if let Some(s) = self.summary {
            return Ok(s);
        }
        let mut summary = IdSummary { count: 0, max: None, first_gap: None, duplicate: None };
        if let Some(sorter) = self.ids.take() {
            // the next id that would keep the run dense
            let mut next = 0u32;
            for rec in sorter.finish()? {
                let (key, _) = rec?;
                let id = key as u32;
                summary.count += 1;
                if summary.max == Some(id) {
                    summary.duplicate.get_or_insert(id);
                    continue;
                }
                if id > next && summary.first_gap.is_none() {
                    summary.first_gap = Some(next);
                }
                next = id + 1;
                summary.max = Some(id);
            }
        }
        self.summary = Some(summary);
        Ok(summary)
    }

    /// The artifact, written to `sink` with no seek, and its length.
    ///
    /// Everything is built and checked before the first byte reaches `sink`, so a refusal writes nothing.
    pub fn finish_to(mut self, sink: &mut dyn ByteSink) -> Result<u64, FinishError> {
        if self.hot_count == 0 && self.notes.is_empty() {
            return Err(WriteError::Empty.into());
        }
        let summary = self.id_summary()?;
        if let Some(id) = summary.duplicate {
            return Err(WriteError::DuplicateId { id }.into());
        }
        let streams: Vec<Stream> = self.streams.records()?.map(|r| r.and_then(|b| decode_stream(&b))).collect::<SpillResult<_>>()?;
        let (capabilities, min_reader) =
            declare(&self.config.capabilities, streams.iter().map(|s| s.capability), true, &Registry::v1())?;
        let source_bounds = match self.config.bounds {
            Some(b) => checked_bounds(b).ok_or(WriteError::BadBounds { id: None })?,
            None => self.extent.ok_or(WriteError::Empty)?,
        };
        let policy = self.options.codecs;
        if !self.hot_emitted {
            self.emit_hot_by_cell()?;
        }
        self.emit_advanced()?;
        let numbered: Vec<(usize, Stream)> = streams.iter().cloned().enumerate().collect();
        if let Some(block) = stream_block_of(&numbered, true)? {
            self.spool.push_planned(policy, block)?;
        }
        for planned in [self.spatial.take(), self.cell_refs.take(), self.straddlers.take()].into_iter().flatten() {
            self.spool.push_planned(policy, planned)?;
        }
        self.emit_rasters(streams.iter().find(|s| s.capability == RASTER_CAPABILITY))?;
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
        let integrity = self.options.integrity;
        self.spool.assemble_to(manifest, integrity, sink)
    }

    /// The hot entities, by cell and geometry type and then by id, one block at a time.
    fn emit_hot_by_cell(&mut self) -> Result<(), FinishError> {
        let mut sorter = ExternalSorter::new(&self.store, self.budget);
        for rec in self.hot.records()? {
            let bytes = rec?;
            let r = HotRecord::decode(&bytes)?;
            let m = r.meta();
            let key = u128::from(m.cell) << 64 | u128::from(r.item.shape.kind().id()) << 32 | u128::from(m.id.index());
            sorter.push(key, bytes)?;
        }
        // records of one cell and geometry type are adjacent; each goes into its block as it arrives, and the
        // block is finished when the next one starts
        let mut open: Option<((u32, u16), HotGroup)> = None;
        for rec in sorter.finish()? {
            let (_, bytes) = rec?;
            let r = HotRecord::decode(&bytes)?;
            let at = (r.meta().cell, r.item.shape.kind().id());
            if open.as_ref().is_some_and(|(k, _)| *k != at) {
                if let Some((_, group)) = open.take() {
                    self.spool.push_planned(self.options.codecs, group.finish(true)?)?;
                }
            }
            let (_, group) = open.get_or_insert_with(|| (at, HotGroup::new(r.item.shape.kind(), at.0)));
            group.push(&r.item)?;
        }
        if let Some((_, group)) = open.take() {
            self.spool.push_planned(self.options.codecs, group.finish(true)?)?;
        }
        Ok(())
    }

    fn emit_advanced(&mut self) -> Result<(), FinishError> {
        if self.notes.is_empty() {
            return Ok(());
        }
        self.notes.sort_by_key(|n| n.id);
        let links: Vec<(EntityId, u64, &[EntityId])> =
            self.notes.iter().map(|n| (n.id, n.size, n.segments.as_slice())).collect();
        let chunks = plan_chunks(&links, self.advanced_chunk);
        for (k, chunk) in chunks.iter().enumerate() {
            let mut items = Vec::with_capacity(chunk.len());
            for i in chunk {
                items.push(decode_advanced(&self.advanced.read_at(self.notes[*i].offset)?)?);
            }
            let members: Vec<&AdvancedItem> = items.iter().collect();
            for planned in advanced_chunk(&members, k)? {
                self.spool.push_planned(self.options.codecs, planned)?;
            }
        }
        Ok(())
    }

    fn emit_rasters(&mut self, placements: Option<&Stream>) -> Result<(), FinishError> {
        if self.rasters.is_empty() {
            return Ok(());
        }
        let metas: BTreeMap<u32, Vec<TileMeta>> =
            self.rasters.iter().map(|(id, tiles)| (*id, tiles.iter().map(|t| t.meta).collect())).collect();
        let rules = RasterRules { inline_bytes: self.raster_inline, chunk_bytes: self.raster_chunk, sniff: self.sniff_tiles };
        let (tiles_log, rasters, spool) = (&mut self.tiles, &self.rasters, &mut self.spool);
        raster_blocks_of::<FinishError>(
            &metas,
            placements,
            rules,
            &mut |raster, i, _| {
                let note = &rasters[&raster][i];
                tiles_log.read_at(note.offset).map_err(FinishError::Spill)
            },
            &mut |block| spool.push(block),
        )
        .map_err(|f| match f {
            RasterFail::Write(e) => FinishError::Write(e),
            RasterFail::Source(e) => e,
        })
    }
}
