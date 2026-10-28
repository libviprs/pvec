//! Layouts, index designs and compression on the pinned corpus (issue P8.3). This file is the shipped half; the
//! alternatives that are not in the container run in the spikes (`spikes/spatial-index` for the index models and
//! the block size sweep, `spikes/codec-sweep` for the codecs), on the same entries, and write the same rows.
//!
//! ```text
//! layout prepare ENTRY OUT                  write ENTRY.pvec (no compression) and ENTRY.shapes for the spikes
//! layout decode  ENTRY OUT PLATFORM         decode throughput: SoA against per-entity records and AoS, arenas
//!                                           against per-entity variable records
//! layout shipped ENTRY OUT PLATFORM         the shipped index and block size: ranges and bytes over the 200
//!                                           recorded viewports, index share of the file
//! layout header                             the CSV header
//! layout report CSV                         the verdict lines the document records
//! ```
//!
//! Rows are tidy (`table,entry,platform,arm,param,metric,value,note`), one number per row, on stdout.

use pvec_bench::dump::dump;
use pvec_bench::layout_bench::{HEADER, Row, parse_rows, verdicts};
use pvec_bench::shape::{Shape, from_artifact};
use pvec_bench::{bounds, build, build_with, viewports};
use pvec_container::blocks::advanced::CurveView;
use pvec_container::compress::CodecPolicy;
use pvec_container::directory::BlockKind;
use pvec_container::source::{MemorySource, ReadAt, ReadAtSource, Recording, SourceError};
use pvec_container::writer::WriterOptions;
use pvec_container::{BlockId, ContainerReader, ReaderConfig};
use pvec_stream::{Aabb, SpatialDirectory, StraddlerIndex, ViewportConfig, ViewportQuery};
use std::f64::consts::TAU;
use std::fs;
use std::hint::black_box;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct FileAt(fs::File);

impl ReadAt for FileAt {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        self.0.read_at(buf, offset).map_err(|e| SourceError::Io(e.to_string()))
    }
}

struct Out<'a> {
    entry: &'a str,
    platform: &'a str,
}

impl Out<'_> {
    fn row(&self, table: &str, arm: &str, param: &str, metric: &str, value: Option<f64>, note: &str) {
        let r = Row {
            table: table.into(),
            entry: self.entry.into(),
            platform: self.platform.into(),
            arm: arm.into(),
            param: param.into(),
            metric: metric.into(),
            value,
            note: note.into(),
        };
        println!("{}", r.line());
    }
}

fn path(out: &Path, entry: &str) -> PathBuf {
    out.join(format!("{entry}.pvec"))
}

/// The file the shipped writer defaults make, which is compressed since P8.6 (the one `path` names is not).
fn default_path(out: &Path, entry: &str) -> PathBuf {
    out.join(format!("{entry}.default.pvec"))
}

fn prepare(entry: &str, out: &Path) {
    // The layout arms are measured on the file with no compression, the way the spikes read it: block size is
    // what P8.3 compares, and a compressed file would be a different experiment from the models'. The writer
    // defaults compress, so this one names the policy.
    let uncompressed = WriterOptions { codecs: CodecPolicy::NONE, ..WriterOptions::default() };
    let raw = build_with(entry, uncompressed).unwrap_or_else(|e| panic!("{e}"));
    fs::write(path(out, entry), &raw).expect("write .pvec");
    fs::write(default_path(out, entry), build(entry).unwrap_or_else(|e| panic!("{e}")))
        .expect("write default");
    let shapes = from_artifact(&raw).expect("shapes");
    fs::write(out.join(format!("{entry}.shapes")), dump(&shapes, bounds(&raw).expect("bounds")))
        .expect("write dump");
}

// ---------------------------------------------------------------- decode

/// A wrapping sum of every number a decoder has to produce: ids, styles (zero here) and the bits of every
/// coordinate. Order does not matter, so every layout must give the same total for the same entities.
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
struct Sum(u64);

impl Sum {
    fn id(&mut self, v: u32) {
        self.0 = self.0.wrapping_add(u64::from(v));
    }
    fn f(&mut self, v: f64) {
        self.0 = self.0.wrapping_add(v.to_bits());
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct LineRow {
    id: u32,
    style: u32,
    a: [f64; 2],
    b: [f64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ArcRow {
    id: u32,
    style: u32,
    c: [f64; 2],
    r: f64,
    start_sweep: [f64; 2],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct EllipseRow {
    id: u32,
    style: u32,
    c: [f64; 2],
    radii: [f64; 2],
    angles: [f64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PolyRow {
    id: u32,
    style: u32,
    first: u32,
    n: u32,
}

/// Array of structs: one row type per kind, vertices interleaved.
#[derive(Default)]
struct Aos {
    lines: Vec<LineRow>,
    arcs: Vec<ArcRow>,
    ellipses: Vec<EllipseRow>,
    polys: Vec<PolyRow>,
    pts: Vec<[f64; 2]>,
}

impl Aos {
    fn bytes(&self) -> usize {
        self.lines.len() * size_of::<LineRow>()
            + self.arcs.len() * size_of::<ArcRow>()
            + self.ellipses.len() * size_of::<EllipseRow>()
            + self.polys.len() * size_of::<PolyRow>()
            + self.pts.len() * 16
    }
    fn decode(&self) -> Sum {
        let mut s = Sum::default();
        for l in &self.lines {
            s.id(l.id);
            s.id(l.style);
            [l.a[0], l.a[1], l.b[0], l.b[1]].into_iter().for_each(|v| s.f(v));
        }
        for a in &self.arcs {
            s.id(a.id);
            s.id(a.style);
            [a.c[0], a.c[1], a.r, a.start_sweep[0], a.start_sweep[1]].into_iter().for_each(|v| s.f(v));
        }
        for e in &self.ellipses {
            s.id(e.id);
            s.id(e.style);
            [e.c[0], e.c[1], e.radii[0], e.radii[1], e.angles[0], e.angles[1], e.angles[2]]
                .into_iter()
                .for_each(|v| s.f(v));
        }
        for p in &self.polys {
            s.id(p.id);
            s.id(p.style);
            for q in &self.pts[p.first as usize..(p.first + p.n) as usize] {
                s.f(q[0]);
                s.f(q[1]);
            }
        }
        s
    }
}

fn aos_of(shapes: &[(u32, &Shape)]) -> Aos {
    let mut a = Aos::default();
    for (id, s) in shapes {
        match s {
            Shape::Line(p, q) => a.lines.push(LineRow { id: *id, style: 0, a: *p, b: *q }),
            Shape::Arc { centre, radius, start_sweep } => {
                a.arcs.push(ArcRow { id: *id, style: 0, c: *centre, r: *radius, start_sweep: *start_sweep })
            }
            Shape::Ellipse { centre, radii, angles } => {
                a.ellipses.push(EllipseRow { id: *id, style: 0, c: *centre, radii: *radii, angles: *angles })
            }
            Shape::Polyline(p) => {
                a.polys.push(PolyRow { id: *id, style: 0, first: a.pts.len() as u32, n: p.len() as u32 });
                a.pts.extend_from_slice(p);
            }
            Shape::Spline(_) => {}
        }
    }
    a
}

/// One variable-length record per entity, back to back: a tag byte, id, style, a count where the kind has one,
/// then the numbers, all little endian and unaligned.
fn records_of(shapes: &[(u32, &Shape)]) -> Vec<u8> {
    let mut out = Vec::new();
    let f = |out: &mut Vec<u8>, vs: &[f64]| vs.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    for (id, s) in shapes {
        let head = |out: &mut Vec<u8>, tag: u8| {
            out.push(tag);
            out.extend_from_slice(&id.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
        };
        match s {
            Shape::Line(p, q) => {
                head(&mut out, 1);
                f(&mut out, &[p[0], p[1], q[0], q[1]]);
            }
            Shape::Arc { centre, radius, start_sweep } => {
                head(&mut out, 3);
                f(&mut out, &[centre[0], centre[1], *radius, start_sweep[0], start_sweep[1]]);
            }
            Shape::Ellipse { centre, radii, angles } => {
                head(&mut out, 4);
                f(&mut out, &[centre[0], centre[1], radii[0], radii[1], angles[0], angles[1], angles[2]]);
            }
            Shape::Polyline(p) => {
                head(&mut out, 2);
                out.extend_from_slice(&(p.len() as u32).to_le_bytes());
                p.iter().for_each(|q| f(&mut out, q));
            }
            Shape::Spline(_) => {}
        }
    }
    out
}

fn decode_records(b: &[u8]) -> Sum {
    let mut s = Sum::default();
    let (mut at, mut take_f) = (0usize, 0usize);
    let u32at = |at: usize| u32::from_le_bytes(b[at..at + 4].try_into().expect("4"));
    let f64at = |at: usize| f64::from_le_bytes(b[at..at + 8].try_into().expect("8"));
    while at < b.len() {
        let tag = b[at];
        s.id(u32at(at + 1));
        s.id(u32at(at + 5));
        at += 9;
        let n = match tag {
            1 => 4,
            3 => 5,
            4 => 7,
            _ => {
                let n = u32at(at) as usize;
                at += 4;
                2 * n
            }
        };
        for k in 0..n {
            s.f(f64at(at + 8 * k));
        }
        take_f += n;
        at += 8 * n;
    }
    black_box(take_f);
    s
}

/// A pass is repeated until it has run for 150 ms, five times; the median is the number.
fn time_pass(mut f: impl FnMut() -> Sum) -> (f64, Sum) {
    let mut last = Sum::default();
    let mut runs: Vec<f64> = (0..5)
        .map(|_| {
            let (start, mut reps) = (Instant::now(), 0u32);
            while start.elapsed().as_millis() < 150 || reps == 0 {
                last = black_box(f());
                reps += 1;
            }
            start.elapsed().as_secs_f64() / f64::from(reps)
        })
        .collect();
    runs.sort_by(f64::total_cmp);
    (runs[2], last)
}

fn decode(entry: &str, dir: &Path, platform: &str) {
    let o = Out { entry, platform };
    let bytes = fs::read(path(dir, entry)).expect("the .pvec");
    let shapes = from_artifact(&bytes).expect("shapes");
    let numbered: Vec<(u32, &Shape)> = shapes.iter().enumerate().map(|(i, s)| (i as u32, s)).collect();
    let hot: Vec<(u32, &Shape)> =
        numbered.iter().copied().filter(|(_, s)| !matches!(s, Shape::Spline(_))).collect();
    let splines: Vec<(u32, &Shape)> =
        numbered.iter().copied().filter(|(_, s)| matches!(s, Shape::Spline(_))).collect();
    let mib = |n: usize| n as f64 / (1024.0 * 1024.0);
    let no_perf = "perf is not available in the container, so no instruction or cache-miss count";
    let report = |arm: &str, n: usize, encoded: usize, secs: f64, note: &str| {
        o.row("decode", arm, "", "entities_per_s", Some(n as f64 / secs), note);
        o.row(
            "decode",
            arm,
            "",
            "mib_per_s",
            Some(mib(encoded) / secs),
            &format!("{encoded} encoded bytes; {note}"),
        );
        o.row("decode", arm, "", "instr_per_entity", None, no_perf);
        o.row("decode", arm, "", "cache_miss_per_entity", None, no_perf);
    };

    // The shipped hot blocks, read from the file and held decoded in memory: the pass is the columns scan.
    let r =
        ContainerReader::open(MemorySource::new(bytes.as_slice()), ReaderConfig::default()).expect("opens");
    let ids: Vec<BlockId> = r
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot))
        .map(|(i, _)| BlockId(i as u32))
        .collect();
    let blocks = r.read_blocks(&ids).expect("blocks");
    let soa_bytes: usize = blocks.iter().map(|b| b.as_bytes().len()).sum();
    let views: Vec<_> = blocks.iter().map(|b| b.hot().expect("a hot block")).collect();
    let soa = |views: &[pvec_container::blocks::hot::HotBlockView<'_>]| {
        let mut s = Sum::default();
        for v in views {
            v.entity_ids().iter().for_each(|i| s.id(*i));
            v.styles().iter().for_each(|i| s.id(*i));
            for col in v.f64_columns() {
                col.iter().for_each(|x| s.f(*x));
            }
        }
        s
    };
    let (t_soa, sum_soa) = time_pass(|| soa(&views));

    let records = records_of(&hot);
    let (t_rec, sum_rec) = time_pass(|| decode_records(&records));
    let aos = aos_of(&hot);
    let (t_aos, sum_aos) = time_pass(|| aos.decode());
    let agree = sum_soa == sum_rec && sum_rec == sum_aos;
    let note = if agree {
        "the three layouts produce the same checksum".to_string()
    } else {
        "CHECKSUMS DIFFER".to_string()
    };
    assert!(agree, "{entry}: the layouts decode different numbers: {sum_soa:?} {sum_rec:?} {sum_aos:?}");
    report("soa", hot.len(), soa_bytes, t_soa, &format!("the shipped hot blocks; {note}"));
    report(
        "record",
        hot.len(),
        records.len(),
        t_rec,
        &format!("one variable-length record per entity; {note}"),
    );
    report(
        "aos",
        hot.len(),
        aos.bytes(),
        t_aos,
        &format!("a row struct per kind, vertices interleaved; {note}"),
    );

    // The advanced curves: shipped arenas and offset tables against one variable record per spline.
    if splines.is_empty() {
        for arm in ["arena", "variable"] {
            o.row("decode", arm, "", "entities_per_s", None, "the entry has no B-splines");
        }
        return;
    }
    let sets: Vec<_> = (0..r.advanced_chunk_count().expect("chunks"))
        .map(|c| r.advanced(c).expect("chunk").expect("set"))
        .collect();
    let arena_bytes: usize = r
        .entries()
        .iter()
        .filter(|e| matches!(e.block_kind(), Some(BlockKind::Records | BlockKind::Arena)))
        .map(|e| e.uncompressed_length as usize)
        .sum();
    let arena = |sets: &[pvec_container::reader::AdvancedSet]| {
        let mut s = Sum::default();
        for set in sets {
            let v = set.view().expect("view");
            for i in 0..v.len() {
                if let Ok(CurveView::Spline(sp)) = v.curve(i) {
                    s.id(sp.id);
                    s.id(sp.style);
                    sp.x.iter().chain(sp.y).chain(sp.knots).for_each(|x| s.f(*x));
                    sp.weights.into_iter().flatten().for_each(|x| s.f(*x));
                }
            }
        }
        s
    };
    let (t_arena, sum_arena) = time_pass(|| arena(&sets));
    // One record per spline: id, style, degree, point count, knot count, flag, then the arrays.
    let mut var = Vec::new();
    for (id, s) in &splines {
        let Shape::Spline(sp) = s else { continue };
        for v in [
            *id,
            0,
            sp.degree as u32,
            sp.x.len() as u32,
            sp.knots.len() as u32,
            u32::from(sp.weights.is_some()),
        ] {
            var.extend_from_slice(&v.to_le_bytes());
        }
        for x in sp.x.iter().chain(&sp.y).chain(&sp.knots).chain(sp.weights.iter().flatten()) {
            var.extend_from_slice(&x.to_le_bytes());
        }
    }
    let decode_var = || {
        let (mut s, mut at) = (Sum::default(), 0usize);
        let u32at = |at: usize| u32::from_le_bytes(var[at..at + 4].try_into().expect("4"));
        while at < var.len() {
            let (id, style, n, k, w) =
                (u32at(at), u32at(at + 4), u32at(at + 12) as usize, u32at(at + 16) as usize, u32at(at + 20));
            s.id(id);
            s.id(style);
            at += 24;
            let count = 2 * n + k + if w == 1 { n } else { 0 };
            for j in 0..count {
                s.f(f64::from_le_bytes(var[at + 8 * j..at + 8 * j + 8].try_into().expect("8")));
            }
            at += 8 * count;
        }
        s
    };
    let (t_var, sum_var) = time_pass(decode_var);
    assert_eq!(sum_arena, sum_var, "{entry}: the advanced layouts decode different numbers");
    report(
        "arena",
        splines.len(),
        arena_bytes,
        t_arena,
        "the shipped record block and arenas; same checksum as the variable records",
    );
    report(
        "variable",
        splines.len(),
        var.len(),
        t_var,
        "one variable-length record per spline; same checksum as the arenas",
    );
    let _ = TAU;
}

// ---------------------------------------------------------------- the shipped index

fn merge(reads: &[(u64, u64)], gap: u64) -> (usize, u64) {
    let mut v: Vec<(u64, u64)> = reads.iter().map(|&(s, n)| (s, s + n)).collect();
    v.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (s, e) in v {
        match out.last_mut() {
            Some(l) if s <= l.1 + gap => l.1 = l.1.max(e),
            _ => out.push((s, e)),
        }
    }
    (out.len(), out.iter().map(|r| r.1 - r.0).sum())
}

/// One recorded viewport on a cold reader over the file; the requests it made, as `(offset, len)`.
fn viewport_reads(file: &Path, window: [f64; 4]) -> Vec<(u64, u64)> {
    let f = fs::File::open(file).expect("open");
    let len = f.metadata().expect("metadata").len();
    let r = ContainerReader::open(Recording::new(ReadAtSource::new(FileAt(f), len)), ReaderConfig::default())
        .expect("opens");
    let dir = SpatialDirectory::open(&r).expect("directory");
    let long = StraddlerIndex::open(&r).expect("straddler index");
    let (ids, chunks): (Vec<BlockId>, Vec<usize>) = match &dir {
        Some(dir) => {
            let win = Aabb::new([window[0], window[1]], [window[2], window[3]]).expect("a window");
            let scale = 1000.0 / (window[2] - window[0]).max(0.000_001);
            let plan = ViewportQuery::new(win, scale)
                .plan(dir, &r, &ViewportConfig { prefetch_margin_px: 0.0 })
                .expect("plan");
            let mut ids = plan.block_ids();
            if let Some(ix) = &long {
                let mut payload: Vec<BlockId> =
                    ix.query(&r, &win).expect("straddlers").iter().map(|h| h.payload.block_id).collect();
                payload.sort();
                payload.dedup();
                ids.extend(payload);
            }
            (ids, plan.advanced_chunks)
        }
        None => (
            r.entries()
                .iter()
                .enumerate()
                .filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot))
                .map(|(i, _)| BlockId(i as u32))
                .collect(),
            (0..r.advanced_chunk_count().expect("chunks")).collect(),
        ),
    };
    let _ = r.read_blocks(&ids).expect("blocks");
    for c in chunks {
        let _ = r.advanced(c).expect("chunk");
    }
    r.source().requests().iter().map(|q| (q.offset, u64::from(q.len))).collect()
}

fn shipped(entry: &str, dir: &Path, platform: &str) {
    let o = Out { entry, platform };
    let file = path(dir, entry);
    let bytes = fs::read(&file).expect("the .pvec");
    let wins = viewports(entry, bounds(&bytes).expect("bounds"));
    let (mut mid_r, mut mid_b, mut mid_n, mut all_r, mut all_b) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, w) in wins.iter().enumerate() {
        let (r, b) = merge(&viewport_reads(&file, *w), 64 * 1024);
        all_r += r as f64;
        all_b += b as f64;
        if i % 4 == 2 {
            mid_r += r as f64;
            mid_b += b as f64;
            mid_n += 1.0;
        }
    }
    let note = "measured on the real reader over a file; reads at most 64 KiB apart merged; the open and the directory included";
    o.row("index", "shipped", "mid", "ranges_mean", Some(mid_r / mid_n), note);
    o.row("index", "shipped", "mid", "bytes_mean", Some(mid_b / mid_n), note);
    o.row("index", "shipped", "all", "ranges_total", Some(all_r), note);
    o.row("index", "shipped", "all", "bytes_total", Some(all_b), note);
    o.row("block", "shipped", "", "bytes_total", Some(all_b), note);
    let r =
        ContainerReader::open(MemorySource::new(bytes.as_slice()), ReaderConfig::default()).expect("opens");
    let sum = |kinds: &[BlockKind]| -> f64 {
        r.entries()
            .iter()
            .filter(|e| e.block_kind().is_some_and(|k| kinds.contains(&k)))
            .map(|e| f64::from(e.stored_length))
            .sum::<f64>()
            + 0.0
    };
    let hot: Vec<&_> = r.entries().iter().filter(|e| e.block_kind() == Some(BlockKind::Hot)).collect();
    let index = sum(&[BlockKind::SpatialDirectory, BlockKind::CellRefs]);
    let dup = sum(&[BlockKind::Straddlers]);
    o.row(
        "index",
        "shipped",
        "",
        "index_bytes",
        Some(index),
        "the spatial directory and the cell reference arrays, as stored",
    );
    o.row(
        "index",
        "shipped",
        "",
        "dup_bytes",
        Some(dup),
        "the straddler lists, the references a long entity leaves in every cell it crosses",
    );
    o.row("index", "shipped", "", "file_bytes", Some(bytes.len() as f64), "");
    o.row(
        "index",
        "shipped",
        "",
        "index_share",
        Some((index + dup) / bytes.len() as f64),
        "index plus duplicated references over the file",
    );
    let mean_block = hot.iter().map(|e| f64::from(e.stored_length)).sum::<f64>() / hot.len().max(1) as f64;
    o.row("block", "shipped", "", "mean_block_bytes", Some(mean_block), &format!("{} hot blocks", hot.len()));
    // The same 200 viewports over the file the writer defaults make: stored bytes, compression included.
    let file = default_path(dir, entry);
    let bytes = fs::read(&file).expect("the default .pvec");
    let (mut mid_r, mut mid_b, mut mid_n, mut all_r, mut all_b) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (i, w) in viewports(entry, bounds(&bytes).expect("bounds")).iter().enumerate() {
        let (r, b) = merge(&viewport_reads(&file, *w), 64 * 1024);
        all_r += r as f64;
        all_b += b as f64;
        if i % 4 == 2 {
            mid_r += r as f64;
            mid_b += b as f64;
            mid_n += 1.0;
        }
    }
    let note = "the file the writer defaults make (compressed); stored bytes of the same 200 viewports, reads at most 64 KiB apart merged";
    o.row("default", "shipped", "mid", "ranges_mean", Some(mid_r / mid_n), note);
    o.row("default", "shipped", "mid", "bytes_mean", Some(mid_b / mid_n), note);
    o.row("default", "shipped", "all", "ranges_total", Some(all_r), note);
    o.row("default", "shipped", "all", "bytes_total", Some(all_b), note);
    o.row("default", "shipped", "", "file_bytes", Some(bytes.len() as f64), note);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    let dir = |i: usize| PathBuf::from(a.get(i).expect("an output directory"));
    match a.first().map(String::as_str) {
        Some("header") => println!("{HEADER}"),
        Some("prepare") => prepare(&a[1], &dir(2)),
        Some("decode") => decode(&a[1], &dir(2), &a[3]),
        Some("shipped") => shipped(&a[1], &dir(2), &a[3]),
        Some("report") => {
            let rows =
                parse_rows(&fs::read_to_string(&a[1]).expect("the CSV")).unwrap_or_else(|e| panic!("{e}"));
            for v in verdicts(&rows) {
                println!("- {} on {}: {} ({})", v.id, v.platform, v.status.label(), v.detail);
            }
        }
        _ => eprintln!("usage: layout prepare|decode|shipped ENTRY OUT [PLATFORM] | header | report CSV"),
    }
}
