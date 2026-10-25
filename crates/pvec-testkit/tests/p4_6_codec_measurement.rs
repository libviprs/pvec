//! The P4.6 measurement: per block kind, on named fixtures, what each admitted codec stores, and how long it
//! takes to compress and decompress. It is `#[ignore]` because it is a measurement and not a check, and it
//! needs a release build to mean anything:
//!
//! ```text
//! cargo test --release -p pvec-testkit --test p4_6_codec_measurement -- --ignored --nocapture
//! ```
//!
//! It prints the markdown tables that `docs/spikes/04-6-compression.md` quotes, and says which machine it
//! ran on. The decision is written there, beside the numbers. A small test below runs the same code on a
//! tiny fixture so that the measurement itself cannot rot.

mod common;

use common::bundle;
use pvec_capabilities::Registry;
use pvec_components::{LayerStream, TextRecord, TextStream};
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::compress::{Codec, compress, decompress};
use pvec_container::directory::BlockKind;
use pvec_container::integrity::{block_checksum, sha256};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriterConfig, WriterOptions};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_testkit::generate::generate_all_with;
use pvec_testkit::hostile::bases::PARSE;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

fn config(name: &str) -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: format!("p4-6-measurement-{name}"),
            producer_version: "0".to_string(),
            corpus_revision: None,
        },
        bounds: None,
        capabilities: Vec::new(),
    }
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).expect("a small id")
}

fn frac(n: u64, d: f64) -> f64 {
    n as f64 / d
}

/// Blocks of about 512 entities, as ADR 0006's measurement had (mean 33,621 bytes a block): the cell of
/// an entity is its index over 512.
const PER_BLOCK: u32 = 512;

/// A plan-like drawing: mostly lines, then polylines, arcs and ellipses, coordinates on a grid of eighths
/// (rounded, as a plotted drawing is), `n` entities in blocks of 512.
fn plan_like(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::with_options(config("plan-like"), WriterOptions::default());
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for i in 0..n {
        let cell = i / PER_BLOCK;
        let (x, y) = (frac(next() % 80_000, 8.0), frac(next() % 60_000, 8.0));
        let wide = Bounds { min: [0.0, 0.0], max: [10_000.0, 7_500.0] };
        let m = EntityMeta { id: id(i), style: i % 6, cell, bounds: wide };
        match i % 8 {
            0..=4 => {
                let (dx, dy) = (frac(next() % 400, 8.0), frac(next() % 400, 8.0));
                w.push_line(m, [x, y], [x + dx + 0.125, y + dy + 0.125]).expect("a line");
            }
            5 | 6 => {
                let k = 3 + (next() % 8) as usize;
                let pts: Vec<[f64; 2]> =
                    (0..k).map(|j| [x + frac(j as u64 * 40, 8.0), y + frac(next() % 200, 8.0)]).collect();
                w.push_polyline(m, &pts).expect("a polyline");
            }
            _ => {
                let r = 0.5 + frac(next() % 400, 8.0);
                w.push_circle_arc(m, [x, y], r, [frac(next() % 50, 8.0), 1.5]).expect("an arc");
            }
        }
    }
    w.finish().expect("the plan writes")
}

/// Splines (some rational, shared knot vectors) and the lines between them, `n` of each in blocks of 512
/// lines; the advanced curves are one set of six blocks whatever their number.
fn spline_heavy(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::with_options(config("spline-heavy"), WriterOptions::default());
    let wide = Bounds { min: [0.0, 0.0], max: [10_000.0, 7_500.0] };
    for i in 0..n {
        let meta = EntityMeta { id: id(i), style: 0, cell: i / PER_BLOCK, bounds: wide };
        let f = f64::from(i);
        w.push_line(meta, [f, 0.0], [f + 1.0, 2.0]).expect("a line");
    }
    for k in 0..n {
        let degree = 2 + (k % 3) as usize;
        let points = degree + 2 + (k % 5) as usize;
        let x: Vec<f64> = (0..points).map(|j| frac((u64::from(k) * 13 + j as u64 * 7) % 4000, 8.0)).collect();
        let y: Vec<f64> = (0..points).map(|j| frac((u64::from(k) * 17 + j as u64 * 9) % 3000, 8.0)).collect();
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..points - degree).map(|v| v as f64));
        knots.extend(std::iter::repeat_n((points - degree) as f64, degree + 1));
        let weights: Option<Vec<f64>> = (k % 3 == 0)
            .then(|| (0..points).map(|j| 1.0 + frac((u64::from(k) + j as u64) % 4, 4.0)).collect());
        let spec = SplineSpec {
            degree,
            knots: &knots,
            x: &x,
            y: &y,
            weights: weights.as_deref(),
            closed: false,
            periodic: false,
        };
        let meta = EntityMeta { id: id(n + k), style: 1, cell: 0, bounds: wide };
        w.push_spline(meta, spec).expect("a spline");
    }
    w.finish().expect("the splines write")
}

/// A drawing with its layer and text streams, so the component set is a block too.
fn with_streams(n: u32) -> Vec<u8> {
    let mut w = ContainerWriter::with_options(config("with-streams"), WriterOptions::default());
    let wide = Bounds { min: [0.0, 0.0], max: [10_000.0, 7_500.0] };
    let mut layers = Vec::new();
    let mut texts = Vec::new();
    for i in 0..n {
        let meta = EntityMeta { id: id(i), style: 0, cell: i / PER_BLOCK, bounds: wide };
        let f = f64::from(i % 5000);
        w.push_line(meta, [f, 0.0], [f + 2.0, 3.0]).expect("a line");
        layers.push((id(i), i % 6));
        if i % 20 == 0 {
            texts.push(TextRecord {
                entity: id(i),
                x: f,
                y: 1.0,
                height: 2.5,
                rotation: 0.0,
                text: format!("ROOM {} LEVEL {}", i / 20, i % 4),
            });
        }
    }
    let names =
        ["WALLS", "DOORS", "WINDOWS", "FURNITURE", "ANNOTATION", "DIMENSIONS"].map(String::from).to_vec();
    let registry = Registry::v1();
    w.push_stream(
        LayerStream { flags: vec![0; names.len()], names, assignments: layers }
            .into_stream(&registry)
            .expect("layers"),
    )
    .expect("a stream");
    w.push_stream(TextStream { records: texts }.into_stream(&registry).expect("text")).expect("a stream");
    w.finish().expect("the streams write")
}

/// The decoded blocks of an artifact, by kind.
fn blocks_by_kind(art: &[u8]) -> BTreeMap<u16, Vec<Vec<u8>>> {
    let reader =
        ContainerReader::open(MemorySource::new(art.to_vec()), ReaderConfig::default()).expect("opens");
    let mut out: BTreeMap<u16, Vec<Vec<u8>>> = BTreeMap::new();
    for i in 0..reader.entries().len() {
        let kind = reader.entries()[i].block_kind().expect("a known kind");
        let block = reader.read_block(BlockId(i as u32)).expect("reads");
        out.entry(kind.id()).or_default().push(block.as_bytes().to_vec());
    }
    out
}

/// How long to spend on each timing: a real measurement, or just enough to prove the code runs.
#[derive(Clone, Copy)]
struct Effort {
    per_run: Duration,
    runs: usize,
}

const MEASURE: Effort = Effort { per_run: Duration::from_millis(150), runs: 5 };
const SMOKE: Effort = Effort { per_run: Duration::from_micros(1), runs: 1 };

/// The least time of a few runs of `f`, each repeated enough to last a measurable while.
fn best(effort: Effort, mut f: impl FnMut()) -> Duration {
    let t = Instant::now();
    f();
    let one = t.elapsed().max(Duration::from_micros(1));
    let reps = ((effort.per_run.as_nanos() / one.as_nanos()) as u32).clamp(1, 2000);
    (0..effort.runs)
        .map(|_| {
            let t = Instant::now();
            for _ in 0..reps {
                f();
            }
            t.elapsed() / reps
        })
        .min()
        .unwrap_or(one)
}

struct Row {
    fixture: String,
    kind: BlockKind,
    codec: Codec,
    blocks: usize,
    raw: usize,
    stored: usize,
    compress: Duration,
    decompress: Duration,
    checksum: Duration,
}

fn measure(effort: Effort, fixture: &str, blocks: &BTreeMap<u16, Vec<Vec<u8>>>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (kind, set) in blocks {
        let kind = BlockKind::from_id(*kind).expect("a known kind");
        let raw: usize = set.iter().map(Vec::len).sum();
        for codec in [Codec::Lz4, Codec::Deflate] {
            let stored: Vec<Vec<u8>> = set.iter().map(|b| compress(codec, b)).collect();
            let compress_t = best(effort, || {
                for b in set {
                    std::hint::black_box(compress(codec, b));
                }
            });
            let decompress_t = best(effort, || {
                for (s, b) in stored.iter().zip(set) {
                    std::hint::black_box(decompress(codec, s, b.len()).expect("round trips"));
                }
            });
            let checksum_t = best(effort, || {
                for s in &stored {
                    std::hint::black_box(block_checksum(s));
                }
            });
            rows.push(Row {
                fixture: fixture.to_string(),
                kind,
                codec,
                blocks: set.len(),
                raw,
                stored: stored.iter().map(Vec::len).sum(),
                compress: compress_t,
                decompress: decompress_t,
                checksum: checksum_t,
            });
        }
    }
    rows
}

fn mbps(bytes: usize, t: Duration) -> f64 {
    bytes as f64 / 1_000_000.0 / t.as_secs_f64().max(f64::MIN_POSITIVE)
}

fn table(rows: &[Row]) -> String {
    let mut s = String::from(
        "| fixture | block kind | codec | blocks | raw bytes | stored bytes | ratio | compress MB/s | decompress MB/s | XXH3 of the stored bytes, % of decode time |\n|---|---|---|---|---|---|---|---|---|---|\n",
    );
    for r in rows {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {:.3} | {:.0} | {:.0} | {:.1} |\n",
            r.fixture,
            r.kind.name(),
            r.codec.name(),
            r.blocks,
            r.raw,
            r.stored,
            r.stored as f64 / r.raw as f64,
            mbps(r.raw, r.compress),
            mbps(r.raw, r.decompress),
            checksum_share(r),
        ));
    }
    s
}

/// ADR 0006's reversal threshold: a block checksum that costs more than this share of block decode time
/// becomes opt-in per file.
const REVERSAL_PERCENT: f64 = 25.0;

fn checksum_share(r: &Row) -> f64 {
    100.0 * r.checksum.as_secs_f64() / r.decompress.as_secs_f64().max(f64::MIN_POSITIVE)
}

/// The worst checksum share over the rows, against the threshold, in a line the document can quote.
fn reversal_check(rows: &[Row]) -> String {
    let Some(worst) = rows.iter().max_by(|a, b| checksum_share(a).total_cmp(&checksum_share(b))) else {
        return "no rows".to_string();
    };
    let share = checksum_share(worst);
    let verdict = if share > REVERSAL_PERCENT { "OVER" } else { "under" };
    format!(
        "{verdict}: the worst XXH3 share is {share:.1}% of decode time ({} {} {}), threshold {REVERSAL_PERCENT:.0}%",
        worst.fixture,
        worst.kind.name(),
        worst.codec.name()
    )
}

fn hash_rates(effort: Effort, artifact: &[u8]) -> String {
    let xxh = best(effort, || {
        std::hint::black_box(block_checksum(artifact));
    });
    let sha = best(effort, || {
        std::hint::black_box(sha256(artifact));
    });
    format!(
        "| XXH3-64 | {:.0} |\n| SHA-256 (the safe implementation in `integrity.rs`) | {:.0} |\n",
        mbps(artifact.len(), xxh),
        mbps(artifact.len(), sha)
    )
}

fn corpus_blocks() -> BTreeMap<u16, Vec<Vec<u8>>> {
    let mut all: BTreeMap<u16, Vec<Vec<u8>>> = BTreeMap::new();
    for outcome in generate_all_with(&bundle(), PARSE).expect("the bundle registers") {
        if let Ok(a) = outcome.result {
            for (k, v) in blocks_by_kind(&a.bytes) {
                all.entry(k).or_default().extend(v);
            }
        }
    }
    all
}

#[test]
#[ignore = "a measurement: run it in release mode with --ignored --nocapture"]
fn print_the_codec_measurement() {
    let fixtures: Vec<(&str, Vec<u8>)> = vec![
        ("plan-like-200k", plan_like(200_000)),
        ("spline-heavy-20k", spline_heavy(20_000)),
        ("with-streams-50k", with_streams(50_000)),
    ];
    println!("MEASUREMENT-BEGIN");
    println!("host: {} {}", std::env::consts::ARCH, std::env::consts::OS);
    let mut rows = Vec::new();
    for (name, bytes) in &fixtures {
        rows.extend(measure(MEASURE, name, &blocks_by_kind(bytes)));
    }
    let large = rows.len();
    rows.extend(measure(MEASURE, "corpus-r0-open (16 fixtures)", &corpus_blocks()));
    println!("{}", table(&rows));
    println!("REVERSAL-CHECK (the three generated fixtures) {}", reversal_check(&rows[..large]));
    println!("REVERSAL-CHECK (all rows, tiny corpus blocks too) {}", reversal_check(&rows));
    println!("| hash | MB/s over the largest fixture |\n|---|---|");
    println!("{}", hash_rates(MEASURE, &fixtures[0].1));
    println!("MEASUREMENT-END");
}

#[test]
fn the_measurement_runs_on_a_tiny_fixture_and_the_codecs_round_trip_it() {
    let tiny = plan_like(1500);
    let blocks = blocks_by_kind(&tiny);
    let rows = measure(SMOKE, "tiny", &blocks);
    assert!(rows.len() >= 2 && rows.iter().all(|r| r.stored > 0 && r.raw > 0));
    assert!(table(&rows).contains("| tiny | hot | lz4 |"));
    assert!(hash_rates(SMOKE, &tiny).contains("XXH3-64"));
    let spl = measure(SMOKE, "tiny-splines", &blocks_by_kind(&spline_heavy(300)));
    assert!(spl.iter().any(|r| r.kind == BlockKind::Arena));
    assert!(
        measure(SMOKE, "tiny-streams", &blocks_by_kind(&with_streams(500)))
            .iter()
            .any(|r| r.kind == BlockKind::Components)
    );
}

fn row_with(decompress_us: u64, checksum_us: u64) -> Row {
    Row {
        fixture: "synthetic".to_string(),
        kind: BlockKind::Hot,
        codec: Codec::Lz4,
        blocks: 1,
        raw: 1000,
        stored: 500,
        compress: Duration::from_micros(10),
        decompress: Duration::from_micros(decompress_us),
        checksum: Duration::from_micros(checksum_us),
    }
}

#[test]
fn the_reversal_check_flags_a_block_checksum_over_a_quarter_of_decode_time() {
    // ADR 0006: a block checksum that costs more than 25% of block decode time becomes opt-in per file.
    let under = [row_with(100, 24), row_with(100, 5)];
    let over = [row_with(100, 24), row_with(100, 26)];
    assert!(reversal_check(&under).starts_with("under"), "{}", reversal_check(&under));
    let verdict = reversal_check(&over);
    assert!(verdict.starts_with("OVER"), "{verdict}");
    assert!(verdict.contains("synthetic") && verdict.contains("26.0"), "{verdict}");
    assert!(reversal_check(&[]).starts_with("no rows"));
}
