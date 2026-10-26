//! Issue P8.5 (pvec#64): progressive coverage, measured as latency to coverage. The thresholds and the network
//! model are pinned here before the first run, and a missed threshold has to be in the document as a result.
#![allow(missing_docs)]

use pvec_bench::layout_bench::{Row, Status, parse_rows};
use pvec_bench::progressive_bench::{
    ARMS, Arm, DIRECTORY_MAX_S, IDS, PAN_MAX_DISCARD, PREVIEW_BUDGET_PX, PREVIEW_MAX_PX, RANGE_CONCURRENCY,
    REFINE_MAX_S, REPEAT_MAX_SPREAD, SCREEN_PX, TIER0_MAX_S, TIER0_SLOW_MAX_S, completions, coverage, spread,
    verdicts,
};
use std::fs;
use std::path::PathBuf;

const HEADER: &str = "table,entry,platform,arm,param,metric,value,note";
const X: &str = "linux-x86_64";
const E: &str = "architectural-1e6";
const FAST: &str = "25mbit-60ms";
const SLOW: &str = "5mbit-200ms";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc() -> String {
    fs::read_to_string(repo().join("docs/bench/PROGRESSIVE.md")).expect("docs/bench/PROGRESSIVE.md")
}

fn csv() -> String {
    fs::read_to_string(repo().join("docs/bench/progressive.csv")).expect("docs/bench/progressive.csv")
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

fn stage(entry: &str, arm: &str, param: &str, metric: &str, value: f64) -> Row {
    r("stage", entry, arm, param, metric, value)
}

fn status(rows: &[Row], id: &str) -> Option<Status> {
    verdicts(rows).into_iter().find(|v| v.id == id && v.platform == X).map(|v| v.status)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

// ---------------------------------------------------------------- the thresholds, declared before the run

#[test]
fn the_thresholds_are_pinned_before_the_run() {
    assert_eq!(DIRECTORY_MAX_S, 0.3);
    assert_eq!(TIER0_MAX_S, 1.5);
    assert_eq!(TIER0_SLOW_MAX_S, 4.0);
    assert_eq!(PREVIEW_MAX_PX, 2.0);
    assert_eq!(REFINE_MAX_S, 5.0);
    assert_eq!(PAN_MAX_DISCARD, 0.2);
    assert_eq!(REPEAT_MAX_SPREAD, 0.2);
    assert_eq!(RANGE_CONCURRENCY, 6);
    assert_eq!(SCREEN_PX, 1920.0);
    assert_eq!(PREVIEW_BUDGET_PX, 0.5);
}

#[test]
fn there_are_three_network_arms_as_the_issue_names_them() {
    let names: Vec<&str> = ARMS.iter().map(|a| a.name).collect();
    assert_eq!(names, ["local", FAST, SLOW]);
    assert_eq!(ARMS[0].mbit_per_s, None);
    assert_eq!((ARMS[1].mbit_per_s, ARMS[1].rtt_ms), (Some(25.0), 60.0));
    assert_eq!((ARMS[2].mbit_per_s, ARMS[2].rtt_ms), (Some(5.0), 200.0));
}

// ---------------------------------------------------------------- the network model

#[test]
fn one_request_costs_a_round_trip_and_its_bytes_over_the_link() {
    // 25 Mbit/s is 3,125,000 bytes/s: 312,500 bytes take 0.1 s, after 60 ms for the first byte.
    let t = completions(&ARMS[1], 0.0, &[312_500]);
    assert!(near(t[0], 0.16), "{t:?}");
    // 5 Mbit/s is 625,000 bytes/s, and the round trip is 200 ms.
    let t = completions(&ARMS[2], 1.0, &[625_000]);
    assert!(near(t[0], 2.2), "{t:?}");
}

#[test]
fn requests_overlap_their_round_trips_up_to_the_concurrency_limit() {
    let t = completions(&ARMS[1], 0.0, &[0; 7]);
    assert!(t[..RANGE_CONCURRENCY].iter().all(|t| near(*t, 0.06)), "six at once: {t:?}");
    assert!(near(t[RANGE_CONCURRENCY], 0.12), "the seventh waits for a lane: {t:?}");
}

#[test]
fn requests_share_the_one_link_and_do_not_get_more_bandwidth_by_overlapping() {
    // two requests of one second of link each: the bytes of the second move after the first's.
    let t = completions(&ARMS[1], 0.0, &[3_125_000, 3_125_000]);
    assert!(near(t[0], 1.06), "{t:?}");
    assert!(near(t[1], 2.06), "{t:?}");
}

#[test]
fn a_local_file_has_no_network_time() {
    assert_eq!(completions(&ARMS[0], 0.25, &[1 << 20, 5]), vec![0.25, 0.25]);
    assert!(completions(&Arm { name: "x", mbit_per_s: Some(1.0), rtt_ms: 0.0 }, 0.0, &[]).is_empty());
}

// ---------------------------------------------------------------- the metrics

#[test]
fn coverage_is_represented_over_intersecting_and_an_empty_viewport_is_complete() {
    assert_eq!(coverage(10, 10), 1.0);
    assert_eq!(coverage(10, 9), 0.9);
    assert_eq!(coverage(0, 0), 1.0);
}

#[test]
fn the_repeat_spread_is_the_range_over_the_median() {
    assert!(near(spread(&[1.0, 1.1, 1.2]), 0.2 / 1.1));
    assert_eq!(spread(&[2.0, 2.0, 2.0]), 0.0);
}

// ---------------------------------------------------------------- the verdicts

#[test]
fn directory_ready_is_judged_at_300_ms_on_the_architectural_million() {
    let mk = |s: f64| vec![stage(E, FAST, "p95", "directory_s", s)];
    assert_eq!(status(&mk(0.3), "directory-300ms"), Some(Status::Pass), "300 ms exactly passes");
    assert_eq!(status(&mk(0.301), "directory-300ms"), Some(Status::Fail));
    assert_eq!(status(&[], "directory-300ms"), None, "no rows, no platform");
    let other = vec![stage("mep-1e5", FAST, "p95", "directory_s", 0.1)];
    assert_eq!(status(&other, "directory-300ms"), Some(Status::NotMeasured), "another entry is not a pass");
}

#[test]
fn complete_tier_0_is_judged_at_1_5_s_on_25_mbit_and_4_s_on_5_mbit() {
    let mk = |fast: f64, slow: f64| {
        vec![stage(E, FAST, "p95", "tier0_s", fast), stage(E, SLOW, "p95", "tier0_s", slow)]
    };
    assert_eq!(status(&mk(1.5, 4.0), "tier0-1500ms"), Some(Status::Pass));
    assert_eq!(status(&mk(1.5, 4.0), "tier0-4s-5mbit"), Some(Status::Pass));
    assert_eq!(status(&mk(1.6, 3.0), "tier0-1500ms"), Some(Status::Fail));
    assert_eq!(status(&mk(1.6, 3.0), "tier0-4s-5mbit"), Some(Status::Pass), "the arms are judged apart");
    assert_eq!(status(&mk(1.0, 4.1), "tier0-4s-5mbit"), Some(Status::Fail));
}

#[test]
fn a_missing_entity_is_a_failure_on_any_entry_and_any_arm_not_a_slow_result() {
    let mk = |a: f64, b: f64| {
        vec![
            stage("mep-1e5", FAST, "min", "coverage", a),
            stage("spline-heavy-1e6", SLOW, "min", "coverage", b),
        ]
    };
    assert_eq!(status(&mk(1.0, 1.0), "coverage-complete"), Some(Status::Pass));
    assert_eq!(status(&mk(1.0, 0.999_999), "coverage-complete"), Some(Status::Fail));
    let why =
        verdicts(&mk(1.0, 0.5)).into_iter().find(|v| v.id == "coverage-complete").expect("verdict").detail;
    assert!(why.contains("spline-heavy-1e6"), "the detail names the entry: {why}");
}

#[test]
fn the_worst_preview_on_screen_is_held_to_2_px_and_every_preview_to_a_stable_identity() {
    let mk = |px: f64, unnamed: f64| {
        vec![
            stage(E, FAST, "max", "worst_preview_px", px),
            stage(E, FAST, "total", "previews_without_identity", unnamed),
        ]
    };
    assert_eq!(status(&mk(2.0, 0.0), "preview-2px"), Some(Status::Pass));
    assert_eq!(status(&mk(2.01, 0.0), "preview-2px"), Some(Status::Fail));
    assert_eq!(status(&mk(1.0, 0.0), "preview-identity"), Some(Status::Pass));
    assert_eq!(status(&mk(1.0, 1.0), "preview-identity"), Some(Status::Fail), "one anonymous preview fails");
}

#[test]
fn full_visible_refinement_is_judged_at_5_s() {
    let mk = |s: f64| vec![stage(E, FAST, "p95", "refine_full_s", s)];
    assert_eq!(status(&mk(5.0), "refine-5s"), Some(Status::Pass));
    assert_eq!(status(&mk(5.1), "refine-5s"), Some(Status::Fail));
}

#[test]
fn a_mid_load_pan_may_discard_at_most_a_fifth_of_the_decoded_bytes() {
    let mk = |s: f64| vec![r("interaction", E, FAST, "pan", "discard_share", s)];
    assert_eq!(status(&mk(0.2), "pan-discard-20pct"), Some(Status::Pass));
    assert_eq!(status(&mk(0.21), "pan-discard-20pct"), Some(Status::Fail));
}

#[test]
fn stage_times_may_vary_by_at_most_a_fifth_across_three_repeat_runs() {
    let mk = |runs: [f64; 3]| {
        runs.iter()
            .enumerate()
            .map(|(i, v)| r("repeat", E, FAST, &format!("run{}", i + 1), "tier0_s", *v))
            .collect::<Vec<_>>()
    };
    assert_eq!(status(&mk([1.0, 1.1, 1.2]), "repeat-spread-20pct"), Some(Status::Pass));
    assert_eq!(status(&mk([1.0, 1.1, 1.3]), "repeat-spread-20pct"), Some(Status::Fail));
    assert_eq!(
        status(&mk([1.0, 1.0, 1.0])[..2], "repeat-spread-20pct"),
        Some(Status::NotMeasured),
        "two runs are not three"
    );
}

// ---------------------------------------------------------------- the committed run

#[test]
fn the_csv_has_the_columns_and_every_mandatory_entry_has_all_four_stages_on_every_arm() {
    assert_eq!(csv().lines().next(), Some(HEADER));
    let rows = rows();
    for entry in [E, "spline-heavy-1e6", "spline-heavy-1e5", "huge-coordinate-1e5"] {
        for arm in ARMS {
            for metric in
                ["directory_s", "tier0_s", "refine_first_s", "refine_full_s", "coverage", "preview_fraction"]
            {
                assert!(
                    rows.iter().any(|r| r.table == "stage"
                        && r.entry == entry
                        && r.platform == X
                        && r.arm == arm.name
                        && r.metric == metric
                        && r.value.is_some()),
                    "no {metric} for {entry} on {} on x64",
                    arm.name
                );
            }
        }
    }
    for metric in ["ranges", "bytes"] {
        for st in ["directory", "tier0", "refine"] {
            let m = format!("{metric}_{st}");
            assert!(rows.iter().any(|r| r.metric == m), "no {m} row: ranges and bytes per stage");
        }
    }
    for param in ["pan", "zoom"] {
        assert!(rows.iter().any(|r| r.table == "interaction" && r.param == param), "no {param} row");
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
    assert!(doc().contains(&digest), "PROGRESSIVE.md does not name the corpus.lock.json digest {digest}");
}

#[test]
fn the_document_records_every_verdict_including_failures() {
    let d = doc();
    let computed = verdicts(&rows());
    for id in IDS {
        assert!(computed.iter().any(|v| v.id == id), "the CSV yields no verdict for {id}");
    }
    for v in computed {
        let line = format!("- {} on {}: {}", v.id, v.platform, v.status.label());
        assert!(d.contains(&line), "the document does not record `{line}`");
    }
}

#[test]
fn the_document_labels_every_preview_as_display_fidelity_and_never_as_geometric_exactness() {
    let d = doc();
    assert!(d.contains("display fidelity"));
    assert!(d.contains("not geometric exactness"));
    for needle in
        ["arm64", "native x64", "coverage = entities represented in viewport", "modelled", "P8.6", "P8.7"]
    {
        assert!(d.contains(needle), "PROGRESSIVE.md never says {needle:?}");
    }
    for banned in ["exact preview", "geometrically exact preview", "lossless preview"] {
        assert!(!d.to_lowercase().contains(banned), "PROGRESSIVE.md calls a preview {banned:?}");
    }
}
