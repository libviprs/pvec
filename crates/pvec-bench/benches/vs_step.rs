//! PVec against STEP Part 21 and STEP-HDF (issue P8.2): the `.pvec` arm, the Part 21 arm, and the assembly of
//! the three arms into `docs/bench/step_comparison.csv`. The STEP-HDF arm runs in `spikes/step-hdf` and the
//! independent Part 21 evaluator in the `tools/step-eval` image; `tools/step-compare.sh` drives all of them
//! and hands their `key=value` files to `assemble`.
//!
//! ```text
//! vs_step prepare  ENTRY OUT              build the files every arm reads, and the sizes
//! vs_step pvec     ENTRY OUT              the .pvec arm: cold open, 200 viewports, bytes, peak memory
//! vs_step part21   ENTRY OUT              the reference parse of the Part 21 text
//! vs_step assemble OUT PLATFORM HOST E..  the CSV rows (no header) of the entries, from the key=value files
//! vs_step header                          the CSV header
//! ```
//!
//! Every measurement starts from a file the previous step wrote, so a peak resident set is the reader's and
//! not the writer's. Peak memory is `VmHWM` and exists on Linux only; elsewhere the cell is empty.

use pvec_bench::dump::{digest, dump};
use pvec_bench::part21;
use pvec_bench::shape::from_artifact;
use pvec_bench::step_bench::{HEADER, Row, parse_rows, percentile, verdicts};
use pvec_bench::{bounds, build_with};
use pvec_container::compress::{Codec, CodecPolicy};
use pvec_container::directory::BlockKind;
use pvec_container::source::{ReadAt, ReadAtSource, Recording, SourceError};
use pvec_container::writer::WriterOptions;
use pvec_container::{BlockId, ContainerReader, ReaderConfig};
use pvec_stream::{Aabb, SpatialDirectory, StraddlerIndex, ViewportConfig, ViewportQuery};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Entities above which the Part 21 and STEP-HDF arms are not attempted: the text alone would be several
/// gigabytes and neither the evaluator nor the HDF writer can hold it.
const OTHER_ARMS_MAX_ENTITIES: usize = 2_000_000;

struct FileAt(fs::File);

impl ReadAt for FileAt {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> Result<usize, SourceError> {
        self.0.read_at(buf, offset).map_err(|e| SourceError::Io(e.to_string()))
    }
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches/corpus")
}

fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find(|l| l.starts_with("VmHWM:"))?.split_whitespace().nth(1)?.parse().ok()
}

type Kv = BTreeMap<String, String>;

fn write_kv(path: &Path, kv: &Kv) {
    let text: String = kv.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    fs::write(path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

fn read_kv(path: &Path) -> Option<Kv> {
    let text = fs::read_to_string(path).ok()?;
    Some(
        text.lines().filter_map(|l| l.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect(),
    )
}

fn num(kv: &Kv, key: &str) -> Option<f64> {
    kv.get(key)?.parse().ok()
}

/// The recorded viewports of one entry, from the checked-in CSV.
fn viewports(entry: &str) -> Vec<[f64; 4]> {
    let text = fs::read_to_string(corpus_dir().join("viewports.csv")).expect("viewports.csv");
    text.lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(',').collect();
            (f.first() == Some(&entry)).then(|| [2, 3, 4, 5].map(|i| f[i].parse().expect("a coordinate")))
        })
        .collect()
}

fn codec_options(codec: Codec) -> WriterOptions {
    WriterOptions { codecs: CodecPolicy::uniform(codec), ..WriterOptions::default() }
}

fn deflate_len(bytes: &[u8]) -> usize {
    miniz_oxide::deflate::compress_to_vec(bytes, 6).len()
}

fn prepare(entry: &str, out: &Path) {
    let mut kv = Kv::new();
    let raw = build_with(entry, WriterOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    fs::write(out.join(format!("{entry}.pvec")), &raw).expect("write .pvec");
    kv.insert("pvec_bytes".into(), raw.len().to_string());
    for (key, codec) in [("pvec_lz4_bytes", Codec::Lz4), ("pvec_deflate_bytes", Codec::Deflate)] {
        let bytes = build_with(entry, codec_options(codec)).unwrap_or_else(|e| panic!("{e}"));
        kv.insert(key.into(), bytes.len().to_string());
    }
    let shapes = from_artifact(&raw).expect("shapes");
    kv.insert("entities".into(), shapes.len().to_string());
    kv.insert("geometry".into(), digest(&shapes));
    if shapes.len() > OTHER_ARMS_MAX_ENTITIES {
        let why = format!(
            "not attempted above {OTHER_ARMS_MAX_ENTITIES} entities: the text and the HDF file would not fit the hosts"
        );
        kv.insert("part21_error".into(), why.clone());
        kv.insert("hdf_error".into(), why);
    } else {
        fs::write(out.join(format!("{entry}.shapes")), dump(&shapes, bounds(&raw).expect("bounds")))
            .expect("write dump");
        match part21::text(&shapes) {
            Ok(text) => {
                kv.insert("part21_bytes".into(), text.len().to_string());
                kv.insert("part21_deflate_bytes".into(), deflate_len(text.as_bytes()).to_string());
                fs::write(out.join(format!("{entry}.stp")), text).expect("write .stp");
            }
            Err(why) => {
                kv.insert("part21_error".into(), why);
            }
        }
    }
    write_kv(&out.join(format!("{entry}.prepare.kv")), &kv);
}

/// One recorded viewport on a cold reader over the file: open, directory, plan, the planned blocks, the
/// straddler payloads the viewport touches, and the advanced set when it is in view. Returns the time from
/// the first byte requested to the viewport drawn, the bytes asked of the file (the open included) and the
/// number of range requests.
fn one_viewport(path: &Path, window: [f64; 4]) -> (f64, u64, usize) {
    let file = fs::File::open(path).expect("open");
    let len = file.metadata().expect("metadata").len();
    let start = Instant::now();
    let r =
        ContainerReader::open(Recording::new(ReadAtSource::new(FileAt(file), len)), ReaderConfig::default())
            .expect("opens");
    let dir = SpatialDirectory::open(&r).expect("directory");
    let long = StraddlerIndex::open(&r).expect("straddler index");
    let (ids, advanced): (Vec<BlockId>, bool) = match &dir {
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
            (ids, plan.advanced_in_view)
        }
        // A file without a spatial directory (the known-truth fixtures) has no index to select with: the
        // viewport reads every hot block and every advanced chunk.
        None => {
            let ids = r
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, e)| e.block_kind() == Some(BlockKind::Hot))
                .map(|(i, _)| BlockId(i as u32))
                .collect();
            (ids, true)
        }
    };
    let blocks = r.read_blocks(&ids).expect("blocks");
    let drawn: usize = blocks.iter().map(|b| b.hot().map_or(0, |h| h.len())).sum();
    let mut sets = 0;
    if advanced {
        for c in 0..r.advanced_chunk_count().expect("chunk count") {
            let set = r.advanced(c).expect("chunk").expect("exists");
            sets += set.view().expect("view").len();
        }
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    assert!(drawn + sets > 0 || dir.is_some(), "a file without a directory draws everything");
    let source = r.source();
    (ms, source.bytes_requested(), source.requests().len())
}

fn measure_pvec(entry: &str, out: &Path) {
    let path = out.join(format!("{entry}.pvec"));
    let mut kv = Kv::new();
    // Cold open to directory ready: median of five fresh opens.
    let mut opens = Vec::new();
    for _ in 0..5 {
        let file = fs::File::open(&path).expect("open");
        let len = file.metadata().expect("metadata").len();
        let t = Instant::now();
        let r = ContainerReader::open(ReadAtSource::new(FileAt(file), len), ReaderConfig::default())
            .expect("opens");
        let _ = SpatialDirectory::open(&r).expect("directory");
        let _ = StraddlerIndex::open(&r).expect("straddler index");
        opens.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    kv.insert("cold_open_ms".into(), percentile(&opens, 0.5).expect("five opens").to_string());
    let (mut times, mut bytes, mut ranges) = (Vec::new(), Vec::new(), Vec::new());
    let views = viewports(entry);
    assert_eq!(views.len(), 200, "{entry}: 200 recorded viewports");
    for w in &views {
        let (ms, b, n) = one_viewport(&path, *w);
        times.push(ms);
        bytes.push(b as f64);
        ranges.push(n as f64);
    }
    let q = |v: &[f64], q| percentile(v, q).expect("200 viewports").to_string();
    kv.insert("viewport_p50_ms".into(), q(&times, 0.5));
    kv.insert("viewport_p95_ms".into(), q(&times, 0.95));
    kv.insert("viewport_bytes_max".into(), q(&bytes, 1.0));
    kv.insert("viewport_bytes_p95".into(), q(&bytes, 0.95));
    kv.insert("viewport_ranges_max".into(), q(&ranges, 1.0));
    if let Some(rss) = peak_rss_kib() {
        kv.insert("peak_rss_kib".into(), rss.to_string());
    }
    write_kv(&out.join(format!("{entry}.pvec.kv")), &kv);
}

fn measure_part21(entry: &str, out: &Path) {
    let mut kv = Kv::new();
    let t = Instant::now();
    let text = fs::read_to_string(out.join(format!("{entry}.stp"))).expect("the Part 21 file");
    kv.insert("read_ms".into(), (t.elapsed().as_secs_f64() * 1000.0).to_string());
    let t = Instant::now();
    let scan = part21::scan(&text).unwrap_or_else(|e| panic!("{entry}: {e}"));
    kv.insert("scan_ms".into(), (t.elapsed().as_secs_f64() * 1000.0).to_string());
    kv.insert("scan_instances".into(), scan.instances.to_string());
    kv.insert("scan_parameters".into(), scan.parameters.to_string());
    kv.insert("scan_checksum".into(), format!("{:016x}", scan.checksum));
    if let Some(rss) = peak_rss_kib() {
        kv.insert("scan_peak_rss_kib".into(), rss.to_string());
    }
    write_kv(&out.join(format!("{entry}.part21.kv")), &kv);
}

fn assemble(out: &Path, platform: &str, host: &str, entries: &[String]) {
    for entry in entries {
        let prep = read_kv(&out.join(format!("{entry}.prepare.kv")))
            .unwrap_or_else(|| panic!("{entry}: not prepared"));
        let base = |arm: &str| Row {
            entry: entry.clone(),
            arm: arm.into(),
            platform: platform.into(),
            host: host.into(),
            entities: num(&prep, "entities"),
            geometry: prep.get("geometry").cloned().unwrap_or_default(),
            ..Row::default()
        };
        // .pvec
        let mut r = base("pvec");
        if let Some(kv) = read_kv(&out.join(format!("{entry}.pvec.kv"))) {
            r.encoded_bytes = num(&prep, "pvec_bytes");
            let (lz4, dfl) = (num(&prep, "pvec_lz4_bytes"), num(&prep, "pvec_deflate_bytes"));
            r.compressed_bytes = lz4.zip(dfl).map(|(a, b)| a.min(b));
            r.cold_open_ms = num(&kv, "cold_open_ms");
            r.viewport_p50_ms = num(&kv, "viewport_p50_ms");
            r.viewport_p95_ms = num(&kv, "viewport_p95_ms");
            r.viewport_bytes_max = num(&kv, "viewport_bytes_max");
            r.peak_rss_kib = num(&kv, "peak_rss_kib");
            r.note = format!(
                "compressed is the smaller of lz4 ({}) and deflate ({}) per block; timings on the uncompressed default file; viewport bytes include the open; at most {} ranges",
                lz4.unwrap_or(0.0),
                dfl.unwrap_or(0.0),
                num(&kv, "viewport_ranges_max").unwrap_or(0.0)
            );
        } else {
            r.note = "not measured".into();
        }
        println!("{}", r.line());
        // Part 21
        let mut r = base("part21");
        let occt = read_kv(&out.join(format!("{entry}.occt.kv")));
        if let Some(why) = prep.get("part21_error") {
            r.note = format!("no Part 21 file: {why}");
        } else {
            r.encoded_bytes = num(&prep, "part21_bytes");
            r.compressed_bytes = num(&prep, "part21_deflate_bytes");
            let scan = read_kv(&out.join(format!("{entry}.part21.kv")));
            let mut note = String::from("compressed is the whole file deflated at level 6");
            if let Some(o) = &occt {
                r.cold_open_ms = num(o, "read_ms");
                r.peak_rss_kib = num(o, "peak_rss_kib");
                note += &format!(
                    "; cold open is the OCCT read of the independent evaluator (the 2D to 3D lift of {} ms is not counted); edges {}",
                    o.get("lift_ms").map_or("?", |s| s),
                    o.get("edges").map_or("?", |s| s)
                );
            } else {
                note +=
                    "; the independent evaluator was not run on this entry, so there is no cold-open time";
            }
            if let Some(s) = &scan {
                note += &format!(
                    "; reference parse {} ms for {} instances at {} KiB",
                    s.get("scan_ms").map_or("?", |v| v),
                    s.get("scan_instances").map_or("?", |v| v),
                    s.get("scan_peak_rss_kib").map_or("?", |v| v)
                );
            }
            // A Part 21 file has no range story: every viewport is the whole parse and the whole file.
            r.viewport_p50_ms = r.cold_open_ms;
            r.viewport_p95_ms = r.cold_open_ms;
            r.viewport_bytes_max = r.encoded_bytes;
            note += "; no random access, so each viewport is the full read of the whole file";
            r.note = note;
        }
        println!("{}", r.line());
        // STEP-HDF
        let mut r = base("step-hdf");
        match (prep.get("hdf_error"), read_kv(&out.join(format!("{entry}.hdf.kv")))) {
            (Some(why), _) => r.note = format!("no STEP-HDF file: {why}"),
            (None, Some(kv)) => {
                r.encoded_bytes = num(&kv, "contiguous_bytes");
                r.compressed_bytes = num(&kv, "compressed_bytes");
                r.cold_open_ms = num(&kv, "cold_open_ms");
                r.viewport_p50_ms = num(&kv, "viewport_p50_ms");
                r.viewport_p95_ms = num(&kv, "viewport_p95_ms");
                r.viewport_bytes_max = num(&kv, "viewport_bytes_max");
                r.peak_rss_kib = num(&kv, "peak_rss_kib");
                r.note = kv.get("note").cloned().unwrap_or_default();
            }
            (None, None) => r.note = "not measured".into(),
        }
        println!("{}", r.line());
    }
}

/// The `(artefact)` rows: the decode artefact of each arm and what its decode path costs in lines.
fn artefacts(out: &Path, platform: &str, host: &str) {
    let Some(kv) = read_kv(&out.join("artefact.kv")) else { return };
    let lines = |key: &str| kv.get(key).map_or("?", |v| v);
    let row = |arm: &str, gz: Option<f64>, note: String| {
        let r = Row {
            entry: "(artefact)".into(),
            arm: arm.into(),
            platform: platform.into(),
            host: host.into(),
            wasm_gz_bytes: gz,
            note,
            ..Row::default()
        };
        println!("{}", r.line());
    };
    row(
        "pvec",
        num(&kv, "pvec_wasm_gz_bytes"),
        format!(
            "stand-in for pvec-wasm (P7.3 is not built): the container reader, spatial directory, viewport plan, LZ4 and deflate behind two exports, wasm32-unknown-unknown, opt-level s, LTO, stripped, gzip -9; {} bytes before gzip; decode_lines={} (non-blank non-comment lines of the decode path files, writer code in the same files included); run under Node 22 on the whole drawing: {}",
            lines("pvec_wasm_bytes"),
            lines("pvec_decode_lines"),
            kv.get("pvec_wasm_run").map_or("not run on this host", |v| v)
        ),
    );
    row(
        "part21",
        None,
        format!(
            "no browser decode of Part 21 exists in the repository and the independent evaluator is OCCT in a container; decode_lines={} is the reference parse only (instances to values, no geometry)",
            lines("part21_reference_parse_lines")
        ),
    );
    row(
        "step-hdf",
        num(&kv, "hdf_wasm_gz_bytes"),
        format!(
            "browser footprint of the only reader that builds for wasm32: the pure-Rust read-only hdf5-reader 0.9 with a decode harness, same flags, {} bytes before gzip, newest-format files only; libhdf5 itself does not build for wasm32-unknown-unknown (phase 1, docs/spikes/01-4-step-hdf.md section 5); decode_lines={} for the H2 reader plus {} for the wasm harness",
            lines("hdf_wasm_bytes"),
            lines("hdf_reader_lines"),
            lines("hdf_wasm_harness_lines")
        ),
    );
}

/// The verdict lines the document records, and a table of the main numbers per entry, in Markdown.
fn report(csv: &Path, platform: &str) {
    let rows = parse_rows(&fs::read_to_string(csv).expect("the CSV")).unwrap_or_else(|e| panic!("{e}"));
    for v in verdicts(&rows) {
        println!("- {} on {}: {} ({})", v.id, v.platform, v.status.label(), v.detail);
    }
    println!();
    println!(
        "| entry | entities | .pvec bytes | Part 21 bytes | HDF bytes | .pvec cmp | Part 21 cmp | HDF cmp | open pvec / P21 / HDF ms | p95 pvec / HDF ms | max viewport bytes pvec / HDF |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|");
    let f = |v: Option<f64>| {
        v.map_or("NA".to_string(), |x| {
            if x >= 100.0 {
                format!("{x:.0}")
            } else if x >= 1.0 {
                format!("{x:.2}")
            } else {
                format!("{x:.4}")
            }
        })
    };
    let mut entries: Vec<&str> = rows
        .iter()
        .filter(|r| r.platform == platform && r.entry != "(artefact)")
        .map(|r| r.entry.as_str())
        .collect();
    entries.dedup();
    for e in entries {
        let g = |arm: &str| rows.iter().find(|r| r.entry == e && r.arm == arm && r.platform == platform);
        let (Some(p), Some(s), Some(h)) = (g("pvec"), g("part21"), g("step-hdf")) else { continue };
        println!(
            "| {e} | {} | {} | {} | {} | {} | {} | {} | {} / {} / {} | {} / {} | {} / {} |",
            p.entities.map_or("NA".to_string(), |n| format!("{n:.0}")),
            f(p.encoded_bytes),
            f(s.encoded_bytes),
            f(h.encoded_bytes),
            f(p.compressed_bytes),
            f(s.compressed_bytes),
            f(h.compressed_bytes),
            f(p.cold_open_ms),
            f(s.cold_open_ms),
            f(h.cold_open_ms),
            f(p.viewport_p95_ms),
            f(h.viewport_p95_ms),
            f(p.viewport_bytes_max),
            f(h.viewport_bytes_max)
        );
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    let out = |i: usize| PathBuf::from(a.get(i).expect("an output directory"));
    match a.first().map(String::as_str) {
        Some("prepare") => prepare(&a[1], &out(2)),
        Some("pvec") => measure_pvec(&a[1], &out(2)),
        Some("part21") => measure_part21(&a[1], &out(2)),
        Some("assemble") => {
            assemble(&out(1), &a[2], &a[3], &a[4..]);
            artefacts(&out(1), &a[2], &a[3]);
        }
        Some("header") => println!("{HEADER}"),
        Some("report") => report(&out(1), a.get(2).map_or("linux-x86_64", |p| p)),
        _ => eprintln!(
            "usage: vs_step prepare|pvec|part21 ENTRY OUT | assemble OUT PLATFORM HOST ENTRY.. | header"
        ),
    }
}
