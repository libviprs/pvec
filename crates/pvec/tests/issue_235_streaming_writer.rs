//! Issue #235: a streaming writer. `PvecWriter::finish_to` writes the artifact to a sink while the build
//! stays inside a budget, with the spill files under a root the caller names. The artifact is the one
//! `finish` returns, byte for byte.
#![cfg(feature = "unstable-stream")]

#[path = "../../pvec-testkit/src/tilegen.rs"]
mod tilegen;

use pvec::components::Stream;
use pvec::unstable::{Band, PixelFormat, Placement, RasterId, TileCodec, TileIn};
use pvec::{Bounds2, Curve, CurveRef, EntityId, Manifest, PvecError, PvecReader, PvecWriter};
use pvec_capabilities::Registry;
use pvec_components::LayerStream;
use pvec_geometry::{BSpline, Circle, Ellipse, Line, Placement2, Point2, Polyline, Trimmed, Vector2};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const MAX_ERROR: f64 = 0.05;

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-235".into(),
        producer_version: "1".into(),
    }
}

fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

/// A small deterministic generator, so a failing case can be replayed from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A fresh, empty directory under the system temp dir: the root a test hands to the writer.
fn root(tag: &str) -> PathBuf {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "pvec235-{}-{tag}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn entries(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

/// The entities of one drawing. Ids are dense, so a Tier 0 bound can be asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mix {
    /// Only hot geometry: short polylines.
    Short,
    /// Hot geometry of every kind, a long wave, splines and a composite.
    Everything,
}

/// Pushes `n` entities (ids 0..n) in the order a seeded shuffle gives.
fn push_drawing(w: &mut PvecWriter, n: u32, mix: Mix, seed: u64) {
    let mut rng = Rng(seed | 1);
    let mut order: Vec<u32> = (0..n).collect();
    for i in (1..order.len()).rev() {
        order.swap(i, rng.below(i as u64 + 1) as usize);
    }
    // the geometry of entity i must not depend on the push order, so draw it from the id
    let at = |i: u32| {
        let mut r = Rng(u64::from(i).wrapping_mul(0x2545_f491_4f6c_dd1d) ^ seed | 1);
        (r.unit() * 900.0, r.unit() * 900.0, r.below(1000))
    };
    // the composite of the Everything mix names the two splines before it
    let composite = n.saturating_sub(1);
    for i in order {
        let (x, y, k) = at(i);
        let eid = id(i);
        // the two curves a composite names are splines that join, so the reader accepts the composite
        let joined = |dx: f64| {
            let (x0, y0) = (100.0 + dx, 100.0);
            Curve::BSpline(
                BSpline::new(
                    2,
                    vec![p(x0, y0), p(x0 + 30.0, y0 + 40.0), p(x0 + 60.0, y0)],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                )
                .unwrap(),
            )
        };
        let curve = match (mix, k % 9) {
            (Mix::Everything, _) if n >= 3 && i == n - 3 => joined(0.0),
            (Mix::Everything, _) if n >= 3 && i == n - 2 => joined(60.0),
            (Mix::Short, _) | (_, 0..=2) => {
                Curve::Polyline(Polyline::new(vec![p(x, y), p(x + 4.0, y + 3.0)]).unwrap())
            }
            (Mix::Everything, 3) => {
                let wave: Vec<Point2> = (0..70 + k as u32 % 40)
                    .map(|j| p(x + f64::from(j), y + (f64::from(j) * 0.2).sin() * 9.0))
                    .collect();
                Curve::Polyline(Polyline::new(wave).unwrap())
            }
            (Mix::Everything, 4) => {
                let frame = Placement2::new(p(x, y), Vector2::new(1.0, 0.0)).unwrap();
                Curve::Circle(Circle::new(frame, 3.0 + (k % 7) as f64).unwrap())
            }
            (Mix::Everything, 5) => {
                let frame = Placement2::new(p(x, y), Vector2::new(1.0, 0.0)).unwrap();
                let c = Curve::Circle(Circle::new(frame, 8.0).unwrap());
                Curve::Trimmed(Trimmed::new(c, 0.25, 2.0 + (k % 3) as f64, true).unwrap())
            }
            (Mix::Everything, 6) => {
                // a rotated frame, so the writer derives an angle with the deterministic arctangent
                let frame = Placement2::new(p(x, y), Vector2::new(0.6, 0.8)).unwrap();
                Curve::Ellipse(Ellipse::new(frame, 20.0, 6.0).unwrap())
            }
            (Mix::Everything, 7) => {
                let l = Line::new(p(x, y), Vector2::new(1.0, 0.5)).unwrap();
                Curve::Trimmed(Trimmed::new(Curve::Line(l), 0.0, 5.0 + (k % 5) as f64, true).unwrap())
            }
            (Mix::Everything, _) => Curve::BSpline(
                BSpline::new(
                    2,
                    vec![p(x, y), p(x + 30.0, y + 40.0), p(x + 60.0, y)],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                )
                .unwrap(),
            ),
        };
        if mix == Mix::Everything && i == composite && n >= 3 {
            let bounds = Bounds2 { min: [100.0, 100.0], max: [220.0, 140.0] };
            let segs = [CurveRef { entity: id(n - 3) }, CurveRef { entity: id(n - 2) }];
            w.push_composite(eid, &segs, bounds, false).unwrap();
        } else {
            w.push(eid, &curve).unwrap();
        }
    }
}

fn layer_stream(n: u32) -> Stream {
    let layers = LayerStream {
        names: vec!["walls".into(), "doors".into()],
        flags: vec![0, 0],
        assignments: (0..n.min(200)).map(|i| (id(i), i % 2)).collect(),
    };
    layers.into_stream(&Registry::v1()).unwrap()
}

fn placement() -> Placement {
    Placement {
        pixel_to_source: [0.5, 0.25, 0.125, -0.5, 100.0, 50.0],
        width: 700,
        height: 500,
        visible_box: [10, 20, 690, 480],
        tile_edge: 256,
        levels: 3,
        tile_codec: TileCodec::Png,
        pixel_format: PixelFormat::Gray,
        bit_depth: 8,
        opacity: 200,
        band: Band::Overlay,
        layer_index: None,
        source_digest: Some([7; 32]),
        attribution: "Survey 2024".into(),
    }
}

fn tiles() -> Vec<TileIn> {
    let mut out = Vec::new();
    for (l, (cols, rows)) in [(1u32, 1u32), (2, 1), (3, 2)].into_iter().enumerate() {
        for row in 0..rows {
            for col in 0..cols {
                let bytes = if (l, col, row) == (2, 2, 1) {
                    Vec::new()
                } else {
                    tilegen::png(
                        256,
                        1,
                        8,
                        &tilegen::payload(843, (13 * col + 29 * row + 71 * l as u32) as u8),
                    )
                };
                out.push(TileIn { level: l as u32, col, row, bytes });
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
struct Shape {
    /// `None`: the plain writer. `Some((capacity, tier0))`: the indexed one.
    indexed: Option<(u32, Option<f64>)>,
    n: u32,
    mix: Mix,
    components: bool,
    raster: bool,
}

fn build(shape: Shape, seed: u64) -> PvecWriter {
    let mut w = match shape.indexed {
        Some((capacity, tier0)) => PvecWriter::new_indexed(manifest(), capacity, tier0).unwrap(),
        None => PvecWriter::new(manifest()).unwrap(),
    };
    push_drawing(&mut w, shape.n, shape.mix, seed);
    if shape.components {
        w.components(layer_stream(shape.n)).unwrap();
    }
    if shape.raster {
        w.rasters(&[placement()]).unwrap();
        w.raster_tiles(RasterId::new(0).unwrap(), tiles()).unwrap();
    }
    w
}

fn spilled(shape: Shape, seed: u64, root: &Path, budget: u64) -> PvecWriter {
    let w = PvecWriter::with_spill(build_empty(shape), root, budget).unwrap();
    fill(w, shape, seed)
}

fn build_empty(shape: Shape) -> PvecWriter {
    match shape.indexed {
        Some((capacity, tier0)) => PvecWriter::new_indexed(manifest(), capacity, tier0).unwrap(),
        None => PvecWriter::new(manifest()).unwrap(),
    }
}

fn fill(mut w: PvecWriter, shape: Shape, seed: u64) -> PvecWriter {
    push_drawing(&mut w, shape.n, shape.mix, seed);
    if shape.components {
        w.components(layer_stream(shape.n)).unwrap();
    }
    if shape.raster {
        w.rasters(&[placement()]).unwrap();
        w.raster_tiles(RasterId::new(0).unwrap(), tiles()).unwrap();
    }
    w
}

fn shapes() -> Vec<Shape> {
    let mut out = Vec::new();
    for indexed in [None, Some((16, None)), Some((16, Some(MAX_ERROR))), Some((3, Some(MAX_ERROR)))] {
        for (n, mix) in [(1, Mix::Short), (40, Mix::Short), (240, Mix::Everything)] {
            for (components, raster) in [(false, false), (true, false), (true, true)] {
                out.push(Shape { indexed, n, mix, components, raster });
            }
        }
    }
    out
}

// ------------------------------------------------------------------ the artifact is the same bytes

#[test]
fn issue_235_the_streamed_artifact_is_the_one_finish_returns_for_every_budget() {
    let tmp = root("equal");
    for shape in shapes() {
        let expect = build(shape, 7).finish().unwrap();
        for budget in [1u64, 4096, 1 << 30] {
            let mut out = Vec::new();
            let n = spilled(shape, 7, &tmp, budget).finish_to(&mut out).unwrap();
            assert_eq!(n as usize, out.len(), "{shape:?} budget {budget}: the count is the bytes written");
            assert!(out == expect, "{shape:?} budget {budget}: the streamed artifact differs from finish()");
            assert_eq!(entries(&tmp), 0, "{shape:?} budget {budget}: the spill is gone after finish_to");
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn issue_235_the_streamed_artifact_reads_back() {
    let tmp = root("reads");
    let shape = Shape {
        indexed: Some((16, Some(MAX_ERROR))),
        n: 240,
        mix: Mix::Everything,
        components: true,
        raster: true,
    };
    let mut out = Vec::new();
    spilled(shape, 3, &tmp, 4096).finish_to(&mut out).unwrap();
    let r = PvecReader::open(out).unwrap();
    assert_eq!(r.entities().unwrap().len(), 240);
    let dir = r.spatial_directory().unwrap().expect("a directory");
    assert!(dir.cells() > 1);
    assert_eq!(r.raster_placements().unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

// -------------------------------------------------------------- the spill is under the caller's root

/// A sink that looks at the root when the first byte arrives.
struct Peek<'a> {
    root: &'a Path,
    seen: Option<Vec<String>>,
    bytes: Vec<u8>,
}

impl Write for Peek<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.seen.is_none() {
            let mut names: Vec<String> = std::fs::read_dir(self.root)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            self.seen = Some(names);
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn issue_235_the_spill_files_live_under_the_root_the_caller_names_and_are_gone_after() {
    let tmp = root("under");
    let shape = Shape {
        indexed: Some((16, Some(MAX_ERROR))),
        n: 240,
        mix: Mix::Everything,
        components: true,
        raster: true,
    };
    let mut sink = Peek { root: &tmp, seen: None, bytes: Vec::new() };
    spilled(shape, 5, &tmp, 4096).finish_to(&mut sink).unwrap();
    let seen = sink.seen.expect("the artifact was written");
    assert_eq!(seen.len(), 1, "one spill directory under the root while the artifact is written: {seen:?}");
    assert_eq!(entries(&tmp), 0, "and nothing once it is done");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn issue_235_dropping_a_writer_that_never_finished_removes_its_spill() {
    let tmp = root("drop");
    let shape =
        Shape { indexed: Some((16, None)), n: 240, mix: Mix::Everything, components: true, raster: true };
    let w = spilled(shape, 5, &tmp, 64);
    assert!(entries(&tmp) > 0, "a writer with a spill has made its directory by the time it holds pushes");
    drop(w);
    assert_eq!(entries(&tmp), 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn issue_235_a_root_that_cannot_hold_a_spill_is_a_typed_io_error_at_once() {
    let missing = std::env::temp_dir().join(format!("pvec235-missing-{}", std::process::id()));
    let err = PvecWriter::with_spill(PvecWriter::new(manifest()).unwrap(), &missing, 1 << 20).unwrap_err();
    match err {
        PvecError::Io { stage, error } => {
            assert_eq!(stage, "spill");
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }
        other => panic!("expected an Io error, got {other:?}"),
    }
    assert!(!missing.exists(), "the writer does not invent a root");
}

#[test]
fn issue_235_a_writer_that_already_holds_pushes_cannot_be_given_a_spill() {
    let tmp = root("late");
    let mut w = PvecWriter::new(manifest()).unwrap();
    push_drawing(&mut w, 4, Mix::Short, 1);
    let err = PvecWriter::with_spill(w, &tmp, 1 << 20).unwrap_err();
    assert!(matches!(err, PvecError::Write { .. }), "{err:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

// ------------------------------------------------------------------------- a failing sink

/// A sink that takes `room` bytes and then fails, as a full disk or a closed pipe does.
struct Full {
    room: usize,
    kind: io::ErrorKind,
    taken: usize,
}

impl Write for Full {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.taken >= self.room {
            return Err(io::Error::new(self.kind, "the sink is full"));
        }
        let n = buf.len().min(self.room - self.taken);
        self.taken += n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn issue_235_a_failing_sink_is_a_typed_io_error_with_the_io_error_as_source_and_no_spill_left() {
    let tmp = root("full");
    for shape in [
        Shape {
            indexed: Some((16, Some(MAX_ERROR))),
            n: 240,
            mix: Mix::Everything,
            components: true,
            raster: true,
        },
        Shape { indexed: None, n: 40, mix: Mix::Short, components: false, raster: false },
    ] {
        let total = build(shape, 9).finish().unwrap().len();
        for room in [0, 1, 31, 32, 100, total / 3, total / 2, total - 1] {
            let mut sink = Full { room, kind: io::ErrorKind::StorageFull, taken: 0 };
            let err = spilled(shape, 9, &tmp, 512).finish_to(&mut sink).unwrap_err();
            match &err {
                PvecError::Io { stage, error } => {
                    assert_eq!(*stage, "output", "room {room}");
                    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
                }
                other => panic!("room {room}: expected an Io error, got {other:?}"),
            }
            let source = std::error::Error::source(&err).expect("the io error is the source");
            assert!(source.downcast_ref::<io::Error>().is_some());
            assert_eq!(entries(&tmp), 0, "room {room}: no spill file is left behind");
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn issue_235_a_build_that_refuses_writes_nothing_to_the_sink() {
    let tmp = root("refuse");
    // two entities with one id: the build refuses, and the sink must not have seen a byte
    for indexed in [None, Some((16, None)), Some((16, Some(MAX_ERROR)))] {
        let w = build_empty(Shape { indexed, n: 0, mix: Mix::Short, components: false, raster: false });
        let mut w = PvecWriter::with_spill(w, &tmp, 256).unwrap();
        push_drawing(&mut w, 30, Mix::Short, 4);
        w.push(id(5), &Curve::Polyline(Polyline::new(vec![p(1.0, 1.0), p(2.0, 2.0)]).unwrap())).unwrap();
        let mut sink = Peek { root: &tmp, seen: None, bytes: Vec::new() };
        let err = w.finish_to(&mut sink).unwrap_err();
        assert!(matches!(err, PvecError::Write { .. }), "{indexed:?}: {err:?}");
        assert!(sink.bytes.is_empty(), "{indexed:?}: bytes reached the sink");
        assert_eq!(entries(&tmp), 0);
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn issue_235_the_refusals_are_the_ones_finish_gives() {
    let tmp = root("rules");
    // a leaf capacity of 0, a Tier 0 bound that is not a bound, and a file with no entity
    let cases: [Option<(u32, Option<f64>)>; 3] =
        [Some((0, None)), Some((16, Some(f64::NAN))), Some((16, Some(-1.0)))];
    for indexed in cases {
        let shape = Shape { indexed, n: 30, mix: Mix::Short, components: false, raster: false };
        let expect = build(shape, 2).finish().unwrap_err();
        let got = spilled(shape, 2, &tmp, 1024).finish_to(&mut Vec::new()).unwrap_err();
        assert_eq!(got, expect, "{indexed:?}");
    }
    let expect = PvecWriter::new(manifest()).unwrap().finish().unwrap_err();
    let w = PvecWriter::with_spill(PvecWriter::new(manifest()).unwrap(), &tmp, 1024).unwrap();
    assert_eq!(w.finish_to(&mut Vec::new()).unwrap_err(), expect);
    assert_eq!(entries(&tmp), 0);
    let _ = std::fs::remove_dir_all(&tmp);
}

// ----------------------------------------------------------- tiny budgets and truncated sinks, replayed

#[test]
fn issue_235_random_drawings_tiny_budgets_and_truncated_sinks_never_panic_or_leak() {
    let tmp = root("fuzz");
    let mut rng = Rng(0x05ee_d235);
    for case in 0..40u32 {
        let indexed = match rng.below(4) {
            0 => None,
            1 => Some((1 + rng.below(40) as u32, None)),
            _ => Some((1 + rng.below(40) as u32, Some(MAX_ERROR))),
        };
        let shape = Shape {
            indexed,
            n: 8 + rng.below(300) as u32,
            mix: if rng.below(2) == 0 { Mix::Short } else { Mix::Everything },
            components: rng.below(2) == 0,
            raster: rng.below(3) == 0,
        };
        let budget = [1u64, 7, 100, 2000, 70_000][rng.below(5) as usize];
        let seed = rng.next();
        let expect = build(shape, seed).finish();
        let mut full = Vec::new();
        let got = spilled(shape, seed, &tmp, budget).finish_to(&mut full);
        match (&expect, &got) {
            (Ok(a), Ok(n)) => {
                assert!(a == &full, "case {case} {shape:?} budget {budget}: bytes differ");
                assert_eq!(*n as usize, a.len());
            }
            (Err(a), Err(b)) => assert_eq!(a, b, "case {case}"),
            _ => panic!(
                "case {case} {shape:?} budget {budget}: finish gave {expect:?} but finish_to gave {got:?}"
            ),
        }
        if let Ok(a) = &expect {
            let room = rng.below(a.len() as u64) as usize;
            let mut sink = Full { room, kind: io::ErrorKind::BrokenPipe, taken: 0 };
            let err = spilled(shape, seed, &tmp, budget).finish_to(&mut sink).unwrap_err();
            assert!(matches!(err, PvecError::Io { stage: "output", .. }), "case {case}: {err:?}");
        }
        assert_eq!(entries(&tmp), 0, "case {case}: the root is empty");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

// ------------------------------------------------------------------------------------ the memory

/// What the child process of the memory tests builds: `entities` two-point polylines, indexed with Tier 0,
/// streamed into a sink that only counts. The parent reads the child's peak resident set.
const CHILD: &str = "PVEC_235_CHILD";

/// The peak resident set of this process, in KiB (Linux `VmHWM`).
fn peak_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

struct Count(u64);

impl Write for Count {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn polyline(i: u32) -> Curve {
    let (x, y) = (f64::from(i % 1000), f64::from(i / 1000 % 1000));
    Curve::Polyline(Polyline::new(vec![p(x, y), p(x + 0.5, y + 0.25)]).unwrap())
}

/// The child: builds the drawing and prints its peak RSS.
#[test]
fn issue_235_child_process_for_the_memory_tests() {
    let Ok(spec) = std::env::var(CHILD) else { return };
    let mut parts = spec.split(',');
    let n: u32 = parts.next().unwrap().parse().unwrap();
    let budget: u64 = parts.next().unwrap().parse().unwrap();
    let tmp = root("rss");
    let mut w = PvecWriter::new_indexed(manifest(), pvec::unstable::CELL_CAPACITY, Some(MAX_ERROR)).unwrap();
    w = PvecWriter::with_spill(w, &tmp, budget).unwrap();
    for i in 0..n {
        w.push(id(i), &polyline(i)).unwrap();
    }
    let mut sink = Count(0);
    let bytes = w.finish_to(&mut sink).unwrap();
    println!("RESULT bytes={bytes} peak_kib={}", peak_rss_kib().unwrap_or(0));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Runs the child and returns (artifact bytes, peak RSS in KiB).
fn measure(n: u32, budget: u64) -> Option<(u64, u64)> {
    peak_rss_kib()?;
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "issue_235_child_process_for_the_memory_tests", "--nocapture", "--test-threads=1"])
        .env(CHILD, format!("{n},{budget}"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "the child failed: {text}\n{}", String::from_utf8_lossy(&out.stderr));
    let line = text
        .lines()
        .find_map(|l| l.find("RESULT ").map(|at| &l[at..]))
        .unwrap_or_else(|| panic!("the child printed no result: {text}"));
    let field = |name: &str| -> u64 {
        line.split_whitespace().find_map(|f| f.strip_prefix(name)).unwrap().parse().unwrap()
    };
    Some((field("bytes="), field("peak_kib=")))
}

const BUDGET: u64 = 32 << 20;
/// The budget, the largest block of the drawing, and the process's own footprint, in KiB. The
/// two-point polylines of the drawing make a block of a leaf a few hundred KiB.
const CEILING_KIB: u64 = 2 * (BUDGET >> 10) + 48 * 1024;

#[test]
fn issue_235_peak_rss_at_one_million_entities_stays_under_the_budget() {
    let Some((bytes, peak)) = measure(1_000_000, BUDGET) else {
        eprintln!("no /proc/self/status here, so the peak RSS cannot be read");
        return;
    };
    assert!(bytes > 1_000_000, "the artifact is {bytes} bytes");
    assert!(
        peak <= CEILING_KIB,
        "peak RSS {peak} KiB is over {CEILING_KIB} KiB (budget {} KiB plus the largest block and the process)",
        BUDGET >> 10
    );
}

#[test]
fn issue_235_peak_rss_does_not_grow_with_the_entity_count() {
    let Some((_, small)) = measure(250_000, BUDGET) else { return };
    let Some((_, large)) = measure(1_000_000, BUDGET) else { return };
    assert!(
        large * 100 <= small * 125,
        "four times the entities moved the peak from {small} KiB to {large} KiB (limit +25%)"
    );
}
