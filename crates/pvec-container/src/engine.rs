//! The checks of the reader, sans IO (issue P4.5).
//!
//! Nothing here reads a file or awaits anything. Each piece says which range it needs next and is fed the
//! bytes that came back, so the blocking reader and the async reader are the same loop around this code
//! and the checks exist exactly once:
//!
//! * [`Opening`] fetches the first 4 KiB (header, and the manifest when it fits), the manifest when it
//!   did not, and the block directory, and checks each against the artifact length and the limits before
//!   the next range is asked for;
//! * [`Core`] is what opening produces: the checked layout, with the range of a block, the read plan and
//!   the decoding of one block;
//! * [`Validation`] is the whole-file check: the bytes between blocks and every block against the parsers,
//!   the directory and each other.

use crate::blocks::AlignedBytes;
use crate::blocks::advanced::{AdvancedView, BlockSlices, CurveView};
use crate::blocks::arena::ArenaError;
use crate::directory::{
    BLOCK_ENTRY_FIELDS, BlockEntry, BlockKind, CODEC_NONE, ENTRY_LEN, parse_directory_checked,
};
use crate::header::Header;
use crate::limits::{Accountant, LimitExceeded, LimitKind, Limits, Reservation};
use crate::manifest::{ByteRange, DirectoryPointer, Manifest};
use crate::reader::{BlockId, DecodedBlock, RangeRequest, ReadError, Validated};
use pvec_capabilities::{Reader, Registry, Version};
use pvec_components::read_streams;
use std::sync::Arc;

/// The first fetch: the most a header may be, which also holds the manifest of any ordinary file.
const PREFIX_LEN: u64 = Header::MAX_LEN as u64;
/// The most bytes of padding one fetch reads during a full validation.
const GAP_CHUNK: u64 = 1 << 20;

fn layout<T>(rule: &'static str, detail: impl Into<String>) -> Result<T, ReadError> {
    Err(ReadError::Layout { rule, detail: detail.into() })
}

/// What opening has fetched so far.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Prefix,
    Manifest,
    Directory,
    Done,
}

/// The opening sequence: header prefix, manifest, directory. See the module documentation.
pub(crate) struct Opening {
    limits: Limits,
    acct: Accountant,
    artifact_len: u64,
    stage: Stage,
    header: Option<Header>,
    manifest: Option<Manifest>,
    directory: Option<ByteRange>,
    entries: Vec<BlockEntry>,
    held: Option<Reservation>,
}

impl Opening {
    /// Starts an opening of an artifact of `artifact_len` bytes. An artifact over the ceiling is refused
    /// before anything is asked of the source.
    pub fn new(artifact_len: u64, limits: &Limits, acct: &Accountant) -> Result<Opening, ReadError> {
        limits.check(LimitKind::ArtifactBytes, artifact_len)?;
        Ok(Opening {
            limits: *limits,
            acct: acct.clone(),
            artifact_len,
            stage: Stage::Prefix,
            header: None,
            manifest: None,
            directory: None,
            entries: Vec::new(),
            held: None,
        })
    }

    /// The next range to fetch, or `None` when the opening is complete.
    pub fn request(&self) -> Option<(u64, u32)> {
        match self.stage {
            Stage::Prefix => Some((0, self.artifact_len.min(PREFIX_LEN) as u32)),
            Stage::Manifest => {
                let h = self.header.as_ref()?;
                Some((h.manifest_offset, h.manifest_length as u32))
            }
            Stage::Directory => {
                let d = self.directory?;
                Some((d.offset, d.length as u32))
            }
            Stage::Done => None,
        }
    }

    /// Takes the bytes of the range [`Opening::request`] named.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), ReadError> {
        self.acct.charge_work(bytes.len() as u64)?;
        match self.stage {
            Stage::Prefix => {
                let _transient = self.acct.reserve(bytes.len() as u64)?;
                let header = Header::parse(bytes, self.artifact_len).map_err(ReadError::Container)?;
                if header.manifest_offset != u64::from(header.header_len) {
                    return layout(
                        "LAYOUT-01",
                        format!(
                            "the manifest starts at {}, not straight after the header",
                            header.manifest_offset
                        ),
                    );
                }
                let end = header.manifest_offset + header.manifest_length;
                if end <= bytes.len() as u64 {
                    let at = header.manifest_offset as usize;
                    self.after_header(header, &bytes[at..end as usize])
                } else {
                    self.stage = Stage::Manifest;
                    self.header = Some(header);
                    Ok(())
                }
            }
            Stage::Manifest => {
                let _transient = self.acct.reserve(bytes.len() as u64)?;
                let header = self.header.take().ok_or_else(internal)?;
                self.after_header(header, bytes)
            }
            Stage::Directory => self.directory_fed(bytes),
            Stage::Done => Ok(()),
        }
    }

    fn after_header(&mut self, header: Header, manifest_bytes: &[u8]) -> Result<(), ReadError> {
        let manifest =
            Manifest::parse(manifest_bytes, &header, self.artifact_len).map_err(ReadError::Container)?;
        if manifest.directories.len() != 1 || manifest.directories[0].kind != DirectoryPointer::KIND_BLOCK {
            return layout(
                "LAYOUT-03",
                format!("expected one block directory pointer, got {:?}", manifest.directories),
            );
        }
        if manifest.capabilities.windows(2).any(|w| w[0].id >= w[1].id) {
            return layout("LAYOUT-02", "capabilities are not strictly ascending by id");
        }
        let ptr = manifest.directories[0];
        if ptr.offset.checked_add(ptr.length) != Some(self.artifact_len) {
            return layout(
                "LAYOUT-03",
                format!(
                    "the directory ends at {}, the file at {}",
                    ptr.offset.saturating_add(ptr.length),
                    self.artifact_len
                ),
            );
        }
        // the directory is fetched whole, so its size is bounded before it is asked for
        let entries = ptr.length / ENTRY_LEN as u64;
        self.limits.check(LimitKind::Blocks, entries)?;
        if ptr.length > u64::from(u32::MAX) {
            // a range request is 32 bits long, whatever the configured ceiling on blocks says
            return Err(ReadError::Limit(LimitExceeded {
                limit: LimitKind::Blocks,
                declared: entries,
                ceiling: u64::from(u32::MAX) / ENTRY_LEN as u64,
            }));
        }
        self.directory = Some(ByteRange { offset: ptr.offset, length: ptr.length });
        self.header = Some(header);
        self.manifest = Some(manifest);
        self.stage = Stage::Directory;
        Ok(())
    }

    fn directory_fed(&mut self, table: &[u8]) -> Result<(), ReadError> {
        let header = self.header.as_ref().ok_or_else(internal)?;
        let ptr = self.directory.ok_or_else(internal)?;
        let n = table.len() / ENTRY_LEN;
        // the table, and the entries parsed from it, are held for the life of the reader
        let held = self.acct.reserve((table.len() + n * core::mem::size_of::<BlockEntry>()) as u64)?;
        check_table(table, self.artifact_len, &self.limits)?;
        let manifest_range = ByteRange { offset: header.manifest_offset, length: header.manifest_length };
        let entries = parse_directory_checked(
            table,
            u64::from(header.header_len),
            self.artifact_len,
            manifest_range,
            ptr,
        )
        .map_err(ReadError::Directory)?;
        if entries.is_empty() {
            return layout("LAYOUT-06", "no blocks");
        }
        let keys: Vec<(u16, u32, u16)> = entries.iter().map(|e| (e.kind, e.cell, e.subtype)).collect();
        if keys.windows(2).any(|w| w[0] >= w[1]) {
            return layout(
                "LAYOUT-04",
                format!("blocks are not in strictly ascending (kind, cell, geometry type) order: {keys:?}"),
            );
        }
        self.entries = entries;
        self.held = Some(held);
        self.stage = Stage::Done;
        Ok(())
    }

    /// The checked layout.
    pub fn finish(self) -> Result<Core, ReadError> {
        match (self.stage, self.header, self.manifest, self.directory, self.held) {
            (Stage::Done, Some(header), Some(manifest), Some(directory), Some(held)) => {
                let manifest_range =
                    ByteRange { offset: header.manifest_offset, length: header.manifest_length };
                Ok(Core {
                    header,
                    manifest,
                    entries: self.entries,
                    directory,
                    manifest_range,
                    limits: self.limits,
                    _held: held,
                })
            }
            _ => Err(internal()),
        }
    }
}

fn internal() -> ReadError {
    ReadError::Layout { rule: "INTERNAL", detail: "the opening was driven out of order".into() }
}

/// A little-endian integer out of an entry. A field that is not there reads as the largest value, so a
/// limit check that cannot see its field refuses and never passes.
fn le(bytes: &[u8], at: usize, size: usize) -> u64 {
    bytes.get(at..at + size).map_or(u64::MAX, |b| b.iter().rev().fold(0u64, |a, x| (a << 8) | u64::from(*x)))
}

/// The limits, applied to the raw entries before anything is parsed or sized from them. A refusal here
/// is a refusal of what the file *declares*: nothing has been fetched or reserved for it.
fn check_table(table: &[u8], artifact_len: u64, limits: &Limits) -> Result<(), ReadError> {
    let at = |name: &str| {
        let f = BLOCK_ENTRY_FIELDS.iter().find(|f| f.name == name);
        // a field that is missing from the table puts its reads out of range, where they refuse
        f.map_or((usize::MAX / 2, 8), |f| (f.offset, f.size))
    };
    let (offset, stored, decoded) = (at("offset"), at("stored_length"), at("uncompressed_length"));
    let (count, kind) = (at("count"), at("kind"));
    let mut entities = 0u64;
    for entry in table.chunks_exact(ENTRY_LEN) {
        let (o, s, d) =
            (le(entry, offset.0, offset.1), le(entry, stored.0, stored.1), le(entry, decoded.0, decoded.1));
        // a declared expansion is judged before anything else, so a bomb is refused for its ratio
        let ceiling = s.saturating_mul(u64::from(limits.max_expansion_ratio));
        if d > ceiling {
            return Err(ReadError::Limit(LimitExceeded {
                limit: LimitKind::ExpansionRatio,
                declared: d,
                ceiling,
            }));
        }
        limits.check(LimitKind::BlockBytes, s)?;
        limits.check(LimitKind::BlockBytes, d)?;
        match o.checked_add(s) {
            Some(end) if end <= artifact_len => {}
            end => {
                return Err(ReadError::Limit(LimitExceeded {
                    limit: LimitKind::Extent,
                    declared: end.unwrap_or(u64::MAX),
                    ceiling: artifact_len,
                }));
            }
        }
        let k = le(entry, kind.0, kind.1);
        if k == u64::from(BlockKind::Hot.id()) || k == u64::from(BlockKind::Records.id()) {
            let c = le(entry, count.0, count.1);
            limits.check(LimitKind::EntitiesPerBlock, c)?;
            entities = entities.saturating_add(c);
        }
    }
    limits.check(LimitKind::EntitiesPerArtifact, entities)?;
    Ok(())
}

/// The checked layout of an artifact.
pub(crate) struct Core {
    pub header: Header,
    pub manifest: Manifest,
    pub entries: Vec<BlockEntry>,
    pub directory: ByteRange,
    pub manifest_range: ByteRange,
    pub limits: Limits,
    _held: Reservation,
}

impl Core {
    /// The directory entry of a block.
    pub fn entry(&self, id: BlockId) -> Result<&BlockEntry, ReadError> {
        self.entries.get(id.0 as usize).ok_or(ReadError::NoSuchBlock { id: id.0, blocks: self.entries.len() })
    }

    /// Where a block is stored.
    pub fn range_of(&self, id: BlockId) -> Result<(u64, u32), ReadError> {
        self.entry(id).map(|e| (e.offset, e.stored_length))
    }

    /// The stored bytes of block `id` out of the bytes of a request that started at `request_offset`.
    pub fn slice_of<'a>(
        &self,
        id: BlockId,
        request_offset: u64,
        bytes: &'a [u8],
    ) -> Result<&'a [u8], ReadError> {
        let e = self.entry(id)?;
        let start = e.offset.checked_sub(request_offset).and_then(|s| usize::try_from(s).ok());
        let end = start.and_then(|s| s.checked_add(e.stored_length as usize));
        match (start, end) {
            (Some(s), Some(e)) if e <= bytes.len() => Ok(&bytes[s..e]),
            _ => Err(ReadError::Source(crate::source::SourceError::OutOfRange {
                offset: e.offset,
                len: u64::from(e.stored_length),
                source_len: request_offset + bytes.len() as u64,
            })),
        }
    }

    /// The requests that fetch `ids`. Blocks whose gap is at most `gap` bytes share a request while the
    /// request stays within `max` bytes; one block longer than `max` is a request of its own.
    pub fn plan(&self, ids: &[BlockId], gap: u32, max: u32) -> Result<Vec<RangeRequest>, ReadError> {
        let mut wanted: Vec<BlockId> = ids.to_vec();
        for id in &wanted {
            self.entry(*id)?;
        }
        // the directory is in file order, so the id order is the offset order
        wanted.sort_unstable();
        wanted.dedup();
        let mut out: Vec<RangeRequest> = Vec::new();
        let mut current: Option<(u64, u64, Vec<BlockId>)> = None;
        for id in wanted {
            let e = self.entry(id)?;
            let (start, end) = (e.offset, e.offset + u64::from(e.stored_length));
            current = match current.take() {
                Some((cs, ce, mut blocks))
                    if start.saturating_sub(ce) <= u64::from(gap) && end - cs <= u64::from(max) =>
                {
                    blocks.push(id);
                    Some((cs, ce.max(end), blocks))
                }
                Some(done) => {
                    out.push(request(done));
                    Some((start, end, vec![id]))
                }
                None => Some((start, end, vec![id])),
            };
        }
        out.extend(current.map(request));
        Ok(out)
    }

    /// Refuses a block whose codec this reader cannot decode, before anything is charged or fetched.
    pub fn check_decodable(&self, entry: &BlockEntry) -> Result<(), ReadError> {
        if entry.codec != CODEC_NONE {
            return layout("NEGOTIATE-01", format!("cannot decode codec {}", entry.codec));
        }
        Ok(())
    }

    /// Decodes the stored bytes of block `id` into aligned memory held against `reservation`.
    pub fn decode(
        &self,
        id: BlockId,
        stored: &[u8],
        reservation: Reservation,
    ) -> Result<DecodedBlock, ReadError> {
        let entry = *self.entry(id)?;
        if stored.len() as u64 != u64::from(entry.stored_length) {
            return Err(ReadError::Source(crate::source::SourceError::Short {
                wanted: u64::from(entry.stored_length),
                got: stored.len() as u64,
            }));
        }
        self.check_decodable(&entry)?;
        let bytes = AlignedBytes::from_bytes(stored);
        precheck_counts(&entry, bytes.as_bytes(), &self.limits)?;
        Ok(DecodedBlock::new(id, entry, bytes, reservation))
    }

    /// The six blocks of the advanced curves, or `None` when the file has none. A file with some of them
    /// and not all is refused.
    pub fn advanced_ids(&self) -> Result<Option<Vec<BlockId>>, ReadError> {
        let find = |kind: BlockKind, subtype: u16| {
            self.entries
                .iter()
                .position(|e| e.block_kind() == Some(kind) && e.subtype == subtype)
                .map(|i| BlockId(i as u32))
        };
        let wanted = [
            (BlockKind::Records, 0),
            (BlockKind::Arena, 1),
            (BlockKind::Arena, 2),
            (BlockKind::Arena, 3),
            (BlockKind::Arena, 4),
            (BlockKind::Arena, 5),
        ];
        let found: Vec<Option<BlockId>> = wanted.iter().map(|(k, s)| find(*k, *s)).collect();
        if found.iter().all(Option::is_none) {
            return Ok(None);
        }
        let mut out = Vec::with_capacity(6);
        for ((kind, subtype), f) in wanted.iter().zip(found) {
            match f {
                Some(id) => out.push(id),
                None => return layout("CONSIST-04", format!("no block {}/{subtype}", kind.id())),
            }
        }
        Ok(Some(out))
    }
}

fn request((start, end, blocks): (u64, u64, Vec<BlockId>)) -> RangeRequest {
    RangeRequest { offset: start, len: (end - start) as u32, blocks }
}

/// Holds the entity count a block declares in its own header to the ceiling before the block is parsed,
/// so a count of 2^32 - 1 is refused as a limit whatever the parser would have said. The directory's
/// own count was held to it when the file was opened.
fn precheck_counts(entry: &BlockEntry, bytes: &[u8], limits: &Limits) -> Result<(), ReadError> {
    let magic: Option<&[u8; 4]> = match entry.block_kind() {
        Some(BlockKind::Hot) => Some(b"PVH1"),
        Some(BlockKind::Records) => Some(b"PVR1"),
        Some(BlockKind::Arena) => Some(b"PVA1"),
        _ => None,
    };
    if let Some(magic) = magic {
        if bytes.len() >= 12 && &bytes[..4] == magic {
            limits.check(LimitKind::EntitiesPerBlock, le(bytes, 8, 4))?;
        }
    }
    Ok(())
}

/// Holds the nesting of the composites to `max_depth` before any segment is resolved. A chain over the
/// ceiling is a limit; a composite that leads back to itself has no depth and stays a cycle.
pub(crate) fn check_nesting(view: &AdvancedView<'_>, max_depth: usize) -> Result<(), ReadError> {
    match view.check_nesting_within(max_depth) {
        Ok(()) => Ok(()),
        Err(ArenaError::NestingTooDeep { .. }) => Err(ReadError::Limit(LimitExceeded {
            limit: LimitKind::NestingDepth,
            declared: max_depth as u64 + 1,
            ceiling: max_depth as u64,
        })),
        Err(e) => Err(ReadError::Arena(e)),
    }
}

/// The curve at `index`, after the counts its offset-table entries declare were held to the limits.
pub(crate) fn checked_curve<'a>(
    view: &AdvancedView<'a>,
    limits: &Limits,
    index: usize,
) -> Result<CurveView<'a>, ReadError> {
    if let Some((knots, control_points)) = view.declared_counts(index).map_err(ReadError::Arena)? {
        limits.check(LimitKind::ControlPoints, u64::from(control_points))?;
        limits.check(LimitKind::Knots, u64::from(knots))?;
    }
    view.curve(index).map_err(ReadError::Arena)
}

/// What a full validation needs next.
#[derive(Clone, Copy)]
pub(crate) enum Need {
    /// Padding between blocks: `len` bytes at `offset`, which must be zero.
    Gap { offset: u64, len: u32 },
    /// A block, to be read and decoded.
    Block(BlockId),
}

/// The whole-file check, driven by what it asks for. Padding comes first, then every block in file order.
pub(crate) struct Validation<'a> {
    core: &'a Core,
    limits: Limits,
    needs: Vec<Need>,
    next: usize,
    ids: Vec<u32>,
    _ids_reservation: Reservation,
    _needs_reservation: Reservation,
    expected_entities: u64,
    entities: u64,
    advanced: Vec<Arc<DecodedBlock>>,
    record_count: usize,
    streams: Vec<u32>,
}

impl<'a> Validation<'a> {
    pub fn new(core: &'a Core, limits: &Limits, acct: &Accountant) -> Result<Validation<'a>, ReadError> {
        // every byte between the manifest, the blocks and the directory
        let mut needs: Vec<Need> = Vec::new();
        let mut at = core.manifest_range.offset + core.manifest_range.length;
        for e in &core.entries {
            if e.offset < at {
                return layout(
                    "LAYOUT-07",
                    format!("the block at {} overlaps what comes before it", e.offset),
                );
            }
            push_gap(&mut needs, at, e.offset);
            at = e.offset + u64::from(e.stored_length);
        }
        if at > core.directory.offset {
            return layout("LAYOUT-07", "the blocks run into the directory");
        }
        push_gap(&mut needs, at, core.directory.offset);
        needs.extend((0..core.entries.len()).map(|i| Need::Block(BlockId(i as u32))));
        let expected_entities: u64 = core
            .entries
            .iter()
            .filter(|e| matches!(e.block_kind(), Some(BlockKind::Hot | BlockKind::Records)))
            .map(|e| u64::from(e.count))
            .sum();
        limits.check(LimitKind::EntitiesPerArtifact, expected_entities)?;
        // one id per entity, held until the identities have been compared
        let reservation = acct.reserve(4 * expected_entities)?;
        let needs_reservation = acct.reserve((needs.len() * core::mem::size_of::<Need>()) as u64)?;
        Ok(Validation {
            core,
            limits: *limits,
            needs,
            next: 0,
            ids: Vec::new(),
            _ids_reservation: reservation,
            _needs_reservation: needs_reservation,
            expected_entities,
            entities: 0,
            advanced: Vec::new(),
            record_count: 0,
            streams: Vec::new(),
        })
    }

    pub fn next_need(&mut self) -> Option<Need> {
        let need = self.needs.get(self.next).copied();
        self.next += 1;
        need
    }

    pub fn feed_gap(&mut self, offset: u64, bytes: &[u8]) -> Result<(), ReadError> {
        if bytes.iter().any(|b| *b != 0) {
            return layout("LAYOUT-08", format!("non-zero padding in the bytes at {offset}"));
        }
        Ok(())
    }

    pub fn feed_block(&mut self, block: Arc<DecodedBlock>) -> Result<(), ReadError> {
        let entry = *block.entry();
        match entry.block_kind() {
            Some(BlockKind::Hot) => {
                let view = block.hot()?;
                if view.kind().id() != entry.subtype || view.len() != entry.count as usize {
                    return layout("CONSIST-01", "the directory entry disagrees with the hot block");
                }
                self.entities += view.len() as u64;
                self.ids.extend_from_slice(view.entity_ids());
            }
            Some(BlockKind::Records) | Some(BlockKind::Arena) => {
                if entry.block_kind() == Some(BlockKind::Records) {
                    self.record_count = entry.count as usize;
                }
                self.advanced.push(block);
            }
            Some(BlockKind::Components) => {
                let reader = Reader { version: Version::new(1, 0, 0), implemented: Vec::new() };
                let opened =
                    read_streams(block.as_bytes(), &Registry::v1(), &reader).map_err(ReadError::Stream)?;
                self.streams = opened.skipped.iter().map(|s| s.capability.get()).collect();
                self.streams.sort_unstable();
            }
            None => {
                return layout(
                    "NEGOTIATE-01",
                    format!("cannot skip an optional block of kind {}", entry.kind),
                );
            }
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<Validated, ReadError> {
        if !self.advanced.is_empty() {
            let ids = self.core.advanced_ids()?.ok_or_else(internal)?;
            let mut six: Vec<Arc<DecodedBlock>> = Vec::with_capacity(6);
            for id in ids {
                let b = self.advanced.iter().find(|b| b.id() == id).ok_or_else(internal)?;
                six.push(Arc::clone(b));
            }
            let slices = BlockSlices {
                records: six[0].as_bytes(),
                knots: six[1].as_bytes(),
                weights: six[2].as_bytes(),
                control_points: six[3].as_bytes(),
                segments: six[4].as_bytes(),
                trims: six[5].as_bytes(),
            };
            let view = AdvancedView::parse(slices).map_err(ReadError::Arena)?;
            check_nesting(&view, self.limits.max_nesting_depth as usize)?;
            if view.len() != self.record_count {
                return layout("CONSIST-02", "record count differs from the directory");
            }
            for i in 0..view.len() {
                let rec = view.record(i).ok_or_else(|| {
                    ReadError::Arena(ArenaError::EntryOutOfRange {
                        index: i as u32,
                        entries: view.len() as u32,
                    })
                })?;
                checked_curve(&view, &self.limits, i)?;
                self.ids.push(rec.id);
            }
            self.entities += view.len() as u64;
        }
        if self.entities != self.expected_entities {
            return layout("CONSIST-02", "the entities found differ from the directory's counts");
        }
        let mut ids = core::mem::take(&mut self.ids);
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return layout("CONSIST-03", format!("entity {} is in two blocks", w[0]));
        }
        Ok(Validated {
            blocks: self.core.entries.len(),
            entities: self.entities,
            stream_capabilities: self.streams,
            skipped: Vec::new(),
        })
    }
}

fn push_gap(needs: &mut Vec<Need>, from: u64, to: u64) {
    let mut at = from;
    while at < to {
        let len = (to - at).min(GAP_CHUNK);
        needs.push(Need::Gap { offset: at, len: len as u32 });
        at += len;
    }
}

/// The refusals that are the reader's own and not a parser's, for the document.
pub(crate) const READER_RULES: &[(&str, &str)] = &[
    ("LAYOUT-01", "the manifest does not start straight after the header"),
    ("LAYOUT-02", "the capabilities are not strictly ascending by id"),
    (
        "LAYOUT-03",
        "the manifest does not hold exactly one block directory pointer, or the directory does not end the file (bytes after it are refused)",
    ),
    ("LAYOUT-04", "the blocks are not in strictly ascending (kind, cell, geometry type) order"),
    ("LAYOUT-06", "the directory has no blocks"),
    (
        "LAYOUT-07",
        "a block overlaps what comes before it, or the blocks run into the directory (the parsers refuse every file that could reach this; it is kept so that removing their checks does not remove it)",
    ),
    ("LAYOUT-08", "a byte between the manifest, the blocks and the directory is not zero"),
    (
        "CONSIST-01",
        "a directory entry disagrees with the hot block it describes (geometry type or entity count)",
    ),
    (
        "CONSIST-02",
        "the record count of the directory differs from the record block, or the entities found differ from the directory's counts",
    ),
    ("CONSIST-03", "an entity id is in two blocks"),
    ("CONSIST-04", "some of the six advanced blocks are present and some are not"),
    ("NEGOTIATE-01", "an optional block of a kind or a codec this reader cannot read (skipping it is P4.7)"),
    ("SOURCE", "the source failed, or answered with another number of bytes than were asked for"),
    ("BLOCK-ID", "a block id that is not in the directory"),
    ("LIMITS", "the configured limits ask for more than the format allows"),
];

/// `docs/READER.md`.
pub(crate) fn render_reader_doc() -> String {
    let mut s = String::new();
    s.push_str(
        "# The reader\n\n\
This file is generated from `crates/pvec-container/src/reader.rs` and `engine.rs`. I do not edit it by hand: a test \
regenerates it and fails if the committed file differs by a single byte. To change it I change the \
constants and run `PVEC_BLESS_READER=1 cargo test -p pvec-container --test reader_doc`.\n\n\
A `.pvec` file arrives from object storage over HTTP ranges and from a browser cache, and it may be hostile. \
One reader serves a local file, memory a caller has mapped, a positional-read interface and async HTTP ranges, and \
it never panics on what it is given: a refusal is a `ReadError` that names its rule. The limits it enforces are in \
[SECURITY_LIMITS.md](SECURITY_LIMITS.md); the layout it reads is [CONTAINER.md](CONTAINER.md).\n\n\
## The checks exist once\n\n\
`engine.rs` holds every check and does no IO. Each piece says which range it needs next and is fed the bytes that \
came back, so `ContainerReader` (blocking) and `AsyncContainerReader` (async) are the same loop around the same \
code. The tests drive both through one shared body and compare what they return, request by request.\n\n\
## Opening a file\n\n\
Three ranges at most are needed to read one block: the first 4 KiB, which holds the header and, in every file the \
writer produces, the manifest; the block directory, which ends the file; and the block. A manifest that does not fit \
the first 4 KiB is one more request. Each range is checked before the next is asked for: the header against the \
artifact length, the manifest against the header, the directory pointer against the manifest, the limits before the \
directory is fetched, and every raw directory entry (extent, expansion ratio, block size, entity count) before the \
table is parsed or any block is fetched.\n\n\
A source that answers with another number of bytes than were asked for is an error and is never decoded, whatever \
its own contract says. A `206` response is the only HTTP answer accepted, and its body must be the bytes asked for.\n\n\
## The read plan\n\n\
`plan(&[BlockId])` returns the requests that fetch the blocks, in file order, each with the blocks it covers. Two \
blocks whose gap is at most `gap_threshold` (64 KiB by default) are one request, the gap read and thrown away, so \
two blocks 512 bytes apart are fetched together and two blocks 8 MiB apart are not. A request never spans more than \
`max_request_bytes` (16 MiB by default) unless one block is longer than that, which is then a request of its own. \
`read_blocks` fetches through the plan and returns the blocks in the order asked for.\n\n\
## The cache\n\n\
Decoded blocks are cached by `BlockId` under a byte budget the caller supplies (`cache_bytes`, 64 MiB by default; \
zero turns it off). The policy is least recently used first out, each entry costing its decoded length, and a block \
larger than the whole budget is not kept. Dropping an entry drops the cache's reference only: the bytes go back to \
the allocation accountant when the last holder of the block lets go. When a new block would pass the allocation \
budget the cache gives up its least recently used blocks that nobody else holds, one at a time, until the block \
fits, and the read is refused when there are none left to give: evicting a block a caller still holds frees nothing.\n\n\
## Memory\n\n\
Every buffer is reserved with the accountant before it is made, the stored bytes of a block while it is fetched \
and its decoded bytes for as long as it lives, so the allocation budget is the sum of what is live. The prefix, \
the directory and the padding a validation reads are charged too, and so is the work: every byte fetched or decoded \
counts against `max_decode_work`. The size a file *declares* is never what is requested: a block of a declared \
4 GiB is refused from its directory entry, before a byte of it is fetched.\n\n\
## Validation\n\n\
`validate()` reads everything and holds the file to the canonical layout the writer produces: the bytes between \
blocks are zero, every block parses, the directory and the blocks agree, identities are unique, and composites \
are acyclic and shallow enough. It streams: blocks are decoded one at a time, padding is read a megabyte at a time, \
and everything it keeps (one id per entity to compare identities, the list of what to fetch, the advanced blocks until \
the curves have been checked) is reserved with the accountant like the blocks are.\n\n\
## Rules\n\n\
A refusal carries the rule of the parser that made it (`DIR-nn`, `HOT-nn`, `ARN-nn`, `STREAM-nn`, or the name of the \
`ContainerError` variant), the rule of a limit (`LIMIT-nn`, see SECURITY_LIMITS.md), or one of these:\n\n\
| rule | meaning |\n|---|---|\n",
    );
    for (rule, meaning) in READER_RULES {
        s.push_str(&format!("| {rule} | {meaning} |\n"));
    }
    s.push_str(
        "\nThe reference reader in `pvec-testkit` also has `LAYOUT-05`, a range that lies outside the file. This reader \
never reaches it, because every extent is held to the artifact length first (`LIMIT-12`).\n\n## Not here\n\n\
There is no memory-mapped source: a file cannot be mapped from safe Rust, and the only exception the workspace allows is the \
confined casts in `blocks/aligned.rs`. A caller that maps a file passes the slice to `MemorySource` and gets the same \
checks. The file sources are in the `pvec-sources` crate, because `pvec-container` is an API crate and opens no file. \
Compression and checksums are P4.6, skipping blocks the reader does not know is P4.7.\n",
    );
    s
}
