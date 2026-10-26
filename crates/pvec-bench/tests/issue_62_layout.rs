//! Issue P8.3 (pvec#62): layouts, index designs and compression, measured on the pinned corpus against the
//! alternatives they beat. The thresholds are pinned here before the first run, and a missed one has to be in the
//! document as a result.

use pvec_bench::layout_bench::{
    BLOCK_MAX_OFF_BEST, INDEX_MAX_BYTES_FACTOR, INDEX_MAX_FILE_SHARE, LINK_MBIT_PER_S, Row, SOA_MIN_SPEEDUP,
    Status, parse_rows, verdicts,
};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const HEADER: &str = "table,entry,platform,arm,param,metric,value,note";
const X: &str = "linux-x86_64";
const IDS: [&str; 5] = [
    "soa-3x-record",
    "index-vs-best-alternative",
    "index-share-15pct",
    "block-size-within-10pct",
    "compression-crossover-25mbit",
];
const MANDATORY: [&str; 2] = ["spline-heavy-1e5", "huge-coordinate-1e5"];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc() -> String {
    fs::read_to_string(repo().join("docs/bench/LAYOUT.md")).expect("docs/bench/LAYOUT.md")
}

fn csv() -> String {
    fs::read_to_string(repo().join("docs/bench/layout.csv")).expect("docs/bench/layout.csv")
}

fn rows() -> Vec<Row> {
    parse_rows(&csv()).unwrap_or_else(|e| panic!("{e}"))
}

fn r(table: &str, entry: &str, arm: &str, param: &str, metric: &str, value: f64) -> Row {
    Row {
        table: table.into(),
        entry: entry.into(),
        platform: X.into(),
        arm: arm.into(),
        param: param.into(),
        metric: metric.into(),
        value: Some(value),
        note: String::new(),
    }
}

fn status(rows: &[Row], id: &str) -> Option<Status> {
    verdicts(rows).into_iter().find(|v| v.id == id && v.platform == X).map(|v| v.status)
}

// ---------------------------------------------------------------- the thresholds, declared before the run

#[test]
fn the_thresholds_are_pinned_before_the_run() {
    assert_eq!(SOA_MIN_SPEEDUP, 3.0);
    assert_eq!(INDEX_MAX_BYTES_FACTOR, 1.25);
    assert_eq!(INDEX_MAX_FILE_SHARE, 0.15);
    assert_eq!(BLOCK_MAX_OFF_BEST, 0.10);
    assert_eq!(LINK_MBIT_PER_S, 25.0);
}

#[test]
fn the_soa_path_must_decode_three_times_as_fast_as_per_entity_records_on_the_architectural_million() {
    let e = "architectural-1e6";
    let mk = |soa: f64| {
        vec![
            r("decode", e, "soa", "", "entities_per_s", soa),
            r("decode", e, "record", "", "entities_per_s", 100.0),
        ]
    };
    assert_eq!(status(&mk(300.0), "soa-3x-record"), Some(Status::Pass), "3x exactly passes");
    assert_eq!(status(&mk(299.0), "soa-3x-record"), Some(Status::Fail), "2.99x fails");
    let gap = vec![r("decode", e, "soa", "", "entities_per_s", 300.0)];
    assert_eq!(status(&gap, "soa-3x-record"), Some(Status::NotMeasured), "a missing arm is not a pass");
}

#[test]
fn the_shipped_index_is_judged_on_ranges_and_bytes_against_the_best_alternative_for_the_mid_zoom_viewports() {
    let e = "architectural-1e6";
    let mk = |sr: f64, sb: f64| {
        vec![
            r("index", e, "shipped", "mid", "ranges_mean", sr),
            r("index", e, "shipped", "mid", "bytes_mean", sb),
            r("index", e, "quadtree", "mid", "ranges_mean", 9.0),
            r("index", e, "quadtree", "mid", "bytes_mean", 1000.0),
            r("index", e, "rtree-64-32", "mid", "ranges_mean", 5.0),
            r("index", e, "rtree-64-32", "mid", "bytes_mean", 2000.0),
        ]
    };
    // the best alternative is 5 ranges and 1000 bytes, each taken from whichever arm is best at it
    assert_eq!(status(&mk(5.0, 1250.0), "index-vs-best-alternative"), Some(Status::Pass));
    assert_eq!(status(&mk(5.1, 1000.0), "index-vs-best-alternative"), Some(Status::Fail), "one range more");
    assert_eq!(
        status(&mk(4.0, 1251.0), "index-vs-best-alternative"),
        Some(Status::Fail),
        "over 1.25x the bytes"
    );
}

#[test]
fn index_plus_duplicated_references_must_stay_within_fifteen_percent_of_the_file_on_every_entry() {
    let mk = |e: &str, idx: f64, dup: f64| {
        vec![
            r("index", e, "shipped", "", "index_bytes", idx),
            r("index", e, "shipped", "", "dup_bytes", dup),
            r("index", e, "shipped", "", "file_bytes", 1000.0),
        ]
    };
    let mut all = mk("a", 100.0, 50.0);
    assert_eq!(status(&all, "index-share-15pct"), Some(Status::Pass), "exactly 15% passes");
    all.extend(mk("b", 100.0, 51.0));
    assert_eq!(status(&all, "index-share-15pct"), Some(Status::Fail), "one entry over fails the lot");
    let why = verdicts(&all).into_iter().find(|v| v.id == "index-share-15pct").expect("verdict").detail;
    assert!(why.contains('b'), "the detail names the entry: {why}");
}

#[test]
fn the_chosen_block_size_is_held_to_ten_percent_of_the_best_arm_at_both_scales() {
    let mk = |e: &str, shipped: f64| {
        vec![
            r("block", e, "shipped", "", "bytes_total", shipped),
            r("block", e, "rtree-16", "", "bytes_total", 1000.0),
            r("block", e, "rtree-256", "", "bytes_total", 1500.0),
        ]
    };
    let mut all = mk("architectural-1e5", 1100.0);
    all.extend(mk("architectural-1e6", 1100.0));
    assert_eq!(status(&all, "block-size-within-10pct"), Some(Status::Pass));
    let mut worse = mk("architectural-1e5", 1100.0);
    worse.extend(mk("architectural-1e6", 1101.0));
    assert_eq!(status(&worse, "block-size-within-10pct"), Some(Status::Fail), "10.1% off the best at 10^6");
}

#[test]
fn compression_is_judged_at_the_crossover_with_a_25_mbit_link_and_the_crossover_is_computed() {
    // LZ4 saves (1 - ratio) of the bytes and decodes at `d` MiB/s: it wins on any link slower than
    // (1 - ratio) * d MiB/s, which is (1 - ratio) * d * 8.388608 Mbit/s.
    let mk = |ratio: f64, d: f64| {
        vec![
            r("compression", "architectural-1e6", "lz4", "block", "ratio", ratio),
            r("compression", "architectural-1e6", "lz4", "block", "decompress_mib_s", d),
            r("compression", "architectural-1e6", "lz4", "file", "ratio", ratio),
            r("compression", "architectural-1e6", "lz4", "file", "decompress_mib_s", d),
        ]
    };
    assert_eq!(status(&mk(0.5, 1000.0), "compression-crossover-25mbit"), Some(Status::Pass));
    assert_eq!(
        status(&mk(0.5, 5.0), "compression-crossover-25mbit"),
        Some(Status::Fail),
        "crossover 21 Mbit/s"
    );
    let why = verdicts(&mk(0.5, 5.0))
        .into_iter()
        .find(|v| v.id == "compression-crossover-25mbit")
        .expect("verdict")
        .detail;
    assert!(why.contains("Mbit/s"), "the crossover is printed: {why}");
}

// ---------------------------------------------------------------- the committed run

#[test]
fn the_csv_has_the_columns_and_the_mandatory_entries_in_every_table() {
    assert_eq!(csv().lines().next(), Some(HEADER));
    let rows = rows();
    for entry in MANDATORY {
        for table in ["decode", "index", "compression"] {
            assert!(
                rows.iter()
                    .any(|r| r.table == table && r.entry == entry && r.platform == X && r.value.is_some()),
                "no {table} number for {entry} on x64"
            );
        }
    }
    let arms: BTreeSet<(&str, &str)> = rows.iter().map(|r| (r.table.as_str(), r.arm.as_str())).collect();
    for want in [
        ("decode", "soa"),
        ("decode", "record"),
        ("decode", "aos"),
        ("decode", "arena"),
        ("decode", "variable"),
        ("index", "shipped"),
        ("index", "quadtree"),
        ("index", "rtree-512-64"),
        ("index", "rtree-64-32"),
        ("index", "cells-ref12"),
        ("index", "cells-ref28"),
        ("index", "buckets-7"),
        ("index", "buckets-8"),
        ("compression", "none"),
        ("compression", "lz4"),
        ("compression", "zstd3"),
        ("compression", "zstd9"),
        ("compression", "zstd19"),
    ] {
        assert!(arms.contains(&want), "no row for {want:?}");
    }
    for size in ["16", "64", "256", "1024"] {
        assert!(
            rows.iter().any(|r| r.table == "block" && r.arm == format!("rtree-{size}")),
            "no block arm of {size} KiB"
        );
    }
}

#[test]
fn an_arm_that_could_not_run_is_a_row_with_no_value_and_a_note_never_an_interpolation() {
    for r in rows().iter().filter(|r| r.value.is_none()) {
        assert!(
            !r.note.trim().is_empty(),
            "{} / {} / {} / {} has no value and no note",
            r.table,
            r.entry,
            r.arm,
            r.metric
        );
    }
}

#[test]
fn every_row_names_its_platform_and_the_corpus_digest_is_in_the_document() {
    assert!(rows().iter().all(|r| ["linux-x86_64", "linux-aarch64"].contains(&r.platform.as_str())));
    let lock =
        fs::read_to_string(repo().join("crates/pvec-bench/benches/corpus/corpus.lock.json")).expect("lock");
    let digest = pvec_corpus::sha256::hex_digest(lock.as_bytes());
    assert!(doc().contains(&digest), "LAYOUT.md does not name the corpus.lock.json digest {digest}");
}

#[test]
fn the_document_records_every_verdict_including_failures_and_states_finding_4_as_viewport_io() {
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
    assert!(d.contains("total viewport I/O"), "finding 4 is stated as total viewport I/O");
    assert!(d.contains("not a duplication score"), "and explicitly not as a duplication score");
    for needle in ["arm64", "native x64", "P8.7", "perf"] {
        assert!(d.contains(needle), "LAYOUT.md never says {needle:?}");
    }
}
