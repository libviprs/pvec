//! The safe range-readable reader (issue P4.5).
//!
//! One reader for a local file, memory the caller mapped, a positional-read interface and async HTTP
//! ranges. The checks live in `core` and are written once, sans IO: they say which range they need
//! next and are fed the bytes, so [`ContainerReader`] (blocking) and [`AsyncContainerReader`] (async)
//! are the same loop around the same code, and the tests drive both through one shared body.
//!
//! What the reader promises about a hostile file:
//!
//! * every offset, length and count is checked against the artifact length and against the [`Limits`]
//!   before a buffer is reserved, so a lie is refused for what it would have cost and not paid for;
//! * the live buffers are counted by an [`Accountant`], so the budget covers their sum;
//! * a range that comes back short is an error and is never decoded;
//! * there is no `unsafe` in this module, and mapped memory is passed in as a slice and held to the same
//!   checks as bytes from a socket;
//! * it does not panic: a refusal is a [`ReadError`] that names its rule.
//!
//! Opening a file costs three requests at most (the first 4 KiB, which holds the header and the
//! manifest; the block directory; and, once a block is wanted, the block), and `plan` coalesces the
//! blocks a caller wants into as few requests as the gap threshold allows. `docs/READER.md` is the
//! contract.

use crate::blocks::AlignedBytes;
use crate::blocks::advanced::{AdvancedCurveRecord, AdvancedView, BlockSlices, CurveView};
use crate::blocks::arena::ArenaError;
use crate::blocks::hot::{HotBlockView, HotError};
use crate::blocks::raster::{IndexError, RasterIndex, Tile};
use crate::blocks::sniff::{TileRefusal, max_tile_bytes, sniff_tile};
use crate::compress::CodecError;
use crate::directory::BlockEntry;
use crate::directory::DirectoryError;
use crate::engine::{Core, DigestCheck, Need, OpenOptions, Opening, Validation};
use crate::integrity::{DigestStatus, IntegrityError};
use crate::limits::{Accountant, LimitExceeded, LimitKind, Limits, LimitsError, Reservation};
use crate::negotiate::{PartialOpen, ReaderCapabilities, Refusal};
use crate::source::{AsyncBlockSource, BlockSource, Bytes, SourceError};
use crate::{ContainerError, Header, Manifest};
use pvec_capabilities::{Reader, Registry};
use pvec_components::{
    CALIBRATION_KIND, Calibration, CalibrationError, Placement, RasterError, RasterRead, RasterSkip,
    StreamError, decode_calibration, read_rasters, read_streams,
};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// A block, by its position in the directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockId(pub u32);

/// One range to fetch, and the blocks it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RangeRequest {
    /// First byte.
    pub offset: u64,
    /// Bytes.
    pub len: u32,
    /// The blocks inside it, in file order.
    pub blocks: Vec<BlockId>,
}

/// How a reader is configured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderConfig {
    /// The ceilings a file is held to.
    pub limits: Limits,
    /// The bytes of decoded blocks the cache may hold. Zero turns the cache off.
    pub cache_bytes: u64,
    /// Blocks whose gap is at most this many bytes are fetched in one request, reading the gap too.
    pub gap_threshold: u32,
    /// The most bytes one coalesced request may span.
    pub max_request_bytes: u32,
    /// What this reader supports, which the file's manifest is negotiated against.
    pub capabilities: ReaderCapabilities,
    /// Refuse a file that carries no block checksum table or no artifact digest.
    pub require_integrity: bool,
}

impl Default for ReaderConfig {
    fn default() -> ReaderConfig {
        ReaderConfig {
            limits: Limits::DEFAULT,
            cache_bytes: 64 << 20,
            gap_threshold: 64 << 10,
            max_request_bytes: 16 << 20,
            capabilities: ReaderCapabilities::v1(),
            require_integrity: false,
        }
    }
}

/// Why a read was refused. Every variant names a rule ([`ReadError::rule`]).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ReadError {
    /// The source failed or answered short.
    Source(SourceError),
    /// A limit was exceeded.
    Limit(LimitExceeded),
    /// The configured limits are not usable.
    BadLimits(LimitsError),
    /// The header or the manifest was refused.
    Container(ContainerError),
    /// The block directory was refused.
    Directory(DirectoryError),
    /// A hot block was refused.
    Hot(HotError),
    /// An advanced block was refused.
    Arena(ArenaError),
    /// The component stream set was refused.
    Stream(StreamError),
    /// The layout of the file is not the canonical one (`LAYOUT-nn`), or the blocks and the directory
    /// disagree (`CONSIST-nn`).
    Layout {
        /// The rule.
        rule: &'static str,
        /// What was wrong.
        detail: String,
    },
    /// A block would not decompress, or decompressed to the wrong length.
    Codec(CodecError),
    /// A checksum or the artifact digest did not match.
    Integrity(IntegrityError),
    /// A required capability is not supported.
    Negotiation(Refusal),
    /// A raster was refused (`RASTER-nn`, or `LIMIT-14`). The raster is, and the file is not: the geometry and
    /// every other raster still read.
    Raster(IndexError),
    /// A tile the sniffer refuses (`RASTER-03`, `RASTER-04`): the raster's other tiles and the file read.
    Tile(TileRefusal),
    /// The raster placement stream is damaged or refused (`RASTER-nn`, `COMP-nn`). The rasters are, the geometry
    /// and the other streams are not.
    Placement(RasterError),
    /// The calibration is refused (`CAL-nn`, `docs/CALIBRATION.md`). The calibration is, the file is not.
    Calibration(CalibrationError),
    /// The block was skipped when the file was opened, so its payload is not read.
    Skipped {
        /// The block.
        id: u32,
    },
    /// A block id that is not in the directory.
    NoSuchBlock {
        /// The id asked for.
        id: u32,
        /// How many blocks there are.
        blocks: usize,
    },
}

impl ReadError {
    /// The rule a refusal is pinned by: the variant name for a header or manifest refusal, `DIR-nn`,
    /// `HOT-nn`, `ARN-nn`, `STREAM-nn`, `LIMIT-nn`, `LAYOUT-nn` or `CONSIST-nn`.
    pub fn rule(&self) -> String {
        match self {
            ReadError::Source(_) => "SOURCE".to_string(),
            ReadError::Limit(l) => l.limit.rule().to_string(),
            ReadError::BadLimits(_) => "LIMITS".to_string(),
            ReadError::Container(e) => e.rule().to_string(),
            ReadError::Directory(e) => e.rule().to_string(),
            ReadError::Hot(e) => e.rule().to_string(),
            ReadError::Arena(e) => e.rule().to_string(),
            ReadError::Stream(e) => e.rule().to_string(),
            ReadError::Layout { rule, .. } => (*rule).to_string(),
            ReadError::NoSuchBlock { .. } => "BLOCK-ID".to_string(),
            ReadError::Codec(e) => e.rule().to_string(),
            ReadError::Integrity(e) => e.rule().to_string(),
            ReadError::Negotiation(r) => r.rule().to_string(),
            ReadError::Raster(e) => e.rule().to_string(),
            ReadError::Tile(e) => e.rule().to_string(),
            ReadError::Placement(e) => e.rule().to_string(),
            ReadError::Calibration(e) => e.rule().to_string(),
            ReadError::Skipped { .. } => "SKIPPED".to_string(),
        }
    }

    /// The limit, when the refusal is one.
    pub fn limit(&self) -> Option<&LimitExceeded> {
        match self {
            ReadError::Limit(l) => Some(l),
            _ => None,
        }
    }
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Source(e) => write!(f, "source: {e}"),
            ReadError::Limit(e) => write!(f, "{e}"),
            ReadError::BadLimits(e) => write!(f, "limits: {e}"),
            ReadError::Container(e) => write!(f, "{e}"),
            ReadError::Directory(e) => write!(f, "{e}"),
            ReadError::Hot(e) => write!(f, "{e}"),
            ReadError::Arena(e) => write!(f, "{e}"),
            ReadError::Stream(e) => write!(f, "{e}"),
            ReadError::Layout { rule, detail } => write!(f, "{rule}: {detail}"),
            ReadError::NoSuchBlock { id, blocks } => write!(f, "no block {id} in a directory of {blocks}"),
            ReadError::Codec(e) => write!(f, "{e}"),
            ReadError::Integrity(e) => write!(f, "{e}"),
            ReadError::Negotiation(e) => write!(f, "{e}"),
            ReadError::Raster(e) => write!(f, "{e}"),
            ReadError::Tile(e) => write!(f, "{e}"),
            ReadError::Placement(e) => write!(f, "{e}"),
            ReadError::Calibration(e) => write!(f, "{e}"),
            ReadError::Skipped { id } => write!(f, "block {id} was skipped when the file was opened"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<SourceError> for ReadError {
    fn from(e: SourceError) -> ReadError {
        ReadError::Source(e)
    }
}

impl From<LimitExceeded> for ReadError {
    fn from(e: LimitExceeded) -> ReadError {
        ReadError::Limit(e)
    }
}

/// A block as it is decoded: aligned bytes whose size is held against the allocation budget until the
/// block is dropped.
#[derive(Debug)]
pub struct DecodedBlock {
    id: BlockId,
    entry: BlockEntry,
    bytes: AlignedBytes,
    _reservation: Reservation,
}

impl DecodedBlock {
    pub(crate) fn new(
        id: BlockId,
        entry: BlockEntry,
        bytes: AlignedBytes,
        reservation: Reservation,
    ) -> DecodedBlock {
        DecodedBlock { id, entry, bytes, _reservation: reservation }
    }

    /// Where the block is in the directory.
    pub fn id(&self) -> BlockId {
        self.id
    }

    /// Its directory entry.
    pub fn entry(&self) -> &BlockEntry {
        &self.entry
    }

    /// The decoded bytes, starting on an 8-byte boundary.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// Parses the block as a hot geometry block, in place.
    pub fn hot(&self) -> Result<HotBlockView<'_>, ReadError> {
        HotBlockView::parse(self.bytes.as_bytes()).map_err(ReadError::Hot)
    }
}

/// The record block and the five arenas of the advanced curves, decoded.
#[derive(Debug)]
pub struct AdvancedSet {
    blocks: [Arc<DecodedBlock>; 6],
    limits: Limits,
}

impl AdvancedSet {
    /// Parses the six blocks and checks the nesting of the composites against the reader's ceiling
    /// before any segment is resolved.
    pub fn view(&self) -> Result<CheckedView<'_>, ReadError> {
        let b = &self.blocks;
        let view = AdvancedView::parse(BlockSlices {
            records: b[0].as_bytes(),
            knots: b[1].as_bytes(),
            weights: b[2].as_bytes(),
            control_points: b[3].as_bytes(),
            segments: b[4].as_bytes(),
            trims: b[5].as_bytes(),
        })
        .map_err(ReadError::Arena)?;
        crate::engine::check_nesting(&view, self.limits.max_nesting_depth as usize)?;
        Ok(CheckedView { view, limits: self.limits })
    }
}

/// The advanced curves of a file, read under the reader's limits: a curve whose offset-table entries
/// declare more control points or knots than the ceiling is refused as a limit before anything is read
/// out of the arenas.
#[derive(Clone, Copy, Debug)]
pub struct CheckedView<'a> {
    view: AdvancedView<'a>,
    limits: Limits,
}

impl<'a> CheckedView<'a> {
    /// Number of records.
    pub fn len(&self) -> usize {
        self.view.len()
    }

    /// Whether there are no records (a parsed block never has none).
    pub fn is_empty(&self) -> bool {
        self.view.is_empty()
    }

    /// Record `index`, in ascending entity id order.
    pub fn record(&self, index: usize) -> Option<AdvancedCurveRecord> {
        self.view.record(index)
    }

    /// The curve at record `index`, borrowed from the arenas.
    pub fn curve(&self, index: usize) -> Result<CurveView<'a>, ReadError> {
        crate::engine::checked_curve(&self.view, &self.limits, index)
    }

    /// The unchecked view, for a caller that applies its own limits.
    pub fn as_view(&self) -> &AdvancedView<'a> {
        &self.view
    }
}

/// Counters a reader keeps about itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadStats {
    /// Range requests issued to the source.
    pub requests: u64,
    /// Bytes asked of the source.
    pub bytes: u64,
    /// Blocks served from the cache.
    pub cache_hits: u64,
    /// Blocks that had to be fetched.
    pub cache_misses: u64,
    /// Cache entries dropped to make room.
    pub evictions: u64,
}

/// What a full validation found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Validated {
    /// Blocks in the directory.
    pub blocks: usize,
    /// Entities: hot entities and advanced records.
    pub entities: u64,
    /// The capability ids of the component streams, ascending.
    pub stream_capabilities: Vec<u32>,
    /// The blocks that were skipped and never read (P4.7).
    pub skipped: Vec<BlockId>,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    bytes: AtomicU64,
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
}

struct CacheEntry {
    block: Arc<DecodedBlock>,
    tick: u64,
}

/// The decoded-block cache: least recently used first out, each entry costing its decoded length, and
/// an entry bigger than the whole budget is not kept. Dropping an entry drops the cache's reference
/// only; the bytes go back to the accountant when the last caller lets go of the block, which is why
/// making room for the allocation budget only evicts entries nobody else holds.
struct Cache {
    map: HashMap<BlockId, CacheEntry>,
    /// Entries by the tick they were last used at, so the oldest is the first.
    order: BTreeMap<u64, BlockId>,
    bytes: u64,
    budget: u64,
    tick: u64,
}

impl Cache {
    fn new(budget: u64) -> Cache {
        Cache { map: HashMap::new(), order: BTreeMap::new(), bytes: 0, budget, tick: 0 }
    }

    fn get(&mut self, id: BlockId) -> Option<Arc<DecodedBlock>> {
        self.tick += 1;
        let tick = self.tick;
        let e = self.map.get_mut(&id)?;
        self.order.remove(&e.tick);
        e.tick = tick;
        self.order.insert(tick, id);
        Some(Arc::clone(&e.block))
    }

    fn cost(block: &DecodedBlock) -> u64 {
        block.as_bytes().len() as u64
    }

    fn remove(&mut self, id: BlockId) {
        if let Some(e) = self.map.remove(&id) {
            self.order.remove(&e.tick);
            self.bytes -= Cache::cost(&e.block);
        }
    }

    /// Drops the least recently used entry, whoever else holds the block. False when there is none.
    fn evict_oldest(&mut self) -> bool {
        let Some(victim) = self.order.values().next().copied() else { return false };
        self.remove(victim);
        true
    }

    /// Drops the least recently used entry that only the cache holds, since dropping one a caller still
    /// holds frees nothing. False when there is none.
    fn evict_unreferenced(&mut self) -> bool {
        let victim = self
            .order
            .values()
            .find(|id| self.map.get(*id).is_some_and(|e| Arc::strong_count(&e.block) == 1))
            .copied();
        match victim {
            Some(id) => {
                self.remove(id);
                true
            }
            None => false,
        }
    }

    fn insert(&mut self, block: Arc<DecodedBlock>) -> u64 {
        let cost = Cache::cost(&block);
        if cost > self.budget {
            return 0;
        }
        let mut evicted = 0;
        self.remove(block.id());
        while self.bytes + cost > self.budget && self.evict_oldest() {
            evicted += 1;
        }
        self.tick += 1;
        self.bytes += cost;
        self.order.insert(self.tick, block.id());
        self.map.insert(block.id(), CacheEntry { block, tick: self.tick });
        evicted
    }
}

/// What both readers hold: the checked layout, the accountant, the cache and the counters.
struct Shared {
    core: Core,
    config: ReaderConfig,
    acct: Accountant,
    cache: Mutex<Cache>,
    counters: Counters,
    /// The raster indexes read so far, parsed and checked, each with the room it holds in the budget.
    rasters: Mutex<HashMap<u32, (Arc<RasterIndex>, Reservation)>>,
    /// The raster placements, read from the component set the first time a raster is asked for.
    placements: Mutex<Option<Result<Arc<Vec<Placement>>, ReadError>>>,
}

/// Where the bytes of a tile are, once its index entry is known.
enum TilePlan {
    /// The tile is not stored: no bytes and no error.
    NotStored,
    /// The tile is in the index block, already in hand.
    Inline(Tile, Bytes),
    /// The tile is a range of a tile block: the offset in the file and the length.
    Range { tile: Tile, offset: u64, len: u32 },
}

impl Shared {
    fn new(core: Core, config: ReaderConfig, counters: Counters, acct: Accountant) -> Shared {
        Shared {
            cache: Mutex::new(Cache::new(config.cache_bytes)),
            core,
            config,
            counters,
            acct,
            rasters: Mutex::new(HashMap::new()),
            placements: Mutex::new(None),
        }
    }

    /// The placements when they were read before (a damaged stream is remembered as damaged).
    fn placements_cached(&self) -> Option<Result<Arc<Vec<Placement>>, ReadError>> {
        self.placements.lock().ok()?.clone()
    }

    /// The block that holds the component streams, when the file has one.
    fn components_block(&self) -> Option<BlockId> {
        self.core
            .entries
            .iter()
            .position(|e| e.block_kind() == Some(crate::directory::BlockKind::Components))
            .map(|i| BlockId(i as u32))
    }

    /// Reads the raster stream out of the component set `block` and keeps what it found. A file with no component
    /// set, or none with a raster stream, places no raster; a damaged stream is an error for the rasters and for
    /// nothing else, and is not read again.
    fn placements_parsed(&self, block: Option<&DecodedBlock>) -> Result<Arc<Vec<Placement>>, ReadError> {
        let parsed = match block {
            None => Ok(Vec::new()),
            Some(b) => {
                let registry = Registry::v1();
                let reader = Reader {
                    version: pvec_capabilities::FORMAT_VERSION,
                    implemented: registry.entries().iter().map(|c| c.id).collect(),
                };
                read_streams(b.as_bytes(), &registry, &reader).map_err(ReadError::Stream).and_then(|opened| {
                    match read_rasters(&opened) {
                        RasterRead::Absent => Ok(Vec::new()),
                        RasterRead::Present(p) => Ok(p),
                        RasterRead::Damaged(e) => Err(ReadError::Placement(e)),
                    }
                })
            }
        }
        .map(Arc::new);
        if let Ok(mut slot) = self.placements.lock() {
            *slot = Some(parsed.clone());
        }
        parsed
    }

    /// The parsed index of `raster` when it was read before.
    fn raster_cached(&self, raster: u32) -> Option<Arc<RasterIndex>> {
        self.rasters.lock().ok()?.get(&raster).map(|(i, _)| Arc::clone(i))
    }

    /// The block to read for the index of `raster`: an error that refuses the raster when there is none.
    fn raster_index_id(&self, raster: u32) -> Result<BlockId, ReadError> {
        let id =
            self.core.raster_index_block(raster).ok_or(ReadError::Raster(IndexError::NoIndex { raster }))?;
        let entry = self.core.check_readable(id)?;
        // stored as it is (RASTER-06): refused before a compressed block is decoded for nothing
        if entry.codec != crate::directory::CODEC_NONE {
            return Err(ReadError::Raster(IndexError::Block { raster, subtype: 0 }));
        }
        Ok(id)
    }

    /// Parses the index block of `raster`, checks it against the directory, and keeps it. The tile table is
    /// held for as long as the reader holds the index, and that room is taken from the budget.
    fn raster_parsed(
        &self,
        raster: u32,
        block: &DecodedBlock,
        placement: &Placement,
    ) -> Result<Arc<RasterIndex>, ReadError> {
        // another thread may have been first
        if let Some(index) = self.raster_cached(raster) {
            return Ok(index);
        }
        // LIMIT-14: the entries of the tile tables over the whole artifact, so what is held already counts. This
        // reads the total for an early refusal; the insert below checks it again under the lock.
        let ceiling = u64::from(self.config.limits.max_raster_tiles);
        let limit =
            |declared| LimitExceeded { limit: crate::limits::LimitKind::RasterTiles, declared, ceiling };
        let held = |m: &HashMap<u32, (Arc<RasterIndex>, Reservation)>| -> u64 {
            m.values().map(|(i, _)| i.tiles().len() as u64).sum()
        };
        let before = self.rasters.lock().map(|m| held(&m)).unwrap_or_default();
        let index =
            RasterIndex::parse(block.as_bytes(), ceiling.saturating_sub(before)).map_err(|e| match e {
                IndexError::TooManyTiles { declared, .. } => ReadError::Limit(limit(before + declared)),
                other => ReadError::Raster(other),
            })?;
        // the grid of the index is the grid of the placement, not one the file chose (RASTER-02)
        if index.levels().len() != usize::from(placement.levels) {
            return Err(ReadError::Raster(IndexError::Grid { level: index.levels().len() as u32 }));
        }
        index
            .check_grid(placement.width, placement.height, u32::from(placement.tile_edge))
            .map_err(ReadError::Raster)?;
        self.core.check_raster_blocks(raster, &index).map_err(ReadError::Raster)?;
        // the tile table is held for as long as the reader holds the index, and that room is taken from the budget
        let room = (core::mem::size_of_val(index.tiles()) + block.as_bytes().len()) as u64;
        let reservation = self.reserve(room)?;
        let index = Arc::new(index);
        let Ok(mut map) = self.rasters.lock() else { return Ok(index) };
        if let Some((existing, _)) = map.get(&raster) {
            return Ok(Arc::clone(existing));
        }
        let total = held(&map) + index.tiles().len() as u64;
        if total > ceiling {
            return Err(ReadError::Limit(limit(total)));
        }
        map.insert(raster, (Arc::clone(&index), reservation));
        Ok(index)
    }

    /// What is decided about a stored tile before a byte of it is asked for: a raster whose codec this build does
    /// not know is skipped and reported (`RASTER-04`), and the length in the index is held to `LIMIT-15`.
    fn tile_admitted(&self, placement: &Placement, tile: &Tile) -> Result<(), ReadError> {
        if let Some(RasterSkip::UnknownCodec(id)) = placement.skip() {
            return Err(ReadError::Tile(TileRefusal::UnknownCodec { id }));
        }
        let ceiling = max_tile_bytes(u32::from(placement.tile_edge), self.config.limits.max_tile_bytes);
        if u64::from(tile.length) > ceiling {
            return Err(ReadError::Limit(LimitExceeded {
                limit: crate::limits::LimitKind::TileBytes,
                declared: u64::from(tile.length),
                ceiling,
            }));
        }
        Ok(())
    }

    /// What it takes to read tile `(col, row)` of `level`: nothing for a tile that is not stored, the bytes for
    /// an inline tile (checked against the index entry), or the range of the chunk to fetch, checked to lie
    /// inside the chunk block and charged to the budget before it is requested.
    fn tile_plan(
        &self,
        raster: u32,
        index: &RasterIndex,
        placement: &Placement,
        (level, col, row): (u32, u32, u32),
    ) -> Result<TilePlan, ReadError> {
        let Some(tile) = index.tile(level, col, row).copied() else {
            return Err(ReadError::Raster(IndexError::Tile { level, col, row }));
        };
        if !tile.is_stored() {
            return Ok(TilePlan::NotStored);
        }
        self.tile_admitted(placement, &tile)?;
        if tile.is_inline() {
            let bytes =
                index.inline_bytes(&tile).ok_or(ReadError::Raster(IndexError::Tile { level, col, row }))?;
            let _room = self.admit_gap(tile.length)?;
            return Ok(TilePlan::Inline(tile, Bytes::new(bytes.to_vec())));
        }
        let chunk = self
            .core
            .raster_chunk_block(raster, level, tile.chunk)
            .ok_or(ReadError::Raster(IndexError::Chunk { level, chunk: tile.chunk }))?;
        let entry = self.core.check_readable(chunk)?;
        let end = u64::from(tile.offset) + u64::from(tile.length);
        if end > u64::from(entry.stored_length) {
            return Err(ReadError::Raster(IndexError::Chunk { level, chunk: tile.chunk }));
        }
        Ok(TilePlan::Range { tile, offset: entry.offset + u64::from(tile.offset), len: tile.length })
    }

    /// Holds the bytes of a tile to the checksum of its index entry, before they leave the reader.
    fn tile_verified(
        &self,
        raster: u32,
        placement: &Placement,
        (level, col, row): (u32, u32, u32),
        tile: &Tile,
        bytes: Bytes,
    ) -> Result<Bytes, ReadError> {
        let found = crate::integrity::block_checksum(&bytes);
        if found != tile.xxh3 {
            return Err(ReadError::Integrity(IntegrityError::TileChecksum {
                raster,
                level,
                col,
                row,
                expected: tile.xxh3,
                found,
            }));
        }
        // RASTER-03 and RASTER-04: the header says what the placement says, and the tile is one image and nothing
        // after it, before the bytes leave the reader
        sniff_tile(placement, self.config.limits.max_tile_bytes, &bytes).map_err(ReadError::Tile)?;
        Ok(bytes)
    }

    fn note_request(&self, len: u32) {
        self.counters.requests.fetch_add(1, Ordering::Relaxed);
        self.counters.bytes.fetch_add(u64::from(len), Ordering::Relaxed);
    }

    fn cached(&self, id: BlockId) -> Option<Arc<DecodedBlock>> {
        let hit = self.cache.lock().ok().and_then(|mut c| c.get(id));
        let counter = if hit.is_some() { &self.counters.hits } else { &self.counters.misses };
        counter.fetch_add(1, Ordering::Relaxed);
        hit
    }

    /// Reserves `bytes`, evicting cache entries (least recently used first) while the budget refuses.
    fn reserve(&self, bytes: u64) -> Result<Reservation, ReadError> {
        loop {
            match self.acct.reserve(bytes) {
                Ok(r) => return Ok(r),
                Err(e) => {
                    let evicted = self.cache.lock().map(|mut c| c.evict_unreferenced()).unwrap_or(false);
                    if !evicted {
                        return Err(ReadError::Limit(e));
                    }
                    self.counters.evictions.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Everything a block costs, charged before a byte of it is fetched: the decode work, the room for
    /// the decoded block, and the room for the bytes in flight. A block the budget would refuse is
    /// refused here, without a request.
    fn admit(&self, id: BlockId) -> Result<(Reservation, Reservation), ReadError> {
        let entry = self.core.check_readable(id)?;
        let in_flight = self.core.in_flight(entry);
        self.acct.charge_work(u64::from(entry.stored_length) + u64::from(entry.uncompressed_length))?;
        let decoded = self.reserve(u64::from(entry.uncompressed_length))?;
        let transient = self.reserve(in_flight)?;
        Ok((decoded, transient))
    }

    /// Decodes the bytes fetched for block `id` into the room reserved for it and caches the result.
    fn finish_block(
        &self,
        id: BlockId,
        stored: &[u8],
        decoded: Reservation,
    ) -> Result<Arc<DecodedBlock>, ReadError> {
        let block = Arc::new(self.core.decode(id, stored, decoded)?);
        if let Ok(mut c) = self.cache.lock() {
            let evicted = c.insert(Arc::clone(&block));
            self.counters.evictions.fetch_add(evicted, Ordering::Relaxed);
        }
        Ok(block)
    }

    fn stats(&self) -> ReadStats {
        let c = &self.counters;
        ReadStats {
            requests: c.requests.load(Ordering::Relaxed),
            bytes: c.bytes.load(Ordering::Relaxed),
            cache_hits: c.hits.load(Ordering::Relaxed),
            cache_misses: c.misses.load(Ordering::Relaxed),
            evictions: c.evictions.load(Ordering::Relaxed),
        }
    }

    fn advanced_set(&self, blocks: Vec<Arc<DecodedBlock>>) -> Result<AdvancedSet, ReadError> {
        let blocks: [Arc<DecodedBlock>; 6] = blocks.try_into().map_err(|_| ReadError::Layout {
            rule: "CONSIST-04",
            detail: "the advanced set is not six blocks".into(),
        })?;
        Ok(AdvancedSet { blocks, limits: self.config.limits })
    }

    /// The padding between blocks is fetched in chunks, and each chunk is charged like anything else.
    fn admit_gap(&self, len: u32) -> Result<Reservation, ReadError> {
        self.acct.charge_work(u64::from(len))?;
        self.reserve(u64::from(len))
    }

    /// For each request of a plan: the room for what it fetches, reserved before it is asked for.
    fn admit_request(&self, request: &RangeRequest) -> Result<(Vec<Reservation>, Reservation), ReadError> {
        let mut decoded = Vec::with_capacity(request.blocks.len());
        let mut decoder_room = 0u64;
        for id in &request.blocks {
            let entry = self.core.check_readable(*id)?;
            decoder_room = decoder_room.max(self.core.in_flight(entry) - u64::from(entry.stored_length));
            self.acct.charge_work(u64::from(entry.stored_length) + u64::from(entry.uncompressed_length))?;
            decoded.push(self.reserve(u64::from(entry.uncompressed_length))?);
        }
        let transient = self.reserve(u64::from(request.len) + decoder_room)?;
        Ok((decoded, transient))
    }
}

fn open_options(config: &ReaderConfig) -> OpenOptions<'_> {
    OpenOptions {
        limits: &config.limits,
        capabilities: &config.capabilities,
        require_integrity: config.require_integrity,
    }
}

/// Exactly `len` bytes, or an error: a source that answers with another length is refused here too,
/// whatever its own contract says.
fn checked(bytes: Bytes, len: u32) -> Result<Bytes, ReadError> {
    if bytes.len() as u64 == u64::from(len) {
        Ok(bytes)
    } else {
        Err(ReadError::Source(SourceError::Short { wanted: u64::from(len), got: bytes.len() as u64 }))
    }
}

macro_rules! accessors {
    () => {
        /// The source the reader reads from.
        pub fn source(&self) -> &S {
            &self.source
        }

        /// The header.
        pub fn header(&self) -> &Header {
            &self.shared.core.header
        }

        /// The manifest.
        pub fn manifest(&self) -> &Manifest {
            &self.shared.core.manifest
        }

        /// The block directory, in file order.
        pub fn entries(&self) -> &[BlockEntry] {
            &self.shared.core.entries
        }

        /// The configuration.
        pub fn config(&self) -> &ReaderConfig {
            &self.shared.config
        }

        /// The accountant: live bytes, the peak and the work done.
        pub fn accountant(&self) -> &Accountant {
            &self.shared.acct
        }

        /// Requests, bytes and cache counters so far.
        pub fn stats(&self) -> ReadStats {
            self.shared.stats()
        }

        /// What the open left out because this reader does not support it, or `None` when it left out
        /// nothing.
        pub fn partial_open(&self) -> Option<&PartialOpen> {
            self.shared.core.partial.as_ref()
        }

        /// Whether block `id` is in the decoded-block cache right now. It looks and does not touch: the
        /// block is not made the most recently used and no hit or miss is counted, so a planner can ask
        /// about many blocks without changing what it is asking about.
        pub fn is_cached(&self, id: BlockId) -> bool {
            self.shared.cache.lock().map(|c| c.map.contains_key(&id)).unwrap_or(false)
        }

        /// The requests that would fetch `ids`, coalescing blocks whose gap is within the configured
        /// threshold and never spanning more than the configured request size. Nothing is fetched.
        pub fn plan(&self, ids: &[BlockId]) -> Result<Vec<RangeRequest>, ReadError> {
            self.shared.core.plan(ids, self.shared.config.gap_threshold, self.shared.config.max_request_bytes)
        }
    };
}

/// The blocking reader.
pub struct ContainerReader<S: BlockSource> {
    source: S,
    shared: Shared,
}

impl<S: BlockSource> ContainerReader<S> {
    /// Opens an artifact: the first 4 KiB (header and, when it fits, the manifest), the manifest when it
    /// did not, and the directory. Everything fetched is checked before the next range is asked for.
    pub fn open(source: S, config: ReaderConfig) -> Result<ContainerReader<S>, ReadError> {
        config.limits.validate().map_err(ReadError::BadLimits)?;
        let counters = Counters::default();
        let acct = Accountant::new(&config.limits);
        let mut opening = Opening::new(BlockSource::len(&source), &open_options(&config), &acct)?;
        while let Some((offset, len)) = opening.request() {
            counters.requests.fetch_add(1, Ordering::Relaxed);
            counters.bytes.fetch_add(u64::from(len), Ordering::Relaxed);
            let bytes = checked(BlockSource::read_range(&source, offset, len)?, len)?;
            opening.feed(&bytes)?;
        }
        let core = opening.finish()?;
        Ok(ContainerReader { source, shared: Shared::new(core, config, counters, acct) })
    }

    accessors!();

    fn fetch(&self, offset: u64, len: u32) -> Result<Bytes, ReadError> {
        self.shared.note_request(len);
        checked(BlockSource::read_range(&self.source, offset, len)?, len)
    }

    /// One block, decoded and cached.
    pub fn read_block(&self, id: BlockId) -> Result<Arc<DecodedBlock>, ReadError> {
        self.shared.core.check_readable(id)?;
        if let Some(block) = self.shared.cached(id) {
            return Ok(block);
        }
        let (offset, len) = self.shared.core.range_of(id)?;
        let (decoded, _in_flight) = self.shared.admit(id)?;
        let stored = self.fetch(offset, len)?;
        self.shared.finish_block(id, &stored, decoded)
    }

    /// Several blocks, fetched through [`ContainerReader::plan`]: blocks close together arrive in one
    /// request. The result is in the order of `ids`.
    pub fn read_blocks(&self, ids: &[BlockId]) -> Result<Vec<Arc<DecodedBlock>>, ReadError> {
        let mut found: HashMap<BlockId, Arc<DecodedBlock>> = HashMap::new();
        let mut missing: Vec<BlockId> = Vec::new();
        for id in ids {
            self.shared.core.check_readable(*id)?;
            match self.shared.cached(*id) {
                Some(b) => {
                    found.insert(*id, b);
                }
                None => missing.push(*id),
            }
        }
        for request in self.plan(&missing)? {
            let (decoded, _in_flight) = self.shared.admit_request(&request)?;
            let bytes = self.fetch(request.offset, request.len)?;
            for (id, room) in request.blocks.iter().zip(decoded) {
                let stored = self.shared.core.slice_of(*id, request.offset, &bytes)?;
                found.insert(*id, self.shared.finish_block(*id, stored, room)?);
            }
        }
        ids.iter()
            .map(|id| {
                found
                    .get(id)
                    .cloned()
                    .ok_or(ReadError::NoSuchBlock { id: id.0, blocks: self.entries().len() })
            })
            .collect()
    }

    /// How many chunks the advanced curves are in (0 when the file has none).
    pub fn advanced_chunk_count(&self) -> Result<usize, ReadError> {
        self.shared.core.advanced_chunks().map(|c| c.len())
    }

    /// The record block of each chunk of the advanced curves, in chunk order: the `block_id` of a
    /// `PayloadRef` to a row of that chunk.
    pub fn advanced_record_blocks(&self) -> Result<Vec<BlockId>, ReadError> {
        self.shared.core.advanced_chunks().map(|c| c.iter().map(|six| six[0]).collect())
    }

    /// The six blocks of each of `chunks` (the record block, then the five arenas), chunk after chunk in the
    /// order given. A caller that wants several chunks puts these in the `read_blocks` call that fetches
    /// its hot blocks, so the planner coalesces the lot; `advanced` called chunk by chunk makes a plan of
    /// its own for each. A chunk the file does not have is `NoSuchBlock` with the chunk number in `id` and
    /// the number of chunks in `blocks`.
    pub fn advanced_chunk_blocks(&self, chunks: &[usize]) -> Result<Vec<BlockId>, ReadError> {
        let all = self.shared.core.advanced_chunks()?;
        let mut out = Vec::with_capacity(chunks.len() * 6);
        for &c in chunks {
            let six = all.get(c).ok_or(ReadError::NoSuchBlock { id: c as u32, blocks: all.len() })?;
            out.extend_from_slice(six);
        }
        Ok(out)
    }

    /// Chunk `chunk` of the advanced curves (its record block and five arenas, one range when they are
    /// adjacent), or `None` when the file has no such chunk. Each block is cached on its own.
    pub fn advanced(&self, chunk: usize) -> Result<Option<AdvancedSet>, ReadError> {
        let chunks = self.shared.core.advanced_chunks()?;
        let Some(ids) = chunks.get(chunk) else { return Ok(None) };
        let blocks = self.read_blocks(ids)?;
        self.shared.advanced_set(blocks).map(Some)
    }

    /// Hashes the whole artifact in bounded chunks and compares it with the digest in the manifest.
    pub fn verify_artifact_digest(&self) -> Result<DigestStatus, ReadError> {
        let mut check = DigestCheck::new(&self.shared.core);
        while let Some((offset, len)) = check.request() {
            let _room = self.shared.admit_gap(len)?;
            let bytes = self.fetch(offset, len)?;
            check.feed(&bytes);
        }
        check.finish()
    }

    /// The bytes of a block as they are stored, undecoded and unchecked, for a tool that carries a block
    /// it does not understand. Skipped blocks can be read this way, and only this way.
    pub fn read_stored(&self, id: BlockId) -> Result<Bytes, ReadError> {
        let (offset, len) = self.shared.core.range_of(id)?;
        let _room = self.shared.admit_gap(len)?;
        self.fetch(offset, len)
    }

    /// The rasters the directory names, by any kind 9 or kind 10 block, ascending.
    pub fn rasters(&self) -> Vec<u32> {
        self.shared.core.raster_ids()
    }

    /// The calibration of the file (kind 11, capability 22, ADR 0019): `None` when the file has none or this reader
    /// skipped it at open. The directory is checked first (`CAL-12`) and the entry's lengths are held to LIMIT-17
    /// before the block is fetched; the block is then read like any other and parsed. A refusal is an error for
    /// the calibration only: the file stays open and the geometry reads.
    pub fn calibration(&self) -> Result<Option<Calibration>, ReadError> {
        let entries = self.entries();
        let mut kind_11 = entries.iter().enumerate().filter(|(_, e)| e.kind == CALIBRATION_KIND);
        let Some((i, e)) = kind_11.next() else {
            return Ok(None);
        };
        if kind_11.next().is_some() || e.cell != 0 || e.subtype != 0 || e.flags != 0 {
            return Err(ReadError::Calibration(CalibrationError::Directory));
        }
        let id = BlockId(i as u32);
        if self.partial_open().is_some_and(|p| p.blocks.iter().any(|b| b.block == id)) {
            return Ok(None);
        }
        let limits = &self.shared.config.limits;
        limits.check(LimitKind::CalibrationBytes, u64::from(e.stored_length.max(e.uncompressed_length)))?;
        let block = self.read_block(id)?;
        decode_calibration(block.as_bytes()).map(Some).map_err(ReadError::Calibration)
    }

    /// The index of `raster`, read when it is first asked for and kept: the kind 9 block through
    /// [`ContainerReader::read_block`] (its accounting, its checksum), parsed and checked against the directory.
    /// A raster that is refused is an error for that raster; the file stays open and the others read.
    pub fn raster_index(&self, raster: u32) -> Result<Arc<RasterIndex>, ReadError> {
        if let Some(index) = self.shared.raster_cached(raster) {
            return Ok(index);
        }
        let placement = self.raster_placement(raster)?;
        let block = self.read_block(self.shared.raster_index_id(raster)?)?;
        self.shared.raster_parsed(raster, &block, &placement)
    }

    /// The placements of the raster stream of the component set, in `RasterId` order, read when first asked for.
    /// A file with no raster stream places none, and a damaged stream is an error for the rasters only.
    pub fn raster_placements(&self) -> Result<Arc<Vec<Placement>>, ReadError> {
        if let Some(p) = self.shared.placements_cached() {
            return p;
        }
        let block = match self.shared.components_block() {
            Some(id) => Some(self.read_block(id)?),
            None => None,
        };
        self.shared.placements_parsed(block.as_deref())
    }

    /// The placement of `raster`; a raster the stream does not place is `RASTER-06`.
    pub fn raster_placement(&self, raster: u32) -> Result<Placement, ReadError> {
        let all = self.raster_placements()?;
        all.get(raster as usize).cloned().ok_or(ReadError::Raster(IndexError::NoPlacement { raster }))
    }

    /// The encoded bytes of tile `(col, row)` of `level` of `raster`, after the index's XXH3-64 has been checked
    /// against them, whether or not the file carries a checksum table. A tile that is not stored is `None`.
    ///
    /// A tile in a chunk is a range of the chunk at the index's offset: the whole chunk is not read and
    /// [`ContainerReader::read_stored`] is not involved, so the range is held to the budget (`LIMIT-03`,
    /// `LIMIT-04`) like any block. The chunk's own checksum is full validation's to check.
    pub fn read_raster_tile(
        &self,
        raster: u32,
        level: u32,
        col: u32,
        row: u32,
    ) -> Result<Option<Bytes>, ReadError> {
        let placement = self.raster_placement(raster)?;
        let index = self.raster_index(raster)?;
        let at = (level, col, row);
        match self.shared.tile_plan(raster, &index, &placement, at)? {
            TilePlan::NotStored => Ok(None),
            TilePlan::Inline(t, bytes) => {
                self.shared.tile_verified(raster, &placement, at, &t, bytes).map(Some)
            }
            TilePlan::Range { tile, offset, len } => {
                let _room = self.shared.admit_gap(len)?;
                let bytes = self.fetch(offset, len)?;
                self.shared.tile_verified(raster, &placement, at, &tile, bytes).map(Some)
            }
        }
    }

    /// Everything about one raster that full validation can say: its index, the chunk blocks against it, and
    /// every stored tile against its checksum, each chunk read whole once. The first fault is the error.
    pub fn validate_raster(&self, raster: u32) -> Result<(), ReadError> {
        let placement = self.raster_placement(raster)?;
        let index = self.raster_index(raster)?;
        for l in 0..index.levels().len() as u32 {
            // the chunk numbers rise along the table, so each chunk block is read once, when its first tile comes
            let mut current: Option<(u32, Arc<DecodedBlock>)> = None;
            for t in index.level_tiles(l).into_iter().flatten().filter(|t| t.is_stored()) {
                let bytes = if t.is_inline() {
                    let tile = IndexError::Tile { level: l, col: t.col, row: t.row };
                    index.inline_bytes(t).ok_or(ReadError::Raster(tile))?.to_vec()
                } else {
                    if current.as_ref().is_none_or(|(c, _)| *c != t.chunk) {
                        let missing = IndexError::Chunk { level: l, chunk: t.chunk };
                        let id = self
                            .shared
                            .core
                            .raster_chunk_block(raster, l, t.chunk)
                            .ok_or(ReadError::Raster(missing))?;
                        current = Some((t.chunk, self.read_block(id)?));
                    }
                    let chunk = IndexError::Chunk { level: l, chunk: t.chunk };
                    let block = current.as_ref().map(|(_, b)| b).ok_or(ReadError::Raster(chunk.clone()))?;
                    let range = t.offset as usize..t.offset as usize + t.length as usize;
                    block.as_bytes().get(range).ok_or(ReadError::Raster(chunk))?.to_vec()
                };
                self.shared.tile_admitted(&placement, t)?;
                self.shared.tile_verified(raster, &placement, (l, t.col, t.row), t, Bytes::new(bytes))?;
            }
        }
        Ok(())
    }

    /// Reads and checks the whole file: everything `open` checks, the bytes between blocks, and every
    /// block against the parsers, the directory and each other. A file that passes is a canonical
    /// artifact.
    pub fn validate(&self) -> Result<Validated, ReadError> {
        let mut v = Validation::new(&self.shared.core, &self.shared.config.limits, &self.shared.acct)?;
        while let Some(need) = v.next_need() {
            match need {
                Need::Gap { offset, len } => {
                    let _room = self.shared.admit_gap(len)?;
                    let bytes = self.fetch(offset, len)?;
                    v.feed_gap(offset, &bytes)?;
                }
                Need::Block(id) => {
                    let block = self.read_block(id)?;
                    v.feed_block(block)?;
                }
            }
        }
        v.finish()
    }
}

/// The async reader: the same code as [`ContainerReader`], with the fetches awaited.
pub struct AsyncContainerReader<S: AsyncBlockSource> {
    source: S,
    shared: Shared,
}

impl<S: AsyncBlockSource> AsyncContainerReader<S> {
    /// As [`ContainerReader::open`].
    pub async fn open(source: S, config: ReaderConfig) -> Result<AsyncContainerReader<S>, ReadError> {
        config.limits.validate().map_err(ReadError::BadLimits)?;
        let counters = Counters::default();
        let acct = Accountant::new(&config.limits);
        let mut opening = Opening::new(AsyncBlockSource::len(&source), &open_options(&config), &acct)?;
        while let Some((offset, len)) = opening.request() {
            counters.requests.fetch_add(1, Ordering::Relaxed);
            counters.bytes.fetch_add(u64::from(len), Ordering::Relaxed);
            let bytes = checked(AsyncBlockSource::read_range(&source, offset, len).await?, len)?;
            opening.feed(&bytes)?;
        }
        let core = opening.finish()?;
        Ok(AsyncContainerReader { source, shared: Shared::new(core, config, counters, acct) })
    }

    accessors!();

    async fn fetch(&self, offset: u64, len: u32) -> Result<Bytes, ReadError> {
        self.shared.note_request(len);
        checked(AsyncBlockSource::read_range(&self.source, offset, len).await?, len)
    }

    /// As [`ContainerReader::read_block`].
    pub async fn read_block(&self, id: BlockId) -> Result<Arc<DecodedBlock>, ReadError> {
        self.shared.core.check_readable(id)?;
        if let Some(block) = self.shared.cached(id) {
            return Ok(block);
        }
        let (offset, len) = self.shared.core.range_of(id)?;
        let (decoded, _in_flight) = self.shared.admit(id)?;
        let stored = self.fetch(offset, len).await?;
        self.shared.finish_block(id, &stored, decoded)
    }

    /// As [`ContainerReader::read_blocks`].
    pub async fn read_blocks(&self, ids: &[BlockId]) -> Result<Vec<Arc<DecodedBlock>>, ReadError> {
        let mut found: HashMap<BlockId, Arc<DecodedBlock>> = HashMap::new();
        let mut missing: Vec<BlockId> = Vec::new();
        for id in ids {
            self.shared.core.check_readable(*id)?;
            match self.shared.cached(*id) {
                Some(b) => {
                    found.insert(*id, b);
                }
                None => missing.push(*id),
            }
        }
        for request in self.plan(&missing)? {
            let (decoded, _in_flight) = self.shared.admit_request(&request)?;
            let bytes = self.fetch(request.offset, request.len).await?;
            for (id, room) in request.blocks.iter().zip(decoded) {
                let stored = self.shared.core.slice_of(*id, request.offset, &bytes)?;
                found.insert(*id, self.shared.finish_block(*id, stored, room)?);
            }
        }
        ids.iter()
            .map(|id| {
                found
                    .get(id)
                    .cloned()
                    .ok_or(ReadError::NoSuchBlock { id: id.0, blocks: self.entries().len() })
            })
            .collect()
    }

    /// As [`ContainerReader::advanced_chunk_count`].
    pub fn advanced_chunk_count(&self) -> Result<usize, ReadError> {
        self.shared.core.advanced_chunks().map(|c| c.len())
    }

    /// As [`ContainerReader::advanced`].
    pub async fn advanced(&self, chunk: usize) -> Result<Option<AdvancedSet>, ReadError> {
        let chunks = self.shared.core.advanced_chunks()?;
        let Some(ids) = chunks.get(chunk) else { return Ok(None) };
        let blocks = self.read_blocks(ids).await?;
        self.shared.advanced_set(blocks).map(Some)
    }

    /// As [`ContainerReader::verify_artifact_digest`].
    pub async fn verify_artifact_digest(&self) -> Result<DigestStatus, ReadError> {
        let mut check = DigestCheck::new(&self.shared.core);
        while let Some((offset, len)) = check.request() {
            let _room = self.shared.admit_gap(len)?;
            let bytes = self.fetch(offset, len).await?;
            check.feed(&bytes);
        }
        check.finish()
    }

    /// As [`ContainerReader::read_stored`].
    pub async fn read_stored(&self, id: BlockId) -> Result<Bytes, ReadError> {
        let (offset, len) = self.shared.core.range_of(id)?;
        let _room = self.shared.admit_gap(len)?;
        self.fetch(offset, len).await
    }

    /// The rasters the directory names, by any kind 9 or kind 10 block, ascending.
    pub fn rasters(&self) -> Vec<u32> {
        self.shared.core.raster_ids()
    }

    /// The index of `raster`, read when it is first asked for and kept: the kind 9 block through
    /// [`AsyncContainerReader::read_block`] (its accounting, its checksum), parsed and checked against the directory.
    /// A raster that is refused is an error for that raster; the file stays open and the others read.
    pub async fn raster_index(&self, raster: u32) -> Result<Arc<RasterIndex>, ReadError> {
        if let Some(index) = self.shared.raster_cached(raster) {
            return Ok(index);
        }
        let placement = self.raster_placement(raster).await?;
        let block = self.read_block(self.shared.raster_index_id(raster)?).await?;
        self.shared.raster_parsed(raster, &block, &placement)
    }

    /// As [`ContainerReader::raster_placements`].
    pub async fn raster_placements(&self) -> Result<Arc<Vec<Placement>>, ReadError> {
        if let Some(p) = self.shared.placements_cached() {
            return p;
        }
        let block = match self.shared.components_block() {
            Some(id) => Some(self.read_block(id).await?),
            None => None,
        };
        self.shared.placements_parsed(block.as_deref())
    }

    /// As [`ContainerReader::raster_placement`].
    pub async fn raster_placement(&self, raster: u32) -> Result<Placement, ReadError> {
        let all = self.raster_placements().await?;
        all.get(raster as usize).cloned().ok_or(ReadError::Raster(IndexError::NoPlacement { raster }))
    }

    /// The encoded bytes of tile `(col, row)` of `level` of `raster`, after the index's XXH3-64 has been checked
    /// against them, whether or not the file carries a checksum table. A tile that is not stored is `None`.
    ///
    /// A tile in a chunk is a range of the chunk at the index's offset: the whole chunk is not read and
    /// [`AsyncContainerReader::read_stored`] is not involved, so the range is held to the budget (`LIMIT-03`,
    /// `LIMIT-04`) like any block. The chunk's own checksum is full validation's to check.
    pub async fn read_raster_tile(
        &self,
        raster: u32,
        level: u32,
        col: u32,
        row: u32,
    ) -> Result<Option<Bytes>, ReadError> {
        let placement = self.raster_placement(raster).await?;
        let index = self.raster_index(raster).await?;
        let at = (level, col, row);
        match self.shared.tile_plan(raster, &index, &placement, at)? {
            TilePlan::NotStored => Ok(None),
            TilePlan::Inline(t, bytes) => {
                self.shared.tile_verified(raster, &placement, at, &t, bytes).map(Some)
            }
            TilePlan::Range { tile, offset, len } => {
                let _room = self.shared.admit_gap(len)?;
                let bytes = self.fetch(offset, len).await?;
                self.shared.tile_verified(raster, &placement, at, &tile, bytes).map(Some)
            }
        }
    }

    /// Everything about one raster that full validation can say: its index, the chunk blocks against it, and
    /// every stored tile against its checksum, each chunk read whole once. The first fault is the error.
    pub async fn validate_raster(&self, raster: u32) -> Result<(), ReadError> {
        let placement = self.raster_placement(raster).await?;
        let index = self.raster_index(raster).await?;
        for l in 0..index.levels().len() as u32 {
            // the chunk numbers rise along the table, so each chunk block is read once, when its first tile comes
            let mut current: Option<(u32, Arc<DecodedBlock>)> = None;
            for t in index.level_tiles(l).into_iter().flatten().filter(|t| t.is_stored()) {
                let bytes = if t.is_inline() {
                    let tile = IndexError::Tile { level: l, col: t.col, row: t.row };
                    index.inline_bytes(t).ok_or(ReadError::Raster(tile))?.to_vec()
                } else {
                    if current.as_ref().is_none_or(|(c, _)| *c != t.chunk) {
                        let missing = IndexError::Chunk { level: l, chunk: t.chunk };
                        let id = self
                            .shared
                            .core
                            .raster_chunk_block(raster, l, t.chunk)
                            .ok_or(ReadError::Raster(missing))?;
                        current = Some((t.chunk, self.read_block(id).await?));
                    }
                    let chunk = IndexError::Chunk { level: l, chunk: t.chunk };
                    let block = current.as_ref().map(|(_, b)| b).ok_or(ReadError::Raster(chunk.clone()))?;
                    let range = t.offset as usize..t.offset as usize + t.length as usize;
                    block.as_bytes().get(range).ok_or(ReadError::Raster(chunk))?.to_vec()
                };
                self.shared.tile_admitted(&placement, t)?;
                self.shared.tile_verified(raster, &placement, (l, t.col, t.row), t, Bytes::new(bytes))?;
            }
        }
        Ok(())
    }

    /// As [`ContainerReader::validate`].
    pub async fn validate(&self) -> Result<Validated, ReadError> {
        let mut v = Validation::new(&self.shared.core, &self.shared.config.limits, &self.shared.acct)?;
        while let Some(need) = v.next_need() {
            match need {
                Need::Gap { offset, len } => {
                    let _room = self.shared.admit_gap(len)?;
                    let bytes = self.fetch(offset, len).await?;
                    v.feed_gap(offset, &bytes)?;
                }
                Need::Block(id) => {
                    let block = self.read_block(id).await?;
                    v.feed_block(block)?;
                }
            }
        }
        v.finish()
    }
}

/// `docs/READER.md`, rendered from the constants and the rule table.
pub fn render_markdown() -> String {
    crate::engine::render_reader_doc()
}

/// The refusals of this module that are not the parsers' own, for the document.
pub const READER_RULES: &[(&str, &str)] = crate::engine::READER_RULES;
