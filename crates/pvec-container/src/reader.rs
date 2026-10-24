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
use crate::directory::BlockEntry;
use crate::directory::DirectoryError;
use crate::engine::{Core, Need, Opening, Validation};
use crate::limits::{Accountant, LimitExceeded, Limits, LimitsError, Reservation};
use crate::source::{AsyncBlockSource, BlockSource, Bytes, SourceError};
use crate::{ContainerError, Header, Manifest};
use pvec_components::StreamError;
use std::collections::HashMap;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReaderConfig {
    /// The ceilings a file is held to.
    pub limits: Limits,
    /// The bytes of decoded blocks the cache may hold. Zero turns the cache off.
    pub cache_bytes: u64,
    /// Blocks whose gap is at most this many bytes are fetched in one request, reading the gap too.
    pub gap_threshold: u32,
    /// The most bytes one coalesced request may span.
    pub max_request_bytes: u32,
}

impl Default for ReaderConfig {
    fn default() -> ReaderConfig {
        ReaderConfig {
            limits: Limits::DEFAULT,
            cache_bytes: 64 << 20,
            gap_threshold: 64 << 10,
            max_request_bytes: 16 << 20,
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
/// only; the bytes go back to the accountant when the last caller lets go of the block.
struct Cache {
    map: HashMap<BlockId, CacheEntry>,
    bytes: u64,
    budget: u64,
    tick: u64,
}

impl Cache {
    fn new(budget: u64) -> Cache {
        Cache { map: HashMap::new(), bytes: 0, budget, tick: 0 }
    }

    fn get(&mut self, id: BlockId) -> Option<Arc<DecodedBlock>> {
        self.tick += 1;
        let tick = self.tick;
        self.map.get_mut(&id).map(|e| {
            e.tick = tick;
            Arc::clone(&e.block)
        })
    }

    fn cost(block: &DecodedBlock) -> u64 {
        block.as_bytes().len() as u64
    }

    /// Drops the least recently used entry. False when there is none.
    fn evict_one(&mut self) -> bool {
        let Some(victim) = self.map.iter().min_by_key(|(_, e)| e.tick).map(|(id, _)| *id) else {
            return false;
        };
        if let Some(e) = self.map.remove(&victim) {
            self.bytes -= Cache::cost(&e.block);
        }
        true
    }

    fn insert(&mut self, block: Arc<DecodedBlock>) -> u64 {
        let cost = Cache::cost(&block);
        if cost > self.budget {
            return 0;
        }
        let mut evicted = 0;
        if let Some(old) = self.map.remove(&block.id()) {
            self.bytes -= Cache::cost(&old.block);
        }
        while self.bytes + cost > self.budget && self.evict_one() {
            evicted += 1;
        }
        self.tick += 1;
        self.bytes += cost;
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
}

impl Shared {
    fn new(core: Core, config: ReaderConfig, counters: Counters, acct: Accountant) -> Shared {
        Shared { cache: Mutex::new(Cache::new(config.cache_bytes)), core, config, counters, acct }
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
                    let evicted = self.cache.lock().map(|mut c| c.evict_one()).unwrap_or(false);
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
    fn admit(&self, id: BlockId, in_flight: u64) -> Result<(Reservation, Reservation), ReadError> {
        let entry = self.core.entry(id)?;
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

    /// For each request of a plan: the room for what it fetches, reserved before it is asked for.
    fn admit_request(&self, request: &RangeRequest) -> Result<(Vec<Reservation>, Reservation), ReadError> {
        let mut decoded = Vec::with_capacity(request.blocks.len());
        for id in &request.blocks {
            let entry = self.core.entry(*id)?;
            self.acct.charge_work(u64::from(entry.stored_length) + u64::from(entry.uncompressed_length))?;
            decoded.push(self.reserve(u64::from(entry.uncompressed_length))?);
        }
        let transient = self.reserve(u64::from(request.len))?;
        Ok((decoded, transient))
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
        let mut opening = Opening::new(BlockSource::len(&source), &config.limits, &acct)?;
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
        self.shared.core.entry(id)?;
        if let Some(block) = self.shared.cached(id) {
            return Ok(block);
        }
        let (offset, len) = self.shared.core.range_of(id)?;
        let (decoded, _in_flight) = self.shared.admit(id, u64::from(len))?;
        let stored = self.fetch(offset, len)?;
        self.shared.finish_block(id, &stored, decoded)
    }

    /// Several blocks, fetched through [`ContainerReader::plan`]: blocks close together arrive in one
    /// request. The result is in the order of `ids`.
    pub fn read_blocks(&self, ids: &[BlockId]) -> Result<Vec<Arc<DecodedBlock>>, ReadError> {
        let mut found: HashMap<BlockId, Arc<DecodedBlock>> = HashMap::new();
        let mut missing: Vec<BlockId> = Vec::new();
        for id in ids {
            self.shared.core.entry(*id)?;
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

    /// The advanced curves (the record block and the five arenas), or `None` when the file has none.
    pub fn advanced(&self) -> Result<Option<AdvancedSet>, ReadError> {
        let Some(ids) = self.shared.core.advanced_ids()? else { return Ok(None) };
        let blocks = self.read_blocks(&ids)?;
        self.shared.advanced_set(blocks).map(Some)
    }

    /// Reads and checks the whole file: everything `open` checks, the bytes between blocks, and every
    /// block against the parsers, the directory and each other. A file that passes is a canonical
    /// artifact.
    pub fn validate(&self) -> Result<Validated, ReadError> {
        let mut v = Validation::new(&self.shared.core, &self.shared.config.limits, &self.shared.acct)?;
        while let Some(need) = v.next_need() {
            match need {
                Need::Gap { offset, len } => {
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
        let mut opening = Opening::new(AsyncBlockSource::len(&source), &config.limits, &acct)?;
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
        self.shared.core.entry(id)?;
        if let Some(block) = self.shared.cached(id) {
            return Ok(block);
        }
        let (offset, len) = self.shared.core.range_of(id)?;
        let (decoded, _in_flight) = self.shared.admit(id, u64::from(len))?;
        let stored = self.fetch(offset, len).await?;
        self.shared.finish_block(id, &stored, decoded)
    }

    /// As [`ContainerReader::read_blocks`].
    pub async fn read_blocks(&self, ids: &[BlockId]) -> Result<Vec<Arc<DecodedBlock>>, ReadError> {
        let mut found: HashMap<BlockId, Arc<DecodedBlock>> = HashMap::new();
        let mut missing: Vec<BlockId> = Vec::new();
        for id in ids {
            self.shared.core.entry(*id)?;
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

    /// As [`ContainerReader::advanced`].
    pub async fn advanced(&self) -> Result<Option<AdvancedSet>, ReadError> {
        let Some(ids) = self.shared.core.advanced_ids()? else { return Ok(None) };
        let blocks = self.read_blocks(&ids).await?;
        self.shared.advanced_set(blocks).map(Some)
    }

    /// As [`ContainerReader::validate`].
    pub async fn validate(&self) -> Result<Validated, ReadError> {
        let mut v = Validation::new(&self.shared.core, &self.shared.config.limits, &self.shared.acct)?;
        while let Some(need) = v.next_need() {
            match need {
                Need::Gap { offset, len } => {
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
