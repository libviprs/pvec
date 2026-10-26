//! Issue P8.4 (pvec#63): exact evaluation and adaptive tessellation by curve type, transform and error budget. The
//! thresholds are pinned here before the first run, and a missed one has to be in the document as a result.

use pvec_bench::geometry_bench::{
    BSPLINE3_MAX_NS, HALVING_MAX_GROWTH, MAX_ALLOCS_PER_TESSELLATION, OFFSET_MAX_COST, Row, Status,
    parse_rows, verdicts,
};
use std::fs;
use std::path::PathBuf;

const HEADER: &str = "table,entry,platform,curve,transform,budget_px,metric,value,note";
const X: &str = "linux-x86_64";
const IDS: [&str; 5] = [
    "bspline3-point-at-150ns",
    "tessellate-one-allocation",
    "conic-halving-1.5x",
    "offset-1.2x-and-accuracy",
    "error-budget-never-exceeded",
];
const CURVES: [&str; 10] = [
    "line",
    "polyline",
    "circle",
    "ellipse",
    "trimmed",
    "composite",
    "bezier",
    "bspline3",
    "bspline7",
    "nurbs",
];
const TRANSFORMS: [&str; 5] = ["identity", "translation", "uniform-scale", "rotation-scale", "chain-offset"];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn doc() -> String {
    fs::read_to_string(repo().join("docs/bench/GEOMETRY.md")).expect("docs/bench/GEOMETRY.md")
}

fn csv() -> String {
    fs::read_to_string(repo().join("docs/bench/geometry.csv")).expect("docs/bench/geometry.csv")
}

fn rows() -> Vec<Row> {
    parse_rows(&csv()).unwrap_or_else(|e| panic!("{e}"))
}

fn r(table: &str, curve: &str, transform: &str, budget: Option<f64>, metric: &str, value: f64) -> Row {
    Row {
        table: table.into(),
        entry: "synthetic".into(),
        platform: X.into(),
        curve: curve.into(),
        transform: transform.into(),
        budget_px: budget,
        metric: metric.into(),
        value: Some(value),
        note: String::new(),
    }
}

fn status(rows: &[Row], id: &str) -> Option<Status> {
    verdicts(rows).into_iter().find(|v| v.id == id && v.platform == X).map(|v| v.status)
}

#[test]
fn the_thresholds_are_pinned_before_the_run() {
    assert_eq!(BSPLINE3_MAX_NS, 150.0);
    assert_eq!(HALVING_MAX_GROWTH, 1.5);
    assert_eq!(OFFSET_MAX_COST, 1.2);
    assert_eq!(MAX_ALLOCS_PER_TESSELLATION, 1.0);
}

#[test]
fn point_at_on_a_cubic_must_be_within_150_ns_and_not_slower_than_the_p1_3_prototype() {
    let mk = |ns: f64, proto: f64| {
        vec![
            r("eval", "bspline3", "identity", None, "point_at_ns", ns),
            r("eval", "bspline3", "identity", None, "prototype_point_at_ns", proto),
        ]
    };
    assert_eq!(
        status(&mk(150.0, 150.0), "bspline3-point-at-150ns"),
        Some(Status::Pass),
        "150 ns, tied with the prototype"
    );
    assert_eq!(status(&mk(150.1, 200.0), "bspline3-point-at-150ns"), Some(Status::Fail), "over 150 ns");
    assert_eq!(
        status(&mk(100.0, 99.0), "bspline3-point-at-150ns"),
        Some(Status::Fail),
        "slower than the prototype at all"
    );
    let gap = vec![r("eval", "bspline3", "identity", None, "point_at_ns", 100.0)];
    assert_eq!(
        status(&gap, "bspline3-point-at-150ns"),
        Some(Status::NotMeasured),
        "no baseline is not a pass"
    );
}

#[test]
fn tessellate_may_allocate_once_per_curve_at_any_budget() {
    let mk = |worst: f64| {
        vec![
            r("tess", "circle", "identity", Some(0.1), "allocs_per_tess", 1.0),
            r("tess", "bspline3", "identity", Some(1.0), "allocs_per_tess", worst),
        ]
    };
    assert_eq!(status(&mk(1.0), "tessellate-one-allocation"), Some(Status::Pass));
    assert_eq!(status(&mk(1.5), "tessellate-one-allocation"), Some(Status::Fail));
    let why =
        verdicts(&mk(7.0)).into_iter().find(|v| v.id == "tessellate-one-allocation").expect("verdict").detail;
    assert!(why.contains("bspline3"), "the detail names the worst curve: {why}");
}

#[test]
fn halving_the_budget_may_grow_a_conic_by_at_most_one_and_a_half_times_and_the_pair_is_named() {
    let mk = |at_half: f64, at_quarter: f64| {
        vec![
            r("tess", "circle", "identity", Some(1.0), "segments_per_curve", 100.0),
            r("tess", "circle", "identity", Some(0.5), "segments_per_curve", at_half),
            r("tess", "circle", "identity", Some(0.25), "segments_per_curve", at_quarter),
        ]
    };
    assert_eq!(status(&mk(150.0, 225.0), "conic-halving-1.5x"), Some(Status::Pass));
    let bad = mk(150.0, 226.0);
    assert_eq!(status(&bad, "conic-halving-1.5x"), Some(Status::Fail), "0.5 to 0.25 grew 1.507x");
    let why = verdicts(&bad).into_iter().find(|v| v.id == "conic-halving-1.5x").expect("verdict").detail;
    assert!(why.contains("circle") && why.contains("0.25"), "the pair is named: {why}");
}

#[test]
fn a_million_unit_offset_may_cost_1_2x_and_must_keep_the_scale_aware_tolerance() {
    let mk = |offset_ns: f64, error: f64, tolerance: f64| {
        vec![
            r("eval", "bspline3", "identity", None, "point_at_ns", 100.0),
            r("eval", "bspline3", "chain-offset", None, "point_at_ns", offset_ns),
            r("accuracy", "bspline3", "chain-offset", None, "error", error),
            r("accuracy", "bspline3", "chain-offset", None, "tolerance", tolerance),
        ]
    };
    assert_eq!(
        status(&mk(120.0, 1.0, 1.0), "offset-1.2x-and-accuracy"),
        Some(Status::Pass),
        "1.2x and exactly on the tolerance"
    );
    assert_eq!(status(&mk(121.0, 1.0, 1.0), "offset-1.2x-and-accuracy"), Some(Status::Fail), "1.21x");
    assert_eq!(
        status(&mk(100.0, 1.1, 1.0), "offset-1.2x-and-accuracy"),
        Some(Status::Fail),
        "outside P1.2's tolerance"
    );
}

#[test]
fn a_speed_obtained_by_breaking_the_error_budget_is_not_a_result() {
    let mk = |achieved: f64| vec![r("tess", "ellipse", "identity", Some(0.5), "achieved_error_px", achieved)];
    assert_eq!(status(&mk(0.5), "error-budget-never-exceeded"), Some(Status::Pass));
    assert_eq!(status(&mk(0.51), "error-budget-never-exceeded"), Some(Status::Fail));
}

#[test]
fn the_csv_has_the_columns_every_curve_type_every_transform_and_the_four_budgets_on_x64() {
    assert_eq!(csv().lines().next(), Some(HEADER));
    let rows = rows();
    for c in CURVES {
        assert!(
            rows.iter().any(|r| r.table == "eval" && r.curve == c && r.platform == X && r.value.is_some()),
            "no eval row for {c}"
        );
    }
    for t in TRANSFORMS {
        assert!(
            rows.iter()
                .any(|r| r.table == "eval" && r.transform == t && r.platform == X && r.value.is_some()),
            "no eval row for transform {t}"
        );
    }
    for b in [0.1, 0.25, 0.5, 1.0] {
        assert!(
            rows.iter().any(|r| r.table == "tess"
                && r.budget_px == Some(b)
                && r.metric == "segments_per_curve"
                && r.value.is_some()),
            "no tessellation at {b} px"
        );
    }
    for m in ["point_at_ns", "derivative_ns", "bounds_ns", "hit_test_ns"] {
        assert!(rows.iter().any(|r| r.metric == m), "no {m} row (a gap is a row with a note)");
    }
    for m in ["ns_per_segment", "segments_per_curve", "allocs_per_tess", "achieved_error_px"] {
        assert!(
            rows.iter().any(|r| r.table == "tess" && r.metric == m && r.value.is_some()),
            "no {m} number"
        );
    }
    assert!(
        rows.iter().any(|r| r.table == "summary" && r.entry == "architectural-1e5"),
        "no viewport-weighted summary"
    );
}

#[test]
fn a_gap_is_a_row_with_a_note_and_every_row_names_a_platform() {
    for r in rows() {
        assert!(["linux-x86_64", "linux-aarch64"].contains(&r.platform.as_str()), "{r:?}");
        assert!(
            r.value.is_some() || !r.note.trim().is_empty(),
            "{} / {} / {} / {} has no value and no note",
            r.table,
            r.curve,
            r.transform,
            r.metric
        );
    }
}

#[test]
fn the_document_names_the_corpus_digest_records_every_verdict_and_says_where_each_table_ran() {
    let d = doc();
    let lock =
        fs::read_to_string(repo().join("crates/pvec-bench/benches/corpus/corpus.lock.json")).expect("lock");
    let digest = pvec_corpus::sha256::hex_digest(lock.as_bytes());
    assert!(d.contains(&digest), "GEOMETRY.md does not name the corpus.lock.json digest {digest}");
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
    for needle in ["arm64", "native x64", "P8.6", "P1.3", "libviprs/maplibre-pvec#1", "scalar", "perf"] {
        assert!(d.contains(needle), "GEOMETRY.md never says {needle:?}");
    }
}
