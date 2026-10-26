//! Issue P8.2 (pvec#61): PVec against STEP Part 21 and STEP-HDF on the pinned corpus. The thresholds are
//! pinned here before the first run, and a missed one has to appear in the document as a result.

use pvec_bench::step_bench::{
    COLD_OPEN_MIN_SPEEDUP, COMPRESSED_MAX_OVERHEAD, Row, Status, VIEWPORT_MAX_FRACTION_OF_PART21,
    WASM_MAX_KIB, parse_rows, verdicts,
};
use pvec_corpus::json::{self, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

const HEADER: &str = "entry,arm,platform,host,entities,geometry,encoded_bytes,compressed_bytes,cold_open_ms,\
viewport_p50_ms,viewport_p95_ms,viewport_bytes_max,peak_rss_kib,wasm_gz_bytes,note";
const ARMS: [&str; 3] = ["pvec", "part21", "step-hdf"];
const PLATFORMS: [&str; 2] = ["linux-x86_64", "linux-aarch64"];
const IDS: [&str; 6] = [
    "cold-open-10x",
    "viewport-bytes-20pct",
    "compressed-size-110pct",
    "viewport-p95-vs-hdf",
    "wasm-500kib",
    "step-hdf-footprint-reported",
];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc() -> String {
    fs::read_to_string(repo().join("docs/bench/STEP_COMPARISON.md")).expect("docs/bench/STEP_COMPARISON.md")
}

fn csv() -> String {
    fs::read_to_string(repo().join("docs/bench/step_comparison.csv")).expect("docs/bench/step_comparison.csv")
}

fn rows() -> Vec<Row> {
    parse_rows(&csv()).unwrap_or_else(|e| panic!("{e}"))
}

fn eligible_entries() -> Vec<String> {
    let lock =
        fs::read_to_string(repo().join("crates/pvec-bench/benches/corpus/corpus.lock.json")).expect("lock");
    let lock = json::parse(lock.as_bytes()).expect("json");
    lock.get("entries")
        .and_then(Value::as_arr)
        .expect("entries")
        .iter()
        .filter(|e| e.get("baseline_eligible").and_then(Value::as_bool) == Some(true))
        .map(|e| e.get("name").and_then(Value::as_str).expect("name").to_string())
        .collect()
}

fn row(entry: &str, arm: &str, platform: &str) -> Row {
    Row { entry: entry.into(), arm: arm.into(), platform: platform.into(), ..Row::default() }
}

fn status(rows: &[Row], id: &str, platform: &str) -> Option<Status> {
    verdicts(rows).into_iter().find(|v| v.id == id && v.platform == platform).map(|v| v.status)
}

// ---------------------------------------------------------------- the thresholds, declared before the run

#[test]
fn the_thresholds_are_pinned_before_the_run() {
    assert_eq!(COLD_OPEN_MIN_SPEEDUP, 10.0);
    assert_eq!(VIEWPORT_MAX_FRACTION_OF_PART21, 0.20);
    assert_eq!(COMPRESSED_MAX_OVERHEAD, 0.10);
    assert_eq!(WASM_MAX_KIB, 500.0);
}

#[test]
fn a_pass_a_fail_and_a_gap_are_each_a_result() {
    const X: &str = "linux-x86_64";
    let mut pvec = row("architectural-1e5", "pvec", X);
    let mut p21 = row("architectural-1e5", "part21", X);
    pvec.cold_open_ms = Some(10.0);
    p21.cold_open_ms = Some(100.0);
    assert_eq!(
        status(&[pvec.clone(), p21.clone()], "cold-open-10x", X),
        Some(Status::Pass),
        "10x exactly passes"
    );
    p21.cold_open_ms = Some(99.0);
    assert_eq!(status(&[pvec.clone(), p21.clone()], "cold-open-10x", X), Some(Status::Fail), "9.9x fails");
    p21.cold_open_ms = None;
    assert_eq!(status(&[pvec, p21], "cold-open-10x", X), Some(Status::NotMeasured), "a gap is not a pass");
    assert_eq!(status(&[], "cold-open-10x", X), None, "no row, no verdict: the document then lists none");
}

#[test]
fn a_viewport_that_moves_more_than_a_fifth_of_the_part_21_file_fails() {
    const X: &str = "linux-x86_64";
    let mut pvec = row("architectural-1e6", "pvec", X);
    let mut p21 = row("architectural-1e6", "part21", X);
    p21.encoded_bytes = Some(1000.0);
    pvec.viewport_bytes_max = Some(200.0);
    assert_eq!(status(&[pvec.clone(), p21.clone()], "viewport-bytes-20pct", X), Some(Status::Pass));
    pvec.viewport_bytes_max = Some(201.0);
    assert_eq!(status(&[pvec, p21], "viewport-bytes-20pct", X), Some(Status::Fail));
}

#[test]
fn compressed_size_and_viewport_p95_are_held_on_every_entry_not_on_average() {
    const X: &str = "linux-x86_64";
    let mk = |entry: &str, pvec_c: f64, hdf_c: f64, pvec_p95: f64, hdf_p95: f64| -> Vec<Row> {
        let mut a = row(entry, "pvec", X);
        let mut b = row(entry, "step-hdf", X);
        (a.compressed_bytes, b.compressed_bytes) = (Some(pvec_c), Some(hdf_c));
        (a.viewport_p95_ms, b.viewport_p95_ms) = (Some(pvec_p95), Some(hdf_p95));
        vec![a, b]
    };
    let mut all = mk("e1", 110.0, 100.0, 1.0, 2.0);
    all.extend(mk("e2", 50.0, 100.0, 1.0, 2.0));
    assert_eq!(
        status(&all, "compressed-size-110pct", X),
        Some(Status::Pass),
        "10% over is within the threshold"
    );
    assert_eq!(status(&all, "viewport-p95-vs-hdf", X), Some(Status::Pass));
    all.extend(mk("e3", 111.0, 100.0, 2.1, 2.0));
    assert_eq!(status(&all, "compressed-size-110pct", X), Some(Status::Fail), "one entry over fails the lot");
    assert_eq!(status(&all, "viewport-p95-vs-hdf", X), Some(Status::Fail), "one slower entry fails the lot");
    let why = verdicts(&all).into_iter().find(|v| v.id == "viewport-p95-vs-hdf").expect("verdict").detail;
    assert!(why.contains("e3"), "the detail names the entry: {why}");
}

#[test]
fn an_arm64_number_is_never_compared_with_an_x64_number() {
    let mut pvec = row("architectural-1e5", "pvec", "linux-aarch64");
    let mut p21 = row("architectural-1e5", "part21", "linux-x86_64");
    pvec.cold_open_ms = Some(1.0);
    p21.cold_open_ms = Some(1000.0);
    let v = verdicts(&[pvec, p21]);
    assert!(v.iter().any(|v| v.id == "cold-open-10x"), "the threshold still gets a verdict: {v:?}");
    assert!(v.iter().all(|v| v.status != Status::Pass), "{v:?}");
}

// ---------------------------------------------------------------- the committed run

#[test]
fn the_csv_has_the_columns_and_a_row_for_every_arm_on_every_eligible_entry_on_x64() {
    let text = csv();
    assert_eq!(text.lines().next(), Some(HEADER));
    let rows = rows();
    let seen: BTreeSet<(String, String, String)> =
        rows.iter().map(|r| (r.entry.clone(), r.arm.clone(), r.platform.clone())).collect();
    assert_eq!(seen.len(), rows.len(), "no (entry, arm, platform) twice");
    for entry in eligible_entries() {
        for arm in ARMS {
            assert!(
                seen.contains(&(entry.clone(), arm.to_string(), "linux-x86_64".to_string())),
                "no x64 row for {entry} / {arm}"
            );
        }
    }
    for r in &rows {
        assert!(PLATFORMS.contains(&r.platform.as_str()), "{r:?}");
        assert!(ARMS.contains(&r.arm.as_str()), "{r:?}");
    }
}

#[test]
fn every_gap_in_a_row_has_a_note_that_says_why() {
    for r in rows().iter().filter(|r| r.entry != "(artefact)") {
        let gaps = [
            r.encoded_bytes,
            r.compressed_bytes,
            r.cold_open_ms,
            r.viewport_p50_ms,
            r.viewport_p95_ms,
            r.viewport_bytes_max,
            r.peak_rss_kib,
        ]
        .iter()
        .filter(|c| c.is_none())
        .count();
        assert!(
            gaps == 0 || !r.note.trim().is_empty(),
            "{} / {} / {} has {gaps} gaps and no note",
            r.entry,
            r.arm,
            r.platform
        );
    }
}

#[test]
fn the_three_arms_hold_identical_geometry() {
    let mut by_entry: BTreeMap<(String, String), Vec<&Row>> = BTreeMap::new();
    let rows = rows();
    for r in rows.iter().filter(|r| r.entry != "(artefact)") {
        by_entry.entry((r.entry.clone(), r.platform.clone())).or_default().push(r);
    }
    for ((entry, platform), arms) in by_entry {
        let digests: BTreeSet<&str> = arms.iter().map(|r| r.geometry.as_str()).collect();
        assert_eq!(
            digests.len(),
            1,
            "{entry} on {platform}: the arms were built from different shape lists: {digests:?}"
        );
        assert!(digests.iter().all(|d| d.len() == 16), "{entry}: a geometry digest");
        let counts: BTreeSet<u64> = arms.iter().filter_map(|r| r.entities).map(|n| n as u64).collect();
        assert!(counts.len() <= 1, "{entry} on {platform}: entity counts differ: {counts:?}");
    }
}

// ---------------------------------------------------------------- the document

#[test]
fn the_document_declares_its_baselines_before_it_reports_and_states_the_toolchain() {
    let d = doc();
    for needle in [
        "interchange baseline",
        "binary-container baseline",
        "regression baseline",
        "--platform",
        "sha256:",
        "step_comparison.csv",
        "arm64",
        "native x64",
    ] {
        assert!(d.contains(needle), "STEP_COMPARISON.md never says {needle:?}");
    }
    let declared = d.find("## Thresholds").expect("a thresholds section");
    let reported = d.find("## Results").expect("a results section");
    assert!(declared < reported, "the thresholds come before the results");
}

#[test]
fn the_document_records_every_verdict_the_csv_computes_including_the_failures() {
    let d = doc();
    let computed = verdicts(&rows());
    for id in IDS {
        assert!(computed.iter().any(|v| v.id == id), "the CSV yields no verdict for {id}");
    }
    for v in computed {
        let label = match v.status {
            Status::Pass => "PASS",
            Status::Fail => "FAIL",
            Status::NotMeasured => "NOT MEASURED",
        };
        let line = format!("- {} on {}: {label}", v.id, v.platform);
        assert!(d.contains(&line), "the document does not record `{line}`");
    }
    assert!(d.contains("P8.7"), "a missed threshold names P8.7 as the place that decides");
}
