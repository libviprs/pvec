//! The named seed corpus.

use crate::layout::Layout;
use pvec_container::compress::{Codec, compress};
use pvec_container::directory::BlockKind;
use pvec_corpus::Bundle;
use pvec_testkit::generate::generate_all;
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
    let bundle = Bundle::vendored().expect("the vendored bundle opens");
    let mut out = Vec::new();
    for o in generate_all(&bundle).expect("the bundle registers") {
        if let Ok(a) = o.result {
            out.push(Seed {
                name: format!("known-truth/{}", a.fixture_id),
                source: Source::KnownTruth,
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
    let mut out: Vec<Vec<u8>> = Vec::new();
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
