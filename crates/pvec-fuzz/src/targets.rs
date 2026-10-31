//! The fuzz targets.
//!
//! Every target does three things with its input: it never panics, it never allocates in proportion to what
//! the input claims (the reader accounts for that, and the smoke driver and libFuzzer's RSS limit hold it to
//! it), and when the input parses it checks what the parser promised, so a parser that accepts something it
//! should have refused is a crash too.

use crate::layout::Layout;
use pvec_capabilities::{Reader, Registry, Version};
use pvec_container::blocks::AlignedBytes;
use pvec_container::blocks::advanced::{AdvancedView, BlockSlices, CurveView};
use pvec_container::blocks::hot::HotBlockView;
use pvec_container::blocks::raster::RasterIndex;
use pvec_container::compress::Codec;
use pvec_container::directory::BlockKind;
use pvec_container::integrity::{Integrity, artifact_digest, block_checksum};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::spec::m;
use pvec_container::writer::{ContainerWriter, EntityMeta, RawBlock, WriterConfig, assemble};
use pvec_container::{Bounds, Header, LengthUnit, Limits, Manifest, Metadata};
use pvec_entity::EntityId;
use std::borrow::Cow;
use std::sync::OnceLock;

/// A target: bytes in, the word for what it reached out.
pub type Target = fn(&[u8]) -> String;

/// The targets by name.
pub const TARGETS: [(&str, Target); 7] = [
    ("fuzz_header", fuzz_header),
    ("fuzz_directory_traversal", fuzz_directory_traversal),
    ("fuzz_immediate_blocks", fuzz_immediate_blocks),
    ("fuzz_advanced_blocks", fuzz_advanced_blocks),
    ("fuzz_decompress_boundary", fuzz_decompress_boundary),
    ("fuzz_component_streams", fuzz_component_streams),
    ("fuzz_raster_index", fuzz_raster_index),
];

/// The input as the target sees it: with the `no-checksum-gate` feature, resealed when it still lays out as
/// a container.
fn input(data: &[u8]) -> Cow<'_, [u8]> {
    if cfg!(feature = "no-checksum-gate") {
        reseal(data).map_or(Cow::Borrowed(data), Cow::Owned)
    } else {
        Cow::Borrowed(data)
    }
}

/// The input with its block checksums and artifact digest recomputed over the layout it already has, when
/// that layout parses. Nothing else is touched, so a hostile layout stays hostile; only the checksum no
/// longer gives it away.
pub fn reseal(data: &[u8]) -> Option<Vec<u8>> {
    let layout = Layout::of(data)?;
    let mut out = data.to_vec();
    if let Some(table) = layout.manifest.block_checksums {
        let at = usize::try_from(table.offset).ok()?;
        let end = at.checked_add(usize::try_from(table.length).ok()?)?;
        if end <= out.len() && table.length == 8 * layout.entries.len() as u64 {
            for (i, _) in layout.entries.iter().enumerate() {
                let sum = block_checksum(layout.stored(data, i)?);
                out[at + 8 * i..at + 8 * i + 8].copy_from_slice(&sum.to_le_bytes());
            }
        }
    }
    if layout.manifest.artifact_digest.is_some() {
        let at = layout.header.manifest_offset as usize + m::ARTIFACT_DIGEST;
        if at + 32 <= out.len() {
            let digest = artifact_digest(&out, at);
            out[at..at + 32].copy_from_slice(&digest);
        }
    }
    Some(out)
}

/// The header and the manifest.
pub fn fuzz_header(data: &[u8]) -> String {
    let data = input(data);
    let len = data.len() as u64;
    let header = match Header::parse(&data[..data.len().min(4096)], len) {
        Ok(h) => h,
        Err(e) => return e.rule().to_string(),
    };
    let (start, length) = (header.manifest_offset, header.manifest_length);
    assert!(
        start.checked_add(length).is_some_and(|end| end <= len),
        "a header that points outside the file was accepted"
    );
    let bytes = &data[start as usize..(start + length) as usize];
    match Manifest::parse(bytes, &header, len) {
        Ok(_) => "ok".to_string(),
        Err(e) => e.rule().to_string(),
    }
}

/// A whole artifact: open, plan, read every block, validate.
pub fn fuzz_directory_traversal(data: &[u8]) -> String {
    let data = input(data);
    let len = data.len() as u64;
    let reader = match ContainerReader::open(MemorySource::new(data.to_vec()), ReaderConfig::default()) {
        Ok(r) => r,
        Err(e) => return e.rule(),
    };
    for e in reader.entries() {
        assert!(
            e.offset.checked_add(u64::from(e.stored_length)).is_some_and(|end| end <= len),
            "a block outside the file was accepted"
        );
    }
    let skipped: Vec<BlockId> =
        reader.partial_open().map(|p| p.blocks.iter().map(|b| b.block).collect()).unwrap_or_default();
    let ids: Vec<BlockId> =
        (0..reader.entries().len() as u32).map(BlockId).filter(|b| !skipped.contains(b)).collect();
    match reader.plan(&ids) {
        Ok(plan) => {
            for r in &plan {
                assert!(
                    r.offset.checked_add(u64::from(r.len)).is_some_and(|end| end <= len),
                    "a planned request outside the file"
                );
            }
        }
        Err(e) => return e.rule(),
    }
    match reader.read_blocks(&ids) {
        Ok(blocks) => {
            for (id, b) in ids.iter().zip(&blocks) {
                assert_eq!(b.as_bytes().len(), reader.entries()[id.0 as usize].uncompressed_length as usize);
            }
        }
        Err(e) => return e.rule(),
    }
    if let Err(e) = reader.validate() {
        return e.rule();
    }
    match reader.verify_artifact_digest() {
        Ok(_) => "ok".to_string(),
        Err(e) => e.rule(),
    }
}

/// One hot (immediate geometry) block.
pub fn fuzz_immediate_blocks(data: &[u8]) -> String {
    let aligned = AlignedBytes::from_bytes(data);
    let view = match HotBlockView::parse(aligned.as_bytes()) {
        Ok(v) => v,
        Err(e) => return e.rule().to_string(),
    };
    let ids = view.entity_ids();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "entity ids that do not ascend were accepted");
    assert_eq!(ids.len(), view.len());
    assert_eq!(view.styles().len(), view.len());
    for name in view.kind().f64_columns() {
        let column = view.column(name).expect("a column of the kind");
        assert!(column.iter().all(|x| x.is_finite()), "a non-finite coordinate was accepted in {name}");
    }
    for i in 0..view.len() {
        if let Some((x, y)) = view.polyline(i) {
            assert_eq!(x.len(), y.len());
        }
    }
    "ok".to_string()
}

/// Splits `data` into the six advanced blocks: six little-endian `u32` lengths, then the blocks, each length
/// clamped to what is left. Fewer than 24 bytes is no frame.
fn six_blocks(data: &[u8]) -> Option<[&[u8]; 6]> {
    let (lens, mut rest) = data.split_at_checked(24)?;
    let mut out: [&[u8]; 6] = [&[]; 6];
    for (slot, l) in out.iter_mut().zip(lens.chunks_exact(4)) {
        let n = (u32::from_le_bytes([l[0], l[1], l[2], l[3]]) as usize).min(rest.len());
        let (block, tail) = rest.split_at(n);
        *slot = block;
        rest = tail;
    }
    Some(out)
}

/// The six advanced blocks (records, knots, weights, control points, segments, trims), framed as six `u32`
/// lengths and then the blocks.
pub fn fuzz_advanced_blocks(data: &[u8]) -> String {
    let Some(blocks) = six_blocks(data) else { return "short".to_string() };
    let aligned: Vec<AlignedBytes> = blocks.iter().map(|b| AlignedBytes::from_bytes(b)).collect();
    let view = match AdvancedView::parse(BlockSlices {
        records: aligned[0].as_bytes(),
        knots: aligned[1].as_bytes(),
        weights: aligned[2].as_bytes(),
        control_points: aligned[3].as_bytes(),
        segments: aligned[4].as_bytes(),
        trims: aligned[5].as_bytes(),
    }) {
        Ok(v) => v,
        Err(e) => return e.rule().to_string(),
    };
    if let Err(e) = view.check_nesting_within(Limits::DEFAULT.max_nesting_depth as usize) {
        return e.rule().to_string();
    }
    for i in 0..view.len() {
        let curve = match view.curve(i) {
            Ok(c) => c,
            Err(e) => return e.rule().to_string(),
        };
        match curve {
            CurveView::Spline(s) => {
                assert_eq!(s.x.len(), s.y.len());
                assert_eq!(
                    s.knots.len(),
                    s.x.len() + s.degree + 1,
                    "a knot vector of the wrong length was accepted"
                );
                assert!(s.knots.windows(2).all(|w| w[0] <= w[1]), "knots that decrease were accepted");
                assert!(
                    s.knots.iter().chain(s.x).chain(s.y).all(|v| v.is_finite()),
                    "a non-finite number was accepted"
                );
                if let Some(w) = s.weights {
                    assert_eq!(w.len(), s.x.len());
                    assert!(
                        w.iter().all(|w| *w > 0.0 && w.is_finite()),
                        "a weight that is not positive was accepted"
                    );
                }
            }
            CurveView::Composite(c) => {
                assert!(
                    c.segments.iter().all(|s| (*s as usize) < view.len()),
                    "a segment outside the records was accepted"
                );
            }
        }
        if let Err(e) = view.declared_counts(i) {
            return e.rule().to_string();
        }
    }
    "ok".to_string()
}

/// What the reader did with a block whose codec, declared length and stored bytes the input chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Boundary {
    /// The input is shorter than its frame.
    NotFramed,
    /// The reader refused, naming the rule.
    Refused(String),
    /// The block decoded to this many bytes.
    Decoded(usize),
}

fn base_manifest() -> &'static Manifest {
    static BASE: OnceLock<Manifest> = OnceLock::new();
    BASE.get_or_init(|| {
        let mut w = ContainerWriter::new(WriterConfig {
            unit: LengthUnit::Metre,
            metadata: Metadata {
                producer_id: "pvec-fuzz".into(),
                producer_version: "0".into(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: Vec::new(),
        });
        let bounds = Bounds { min: [0.0, 0.0], max: [1.0, 1.0] };
        let id = EntityId::new(0).expect("id 0");
        w.push_line(EntityMeta { id, style: 0, cell: 0, bounds }, [0.0, 0.0], [1.0, 1.0]).expect("a line");
        let bytes = w.finish().expect("a file");
        Layout::of(&bytes).expect("a layout").manifest
    })
}

/// Frames the input as `[codec: u8][declared length: u32 le][stored bytes]`, lays it out as a one-block
/// artifact and reads the block back through the reader. A declared length beyond the expansion ratio of the
/// stored length is a refusal and never a decode.
pub fn decompress_boundary(data: &[u8]) -> Boundary {
    let Some((head, stored)) = data.split_at_checked(5) else { return Boundary::NotFramed };
    let declared = u32::from_le_bytes([head[1], head[2], head[3], head[4]]);
    let block = RawBlock {
        kind: BlockKind::Hot.id(),
        subtype: 1,
        cell: 0,
        count: 1,
        bounds: None,
        flags: 0,
        codec: head[0],
        capability: 0,
        uncompressed_length: declared,
        stored: stored.to_vec(),
    };
    let file = match assemble(base_manifest().clone(), &[block], Integrity::None) {
        Ok(f) => f,
        Err(e) => return Boundary::Refused(e.rule().to_string()),
    };
    let reader = match ContainerReader::open(MemorySource::new(file), ReaderConfig::default()) {
        Ok(r) => r,
        Err(e) => return Boundary::Refused(e.rule()),
    };
    match reader.read_block(BlockId(0)) {
        Ok(b) => Boundary::Decoded(b.as_bytes().len()),
        Err(e) => Boundary::Refused(e.rule()),
    }
}

/// What a decompress boundary does with the stream: see [`decompress_boundary`]. It asserts the expansion
/// ratio, so a bomb that is decoded is a crash and not a slow input.
pub fn fuzz_decompress_boundary(data: &[u8]) -> String {
    match decompress_boundary(data) {
        Boundary::NotFramed => "short".to_string(),
        Boundary::Refused(rule) => rule,
        Boundary::Decoded(n) => {
            let stored = data.len() - 5;
            assert!(
                n as u64 <= stored as u64 * u64::from(Limits::DEFAULT.max_expansion_ratio),
                "{n} bytes decoded from {stored} stored, over the expansion ratio"
            );
            assert_eq!(n as u32, u32::from_le_bytes([data[1], data[2], data[3], data[4]]));
            let _ = Codec::from_id(data[0]).expect("a known codec decoded");
            "ok".to_string()
        }
    }
}

/// A component stream set.
pub fn fuzz_component_streams(data: &[u8]) -> String {
    let registry = Registry::v1();
    let reader = Reader {
        version: Version::new(1, 1, 0),
        implemented: registry
            .entries()
            .iter()
            .filter(|c| matches!(c.class, pvec_capabilities::Class::Component | pvec_capabilities::Class::Raster))
            .map(|c| c.id)
            .collect(),
    };
    match pvec_components::read_streams(data, &registry, &reader) {
        Ok(opened) => {
            assert!(
                opened.streams.iter().all(|s| registry.get(s.capability).is_some()),
                "a stream of an unknown capability was read"
            );
            // the raster placements (issue 217): what decodes is valid, and writes back to something that decodes
            // to the same placements
            if let pvec_components::RasterRead::Present(placements) = pvec_components::read_rasters(&opened) {
                let again = pvec_components::encode_rasters(&placements)
                    .and_then(|bytes| pvec_components::decode_rasters(&bytes));
                assert_eq!(again.as_ref(), Ok(&placements), "a decoded raster stream did not survive a round trip");
            }
            "ok".to_string()
        }
        Err(e) => e.rule().to_string(),
    }
}

/// A raster index block (kind 9, `docs/RASTER.md`): the second parser over hostile bytes that ADR 0018 adds.
/// A block that parses is checked against everything the layout promises, and must encode to the bytes it came
/// from, so a parser that accepts a block the writer would not write is a crash.
pub fn fuzz_raster_index(data: &[u8]) -> String {
    // LIMIT-14, the default ceiling on the entries of the tile tables of an artifact
    let index = match RasterIndex::parse(data, 1 << 20) {
        Ok(i) => i,
        Err(e) => return e.rule().to_string(),
    };
    assert_eq!(index.encode(), data, "a block that parses is not the block that encodes");
    let (mut ordinal, mut stored) = (0u64, 0u32);
    for (l, level) in index.levels().iter().enumerate() {
        assert_eq!(u64::from(level.first), ordinal, "a level that does not start where the last ended");
        let tiles = index.level_tiles(l as u32).expect("the tiles of a level of the table");
        assert_eq!(tiles.len() as u64, u64::from(level.cols) * u64::from(level.rows));
        let mut chunks = std::collections::BTreeSet::new();
        for t in tiles {
            assert!(t.col < level.cols && t.row < level.rows, "a tile outside its level");
            assert_eq!(index.tile(l as u32, t.col, t.row), Some(t), "a tile the lookup does not find");
            if t.is_stored() {
                stored += 1;
                if t.is_inline() {
                    let bytes = index.inline_bytes(t).expect("an inline tile has bytes");
                    assert_eq!(bytes.len(), t.length as usize);
                } else {
                    assert!(t.chunk < level.chunks, "a tile in a chunk the level does not have");
                    chunks.insert(t.chunk);
                }
            } else {
                assert_eq!((t.xxh3, t.offset, t.chunk), (0, 0, 0), "an unstored tile that carries something");
            }
        }
        assert_eq!(chunks.len() as u32, level.chunks, "a chunk with no tile");
        assert!(
            index.tile(l as u32, level.cols, 0).is_none() && index.tile(l as u32, 0, level.rows).is_none()
        );
        ordinal += u64::from(level.cols) * u64::from(level.rows);
    }
    assert_eq!(ordinal, index.tiles().len() as u64);
    assert_eq!(stored, index.stored());
    // the chunk blocks the tiles describe are exactly as long as their tiles
    let chunk_end = |l: u32, c: u32| {
        index
            .level_tiles(l)?
            .iter()
            .filter(|t| t.is_stored() && !t.is_inline() && t.chunk == c)
            .map(|t| t.offset + t.length)
            .max()
    };
    index.check_chunks(chunk_end).expect("the chunks the tiles describe");
    // the size a placement would have to give the last level for this index to be its pyramid, and some that it
    // would not: a check against a placement is an answer and never a crash
    let last = index.levels().last().expect("a level");
    for (w, h, edge) in [
        (last.cols.saturating_mul(256), last.rows.saturating_mul(256), 256),
        (last.cols.saturating_mul(512), last.rows.saturating_mul(512), 512),
        (u32::MAX, u32::MAX, 256),
        (1, 1, 0),
    ] {
        let _ = index.check_grid(w, h, edge);
    }
    "ok".to_string()
}
