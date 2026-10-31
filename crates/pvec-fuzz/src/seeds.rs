//! The named seed corpus.

use crate::layout::Layout;
use pvec_container::compress::{Codec, CodecPolicy, compress};
use pvec_container::directory::BlockKind;
use pvec_container::writer::WriterOptions;
use pvec_corpus::Bundle;
use pvec_testkit::generate::{generate_all, generate_all_with};
use pvec_testkit::hostile::{bases, catalog};
use std::collections::BTreeSet;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Where a seed came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// A case of the hostile corpus (P2.7).
    Hostile,
    /// A `KNOWN_TRUTH_EXACT` fixture of the vendored bundle, written by the container writer.
    KnownTruth,
    /// Container writer output of the synthetic shapes, and mutated manifests of it.
    Writer,
}

impl Source {
    /// The word the hardening report uses.
    pub fn name(self) -> &'static str {
        match self {
            Source::Hostile => "hostile (P2.7)",
            Source::KnownTruth => "known-truth (vendored bundle)",
            Source::Writer => "writer output and mutated manifests",
        }
    }
}

/// One seed.
#[derive(Clone, Debug)]
pub struct Seed {
    /// A unique name.
    pub name: String,
    /// Where it came from.
    pub source: Source,
    /// The bytes of an artifact.
    pub bytes: Vec<u8>,
}

/// What is not in the corpus yet, and why: the sources the issue names that are not on main.
pub const DEFERRED: &[(&str, &str)] = &[
    (
        "the P8.1 benchmark corpus after minimisation",
        "P8.1 is not on main, so there is no benchmark corpus to minimise",
    ),
    (
        "KNOWN_TRUTH_EXACT fixtures from libviprs/pvec-gold-fixtures#1",
        "EPIC O has produced no fixtures yet; the KNOWN_TRUTH_EXACT records of the vendored bundle stand in for them",
    ),
];

fn is_sweep(name: &str) -> bool {
    name.starts_with("truncation/prefix/") || name.starts_with("truncation/remove_byte/")
}

/// Every seed, deterministic.
pub fn corpus() -> Vec<Seed> {
    /// The base with a kind 9 and a kind 10 entry added (ADR 0018), once naming capability 21, once none and once
    /// an unregistered one, so the traversal meets the capability-owned kinds in every state the rules decide.
    fn capability_owned(base: &bases::Base) -> Vec<Seed> {
        use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
        use pvec_container::integrity::Integrity;
        use pvec_container::writer::{RawBlock, assemble};
        let Some(layout) = Layout::of(&base.bytes) else { return Vec::new() };
        let stored: Vec<RawBlock> = layout
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                Some(RawBlock {
                    kind: e.kind,
                    subtype: e.subtype,
                    cell: e.cell,
                    count: e.count,
                    bounds: e.bounds,
                    flags: e.flags,
                    codec: e.codec,
                    capability: e.capability,
                    uncompressed_length: e.uncompressed_length,
                    stored: layout.stored(&base.bytes, i)?.to_vec(),
                })
            })
            .collect();
        let mut out = Vec::new();
        for (word, capability) in [("21", 21u16), ("0", 0), ("77", 77)] {
            let mut manifest = layout.manifest.clone();
            for id in [21u32, 77] {
                if !manifest.capabilities.iter().any(|d| d.id.get() == id) {
                    manifest.capabilities.push(Declared {
                        id: CapabilityId::new(id),
                        requirement: Requirement::Optional,
                        version: Version::new(1, 0, 0),
                    });
                }
            }
            manifest.capabilities.sort_by_key(|d| d.id);
            let mut blocks = stored.clone();
            for kind in [9u16, 10] {
                blocks.push(RawBlock {
                    kind,
                    subtype: 0,
                    cell: 0,
                    count: 1,
                    bounds: None,
                    flags: 0,
                    codec: 0,
                    capability,
                    uncompressed_length: 32,
                    stored: (0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(kind as u8)).collect(),
                });
            }
            if stored.len() == layout.entries.len() {
                if let Ok(bytes) = assemble(manifest, &blocks, Integrity::Full) {
                    out.push(Seed {
                        name: format!("writer/{}/capability-owned-{word}", base.name),
                        source: Source::Writer,
                        bytes,
                    });
                }
            }
        }
        out
    }

    let bundle = Bundle::vendored().expect("the vendored bundle opens");
    let mut out = Vec::new();
    // The known-truth seeds are written without compression, as the corpus always was: the mutator and the
    // checksum-gate statistics are about files whose blocks are bytes a flip can land in. The same fixtures
    // as the writer's defaults make them (compressed since P8.6) are seeds too, so the fuzzer starts from the
    // shipped configuration as well.
    let plain = WriterOptions { codecs: CodecPolicy::NONE, ..WriterOptions::default() };
    for o in generate_all_with(&bundle, plain).expect("the bundle registers") {
        if let Ok(a) = o.result {
            out.push(Seed {
                name: format!("known-truth/{}", a.fixture_id),
                source: Source::KnownTruth,
                bytes: a.bytes,
            });
        }
    }
    for o in generate_all(&bundle).expect("the bundle registers") {
        if let Ok(a) = o.result {
            out.push(Seed {
                name: format!("writer/known-truth-default/{}", a.fixture_id),
                source: Source::Writer,
                bytes: a.bytes,
            });
        }
    }
    let bases = bases(&bundle);
    let cases = catalog(&bases);
    for c in cases.iter().filter(|c| !is_sweep(&c.name)) {
        out.push(Seed {
            name: format!("hostile/{}", c.name),
            source: Source::Hostile,
            bytes: c.bytes(&bases[c.base]),
        });
    }
    for b in bases.iter().filter(|b| {
        matches!(
            b.name.as_str(),
            "drawing" | "splines" | "composites" | "streams" | "gapped" | "drawing+lz4" | "drawing+deflate"
        )
    }) {
        out.push(Seed { name: format!("writer/{}", b.name), source: Source::Writer, bytes: b.bytes.clone() });
        // the manifest with one field changed, sealed again so the file still verifies
        for k in 0..4u64 {
            let mut m = b.bytes.clone();
            crate::mutate::mutate_manifest(&mut m, k);
            out.push(Seed {
                name: format!("writer/{}/manifest-{k}", b.name),
                source: Source::Writer,
                bytes: m,
            });
        }
        out.extend(capability_owned(b));
    }
    out
}

fn one_of<T>(items: Vec<T>, key: impl Fn(&T) -> &[u8]) -> Vec<T> {
    let mut seen = BTreeSet::new();
    items
        .into_iter()
        .filter(|i| {
            let mut h = DefaultHasher::new();
            key(i).hash(&mut h);
            seen.insert(h.finish())
        })
        .collect()
}

fn decoded(layout: &Layout, bytes: &[u8], i: usize) -> Option<Vec<u8>> {
    let e = &layout.entries[i];
    let stored = layout.stored(bytes, i)?;
    match Codec::from_id(e.codec)? {
        Codec::None => Some(stored.to_vec()),
        codec => pvec_container::compress::decompress(codec, stored, e.uncompressed_length as usize).ok(),
    }
}

/// The inputs for one target, taken from the seeds: the whole artifact, its header, one hot block, the framed
/// advanced blocks, a framed compressed block or the component stream set.
pub fn for_target(target: &str, seeds: &[Seed]) -> Vec<Vec<u8>> {
    if target == "fuzz_raster_index" {
        // not made of artifacts: the index blocks of the tiler's pyramids, and mutants of them
        return raster_index_inputs();
    }
    /// A stream set with one valid raster placement in it (issue 217): the test vector of `docs/RASTER.md`.
    fn raster_seed() -> Option<Vec<u8>> {
        use pvec_components::{
            Band, PixelFormat, Placement, RASTER_CAPABILITY, Stream, TileCodec, encode_rasters,
            encode_streams,
        };
        let payload = encode_rasters(&[Placement {
            pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
            width: 1024,
            height: 512,
            visible_box: [0, 0, 1024, 512],
            tile_edge: 512,
            levels: 2,
            tile_codec: TileCodec::Png,
            pixel_format: PixelFormat::Rgba,
            bit_depth: 8,
            opacity: 255,
            band: Band::Underlay,
            layer_index: Some(0),
            source_digest: Some([0x5A; 32]),
            attribution: "survey".into(),
        }])
        .ok()?;
        let version = pvec_capabilities::Registry::v1().get(RASTER_CAPABILITY)?.version;
        encode_streams(&[Stream { capability: RASTER_CAPABILITY, version, payload }]).ok()
    }

    let mut out: Vec<Vec<u8>> = Vec::new();
    if target == "fuzz_component_streams" {
        out.extend(raster_seed());
    }
    for s in seeds {
        let bytes = &s.bytes;
        match target {
            "fuzz_header" => out.push(bytes[..bytes.len().min(4096)].to_vec()),
            "fuzz_directory_traversal" => out.push(bytes.clone()),
            _ => {
                let Some(layout) = Layout::of(bytes) else { continue };
                match target {
                    "fuzz_immediate_blocks" => {
                        for (i, e) in layout.entries.iter().enumerate() {
                            if e.block_kind() == Some(BlockKind::Hot) {
                                out.extend(decoded(&layout, bytes, i));
                            }
                        }
                    }
                    "fuzz_component_streams" => {
                        for (i, e) in layout.entries.iter().enumerate() {
                            if e.block_kind() == Some(BlockKind::Components) {
                                out.extend(decoded(&layout, bytes, i));
                            }
                        }
                    }
                    "fuzz_advanced_blocks" => {
                        let cells: BTreeSet<u32> = layout
                            .entries
                            .iter()
                            .filter(|e| e.block_kind() == Some(BlockKind::Records))
                            .map(|e| e.cell)
                            .collect();
                        for cell in cells {
                            // records first, then the arenas by payload kind: knots, weights, points, segments, trims
                            let pick = |kind: BlockKind, subtype: u16| {
                                layout
                                    .entries
                                    .iter()
                                    .position(|e| {
                                        e.block_kind() == Some(kind)
                                            && e.cell == cell
                                            && (kind == BlockKind::Records || e.subtype == subtype)
                                    })
                                    .and_then(|i| decoded(&layout, bytes, i))
                                    .unwrap_or_default()
                            };
                            let blocks = [
                                pick(BlockKind::Records, 0),
                                pick(BlockKind::Arena, 1),
                                pick(BlockKind::Arena, 2),
                                pick(BlockKind::Arena, 3),
                                pick(BlockKind::Arena, 4),
                                pick(BlockKind::Arena, 5),
                            ];
                            let mut framed: Vec<u8> =
                                blocks.iter().flat_map(|b| (b.len() as u32).to_le_bytes()).collect();
                            framed.extend(blocks.concat());
                            out.push(framed);
                        }
                    }
                    "fuzz_decompress_boundary" => {
                        for (i, e) in layout.entries.iter().enumerate() {
                            let Some(stored) = layout.stored(bytes, i) else { continue };
                            let mut framed = vec![e.codec];
                            framed.extend(e.uncompressed_length.to_le_bytes());
                            framed.extend(stored);
                            out.push(framed);
                            // and the same block packed by each codec, so every codec has a stream that decodes
                            if let Some(raw) = decoded(&layout, bytes, i) {
                                for codec in [Codec::Lz4, Codec::Deflate] {
                                    let mut packed = vec![codec.id()];
                                    packed.extend((raw.len() as u32).to_le_bytes());
                                    packed.extend(compress(codec, &raw));
                                    out.push(packed);
                                }
                            }
                        }
                    }
                    other => panic!("no target named {other}"),
                }
            }
        }
    }
    one_of(out, |v| v.as_slice())
}

/// The inputs of `fuzz_raster_index`: the index blocks of the test-only tiler's pyramids (a single tile, a
/// pyramid with inline levels, a sparse one, a split one, one with a tile table of 65,000
/// entries), each with a handful of mutants from the structure-aware mutator, and the header-only blocks that
/// claim more than the ceiling.
fn raster_index_inputs() -> Vec<Vec<u8>> {
    use pvec_container::blocks::raster::build;
    use pvec_testkit::tiler::{Shape, tiles};
    let shape = |width, height| Shape { width, height, tile_edge: 256, channels: 1, bit_depth: 1 };
    let mut out: Vec<Vec<u8>> = Vec::new();
    let mut valid: Vec<Vec<u8>> = Vec::new();
    // (shape, tiles kept, inline levels, bytes a chunk)
    type Keep = fn(u32, u32, u32) -> bool;
    let all: Keep = |_, _, _| true;
    let sparse: Keep = |l, c, r| (l + c + r) % 3 != 0;
    for (s, keep, inline, chunk) in [
        (shape(200, 200), all, 0, 1 << 20),
        (shape(700, 500), all, 0, 1 << 20),
        (shape(700, 500), all, 2, 1 << 20),
        (shape(700, 500), sparse, 1, 1 << 20),
        (shape(1500, 900), all, 0, 20_000),
        (shape(1500, 900), sparse, 3, 20_000),
        (shape(256, 4096), all, 0, 1 << 20),
    ] {
        let grid = s.grid();
        let built = build(&grid, tiles(s, keep), inline, chunk, 1 << 20).expect("the tiler's pyramid builds");
        valid.push(built.index.encode());
    }
    // a wide level that is all entries and no bytes
    let wide = build(&[(65_000, 1)], Vec::new(), 0, 1 << 20, 1 << 20).expect("a wide pyramid");
    valid.push(wide.index.encode());
    for (k, v) in valid.iter().enumerate() {
        out.push(v.clone());
        if v.len() <= 1 << 20 {
            for m in 0..8u64 {
                let mut mutant = v.clone();
                crate::mutate::mutate_index(&mut mutant, 1 + m + 16 * k as u64);
                out.push(mutant);
            }
        }
    }
    // a header that claims more than any block holds, and more than the ceiling
    for tiles in [1u32, 4096, (1 << 20) + 1, u32::MAX] {
        let mut h = vec![0u8; 32];
        h[..4].copy_from_slice(b"PVX1");
        h[4] = 1;
        h[6] = 1;
        h[8..12].copy_from_slice(&tiles.to_le_bytes());
        out.push(h);
    }
    out.push(Vec::new());
    one_of(out, |v| v.as_slice())
}
