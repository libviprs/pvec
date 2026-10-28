//! The benchmark corpus of phase 8 (issue P8.1): a fixed, named set of `.pvec` artifacts that every
//! later phase 8 benchmark measures on, with a lock file that pins each one.
//!
//! An entry is either one of the `r0-open` known-truth fixtures run through the P2.4 generator, or a
//! synthetic drawing from [`models`] and a seed. Nothing is stored: the vendored bundle and the seeds
//! are the corpus, and [`build`] rebuilds any entry from them with no converter, no network and no
//! process. `benches/corpus/CORPUS.md` says what each class is, and `benches/corpus/corpus.lock.json`
//! pins every entry by digest.

pub mod dump;
pub mod geometry_bench;
pub mod layout_bench;
pub mod lock;
pub mod models;
pub mod part21;
pub mod progressive_bench;
pub mod shape;
pub mod step_bench;

use models::Model;
use pvec_container::source::BlockSource;
use pvec_container::writer::WriterOptions;
use pvec_container::{Bounds, ContainerReader, LengthUnit, ReadError};
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{Bundle, CorpusError};
use pvec_testkit::generate::{generate_all, generate_all_with};
use pvec_testkit::reference::{ReadEntity, read};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// The generator's version. Change it when a model changes, so a stale lock cannot match.
pub const GENERATOR: &str = "pvec-bench-corpus/1";

/// The seven corpus classes, by the name the lock file and `CORPUS.md` use.
pub const CLASSES: [&str; 7] = [
    "synthetic-known-truth",
    "pathological",
    "architectural",
    "mep",
    "industrial",
    "huge-coordinate",
    "spline-heavy",
];

/// Viewports recorded per entry.
pub const VIEWPORTS: usize = 200;

/// Fetches the advanced `chunks` a viewport needs and walks their curves, returning how many it saw. The
/// blocks of up to 16 MiB of chunks go to one `read_blocks` call, so the planner coalesces neighbouring
/// chunks into the same ranges (the request cap is 16 MiB anyway), and each chunk is then opened from the cache.
pub fn read_advanced<S: BlockSource>(r: &ContainerReader<S>, chunks: &[usize]) -> Result<usize, ReadError> {
    const BATCH: u64 = 16 << 20;
    let (mut seen, mut batch, mut ids, mut stored) = (0, Vec::new(), Vec::new(), 0u64);
    for (i, &c) in chunks.iter().enumerate() {
        let six = r.advanced_chunk_blocks(&[c])?;
        stored += six.iter().map(|id| u64::from(r.entries()[id.0 as usize].stored_length)).sum::<u64>();
        ids.extend(six);
        batch.push(c);
        if stored >= BATCH || i + 1 == chunks.len() {
            r.read_blocks(&ids)?;
            for c in batch.drain(..) {
                if let Some(set) = r.advanced(c)? {
                    seen += set.view()?.len();
                }
            }
            ids.clear();
            stored = 0;
        }
    }
    Ok(seen)
}

/// One synthetic entry: what to draw, how many, from which seed.
#[derive(Clone, Copy, Debug)]
pub struct Spec {
    /// The entry name, which names its file `<name>.pvec`.
    pub name: &'static str,
    /// The class name, one of [`CLASSES`].
    pub class: &'static str,
    /// What the generator draws.
    pub model: Model,
    /// How many entities.
    pub entities: usize,
    /// The splitmix64 seed.
    pub seed: u64,
    /// Generated on demand and never stored or checked by the default tests (the 10^7 case).
    pub on_demand: bool,
}

/// The synthetic entries. The first column of the ladder is the architectural plan at 10^3, 10^5, 10^6
/// and 10^7 entities; the other classes have their own sizes.
pub const SPECS: [Spec; 10] = [
    Spec {
        name: "pathological-1e3",
        class: "pathological",
        model: Model::Pathological,
        entities: 1_000,
        seed: 0x0b01,
        on_demand: false,
    },
    Spec {
        name: "architectural-1e3",
        class: "architectural",
        model: Model::Architectural,
        entities: 1_000,
        seed: 0xa103,
        on_demand: false,
    },
    Spec {
        name: "architectural-1e5",
        class: "architectural",
        model: Model::Architectural,
        entities: 100_000,
        seed: 0xa105,
        on_demand: false,
    },
    Spec {
        name: "architectural-1e6",
        class: "architectural",
        model: Model::Architectural,
        entities: 1_000_000,
        seed: 0xa106,
        on_demand: false,
    },
    Spec {
        name: "architectural-1e7",
        class: "architectural",
        model: Model::Architectural,
        entities: 10_000_000,
        seed: 0xa107,
        on_demand: true,
    },
    Spec {
        name: "mep-1e5",
        class: "mep",
        model: Model::Mep,
        entities: 100_000,
        seed: 0x3e05,
        on_demand: false,
    },
    Spec {
        name: "industrial-1e5",
        class: "industrial",
        model: Model::Industrial,
        entities: 100_000,
        seed: 0x1d05,
        on_demand: false,
    },
    Spec {
        name: "huge-coordinate-1e5",
        class: "huge-coordinate",
        model: Model::HugeCoordinate,
        entities: 100_000,
        seed: 0x4c05,
        on_demand: false,
    },
    Spec {
        name: "spline-heavy-1e5",
        class: "spline-heavy",
        model: Model::SplineHeavy,
        entities: 100_000,
        seed: 0x5b05,
        on_demand: false,
    },
    Spec {
        name: "spline-heavy-1e6",
        class: "spline-heavy",
        model: Model::SplineHeavy,
        entities: 1_000_000,
        seed: 0x5b06,
        on_demand: false,
    },
];

impl Spec {
    /// The entry with this name.
    pub fn named(name: &str) -> Option<&'static Spec> {
        SPECS.iter().find(|s| s.name == name)
    }

    /// The unit the artifact declares: the huge-coordinate class is in metres (UTM-scale offsets with
    /// millimetre detail), everything else in millimetres.
    pub fn unit(&self) -> LengthUnit {
        if self.model == Model::HugeCoordinate { LengthUnit::Metre } else { LengthUnit::Millimetre }
    }

    /// The shapes this entry draws.
    pub fn shapes(&self) -> Vec<shape::Shape> {
        models::shapes(self.model, self.entities, self.seed)
    }

    /// The artifact bytes, with the writer's default options.
    pub fn artifact(&self) -> Result<Vec<u8>, String> {
        self.artifact_with(WriterOptions::default())
    }

    /// The artifact bytes with the writer set to compress and seal as `options` say.
    pub fn artifact_with(&self, options: WriterOptions) -> Result<Vec<u8>, String> {
        let producer = format!("{GENERATOR}/{}/{:#x}", self.name, self.seed);
        shape::artifact(&self.shapes(), self.unit(), &producer, options, None)
    }
}

/// The `r0-open` fixtures the P2.4 generator can hold, as `(name, artifact)`, and the ones it cannot, as
/// `(name, kind, reason)`.
pub type KnownTruth = (Vec<(String, Vec<u8>)>, Vec<(String, &'static str, String)>);

/// Runs every vendored `r0-open` fixture through the production generator.
pub fn known_truth() -> Result<KnownTruth, CorpusError> {
    use pvec_testkit::generate::Cause;
    let (mut built, mut refused) = (Vec::new(), Vec::new());
    for o in generate_all(&Bundle::vendored()?)? {
        match o.result {
            Ok(a) => built.push((o.fixture, a.bytes)),
            Err(r) => {
                let kind = match r.cause {
                    Cause::Quarantined => "QUARANTINED",
                    Cause::NotEmitted => "NOT_EMITTED",
                    Cause::MissingCapability { .. } => "MISSING_CAPABILITY",
                    _ => "REFUSED",
                };
                refused.push((o.fixture, kind, r.to_string()));
            }
        }
    }
    Ok((built, refused))
}

/// The artifact bytes of one corpus entry, rebuilt from the generator or the vendored bundle.
pub fn build(name: &str) -> Result<Vec<u8>, String> {
    build_with(name, WriterOptions::default())
}

/// [`build`] with the writer set to compress and seal as `options` say.
pub fn build_with(name: &str, options: WriterOptions) -> Result<Vec<u8>, String> {
    if let Some(spec) = Spec::named(name) {
        return spec.artifact_with(options);
    }
    let outcomes = generate_all_with(&Bundle::vendored().map_err(|e| e.to_string())?, options)
        .map_err(|e| e.to_string())?;
    outcomes
        .into_iter()
        .find(|o| o.fixture == name)
        .ok_or_else(|| format!("{name}: not a corpus entry"))?
        .result
        .map(|a| a.bytes)
        .map_err(|r| r.to_string())
}

/// Entity counts by capability name, read back from the artifact through the reference reader.
pub fn counts(artifact: &[u8]) -> Result<BTreeMap<String, u64>, String> {
    let opened = read(artifact).map_err(|e| e.to_string())?;
    let mut counts = BTreeMap::new();
    for e in &opened.entities {
        let name = match e {
            ReadEntity::Line { .. } => "line",
            ReadEntity::Polyline { .. } => "polyline",
            ReadEntity::CircleArc { start_sweep, .. } if start_sweep[1].abs() == TAU => "circle",
            ReadEntity::EllipseArc { angles, .. } if angles[2].abs() == TAU => "ellipse",
            ReadEntity::CircleArc { .. } | ReadEntity::EllipseArc { .. } => "trimmed_curve",
            ReadEntity::Spline { weights: Some(_), .. } => "rational_b_spline_curve",
            ReadEntity::Spline { .. } => "b_spline_curve",
            ReadEntity::Composite { .. } => "composite_curve",
        };
        *counts.entry(name.to_string()).or_insert(0) += 1;
    }
    Ok(counts)
}

/// The artifact's declared source bounds `[min_x, min_y, max_x, max_y]`.
pub fn bounds(artifact: &[u8]) -> Result<[f64; 4], String> {
    let Bounds { min, max } = read(artifact).map_err(|e| e.to_string())?.manifest.bounds;
    Ok([min[0], min[1], max[0], max[1]])
}

/// The 200 recorded viewports of one entry, each `[x0, y0, x1, y1]` inside `bounds`. The first is the whole
/// drawing (a cold open); the rest cycle through a half, a quarter, an eighth and a thirty-second of each
/// side, at positions drawn from a stream seeded by the entry name. Coordinates are rounded to three
/// decimals, so the CSV that records them round-trips exactly.
pub fn viewports(name: &str, bounds: [f64; 4]) -> Vec<[f64; 4]> {
    const SIDES: [f64; 4] = [0.5, 0.25, 0.125, 0.03125];
    let digest = hex_digest(name.as_bytes());
    let mut state = u64::from_str_radix(&digest[..16], 16).expect("hex");
    let mut unit = move || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    let (w, h) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
    let mut out = vec![bounds];
    for i in 1..VIEWPORTS {
        let side = SIDES[i % SIDES.len()];
        let (vw, vh) = (w * side, h * side);
        let x = bounds[0] + unit() * (w - vw);
        let y = bounds[1] + unit() * (h - vh);
        out.push([round(x), round(y), round(x + vw), round(y + vh)]);
    }
    out
}
