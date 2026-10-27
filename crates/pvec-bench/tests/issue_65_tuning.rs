//! Issue P8.6 (pvec#65, and pvec#192 for the thresholds that failed): the tuning log, and what it has to
//! be true to.
//!
//! These were written before the first parameter changed. They hold `docs/bench/TUNING.md` to the rules of
//! the issue: one row per change with a measured metric behind it, the checked-in defaults equal to the log,
//! a net improvement on two of the four headline metrics, no unexplained regression over 5%, and every
//! threshold the committed runs still miss written down as missed. They do not loosen anything: a threshold
//! that is met is asserted met, and one that is not is asserted listed.
#![allow(missing_docs)]

use pvec_bench::{geometry_bench, layout_bench, progressive_bench, step_bench};
use pvec_container::compress::{Codec, CodecPolicy};
use pvec_container::directory::BlockKind;
use pvec_container::reader::ReaderConfig;
use pvec_container::writer::ADVANCED_CHUNK_BYTES;
use pvec_stream::{CELL_CAPACITY, PREVIEW_BUDGET_PX};
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

const X: &str = "linux-x86_64";
const A: &str = "linux-aarch64";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn log() -> String {
    read("docs/bench/TUNING.md")
}

/// The cells of the table under `## heading`, header and rule rows dropped, backticks stripped.
fn table(doc: &str, heading: &str) -> Vec<Vec<String>> {
    let mut lines = doc.lines().skip_while(|l| l.trim() != format!("## {heading}")).skip(1);
    let mut rows = Vec::new();
    for l in lines.by_ref() {
        if l.trim_start().starts_with('|') {
            rows.push(
                l.trim().trim_matches('|').split('|').map(|c| c.trim().replace('`', "")).collect::<Vec<_>>(),
            );
        } else if !rows.is_empty() {
            break;
        } else if l.starts_with("## ") {
            panic!("no table under `## {heading}`");
        }
    }
    assert!(rows.len() > 2, "the table under `## {heading}` has no rows");
    rows.drain(..2);
    rows
}

fn section(doc: &str, heading: &str) -> String {
    let body: Vec<&str> = doc
        .lines()
        .skip_while(|l| l.trim() != format!("## {heading}"))
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .collect();
    assert!(!body.is_empty(), "no `## {heading}` section");
    body.join("\n")
}

/// Every verdict of the four committed reports as `(id, platform, label, detail)`.
fn committed_verdicts() -> Vec<(String, String, &'static str, String)> {
    let mut out = Vec::new();
    let rows = progressive_rows();
    for v in progressive_bench::verdicts(&rows) {
        out.push((v.id.to_string(), v.platform, v.status.label(), v.detail));
    }
    let rows = layout_bench::parse_rows(&read("docs/bench/layout.csv")).unwrap_or_else(|e| panic!("{e}"));
    for v in layout_bench::verdicts(&rows) {
        out.push((v.id.to_string(), v.platform, v.status.label(), v.detail));
    }
    let rows = geometry_bench::parse_rows(&read("docs/bench/geometry.csv")).unwrap_or_else(|e| panic!("{e}"));
    for v in geometry_bench::verdicts(&rows) {
        out.push((v.id.to_string(), v.platform, v.status.label(), v.detail));
    }
    let rows =
        step_bench::parse_rows(&read("docs/bench/step_comparison.csv")).unwrap_or_else(|e| panic!("{e}"));
    for v in step_bench::verdicts(&rows) {
        out.push((v.id.to_string(), v.platform, v.status.label(), v.detail));
    }
    out
}

fn progressive_rows() -> Vec<layout_bench::Row> {
    layout_bench::parse_rows(&read("docs/bench/progressive.csv")).unwrap_or_else(|e| panic!("{e}"))
}

fn label(id: &str, platform: &str) -> &'static str {
    committed_verdicts()
        .into_iter()
        .find(|v| v.0 == id && v.1 == platform)
        .map(|v| v.2)
        .unwrap_or_else(|| panic!("no verdict {id} on {platform} in the committed reports"))
}

// ---------------------------------------------------------------- the log

#[test]
fn the_log_exists_and_states_the_rule_that_an_unmeasured_parameter_is_not_tuned_here() {
    let d = log();
    assert!(
        d.contains("a parameter with no measurement behind it is not tuned here"),
        "TUNING.md never states the rule"
    );
    for forbidden in [
        "a P1.2 tolerance",
        "a P4.8 limit",
        "the decompression expansion ratio",
        "a checksum",
        "a conformance test",
        "the Tier 0 coverage requirement",
    ] {
        assert!(d.contains(forbidden), "TUNING.md does not restate that {forbidden} is not tuned");
    }
}

#[test]
fn every_row_names_a_measured_metric_before_and_after_on_both_platforms_and_the_corpus_digest() {
    let known: BTreeSet<&str> = progressive_bench::IDS
        .iter()
        .chain(layout_bench::IDS.iter())
        .chain(step_bench::IDS.iter())
        .chain(geometry_bench::IDS.iter())
        .copied()
        .collect();
    let digest =
        pvec_corpus::sha256::hex_digest(read("crates/pvec-bench/benches/corpus/corpus.lock.json").as_bytes());
    let rows = table(&log(), "Changes");
    assert!(rows.len() >= 6, "{} rows: the surface changed in more places than that", rows.len());
    for r in &rows {
        assert_eq!(
            r.len(),
            9,
            "{r:?}: # | parameter | old | new | metric | before | after | corpus digest | re-run"
        );
        let (param, metric, before, after) = (&r[1], &r[4], &r[5], &r[6]);
        let ids: Vec<&str> = metric.split(',').map(str::trim).collect();
        assert!(
            !ids.is_empty() && ids.iter().all(|i| known.contains(i)),
            "row {param}: `{metric}` is not a P8.2-P8.5 metric id"
        );
        for (what, cell) in [("before", before), ("after", after)] {
            assert!(
                cell.contains("x64") && cell.contains("arm64"),
                "row {param}: {what} has no number for both platforms"
            );
            assert!(cell.chars().any(|c| c.is_ascii_digit()), "row {param}: {what} has no number");
        }
        assert!(r[7].contains(&digest), "row {param}: the corpus digest {digest} is not named");
        assert!(r[8].starts_with("pass"), "row {param}: the conformance and security re-run is `{}`", r[8]);
    }
}

#[test]
fn the_checked_in_defaults_are_the_new_values_of_the_log() {
    let rows = table(&log(), "Changes");
    let d = CodecPolicy::default();
    let actual: Vec<(&str, String)> = vec![
        ("pvec_stream::CELL_CAPACITY", CELL_CAPACITY.to_string()),
        ("pvec_stream::PREVIEW_BUDGET_PX", PREVIEW_BUDGET_PX.to_string()),
        ("pvec_container::ReaderConfig::gap_threshold", ReaderConfig::default().gap_threshold.to_string()),
        ("pvec_container::writer::ADVANCED_CHUNK_BYTES", ADVANCED_CHUNK_BYTES.to_string()),
        ("pvec_container::CodecPolicy::hot", d.hot.name().to_string()),
        ("pvec_container::CodecPolicy::records", d.records.name().to_string()),
        ("pvec_container::CodecPolicy::arena", d.arena.name().to_string()),
        ("pvec_container::CodecPolicy::components", d.components.name().to_string()),
        ("pvec_container::CodecPolicy::tier0", d.for_kind(BlockKind::Tier0).name().to_string()),
    ];
    let logged: BTreeSet<&str> =
        rows.iter().map(|r| r[1].as_str()).filter(|p| !p.starts_with("rule:")).collect();
    for r in &rows {
        let (param, new) = (&r[1], &r[3]);
        if param.starts_with("rule:") {
            continue;
        }
        let (_, value) = actual
            .iter()
            .find(|(name, _)| name == param)
            .unwrap_or_else(|| panic!("row `{param}` names a parameter the test cannot read from the code"));
        assert_eq!(value, new, "{param}: the code says {value}, the log says {new}");
    }
    // a default that is not in the log was changed without a row, or never tuned: unchanged ones are listed
    let unchanged = section(&log(), "Not tuned");
    for (name, value) in &actual {
        if !logged.contains(name) {
            assert!(
                unchanged.contains(name),
                "{name} (= {value}) is neither a row of the log nor listed under `Not tuned`"
            );
        }
    }
    // the codec ids are the ones the file records
    assert_eq!(Codec::from_id(Codec::Deflate.id()), Some(Codec::Deflate));
}

#[test]
fn the_headline_metrics_improve_on_at_least_two_of_four_and_no_regression_over_5_percent_is_unexplained() {
    let rows = table(&log(), "Headline metrics");
    assert_eq!(
        rows.len(),
        4,
        "P8.2 viewport p95, P8.3 total viewport bytes, P8.4 ns per segment, P8.5 time to Tier 0"
    );
    let mut better = 0;
    for r in &rows {
        assert_eq!(r.len(), 6, "{r:?}: metric | unit | before | after | change | trade");
        let (before, after): (f64, f64) = (r[2].parse().unwrap(), r[3].parse().unwrap());
        if after < before {
            better += 1;
        }
        if after > before * 1.05 {
            assert!(
                r[5].len() > 20 && r[5] != "-",
                "{}: up {:.0}% with no trade stated",
                r[0],
                (after / before - 1.0) * 100.0
            );
        }
    }
    assert!(better >= 2, "{better} of 4 headline metrics improved");
}

// ---------------------------------------------------------------- what the committed runs say

#[test]
fn the_progressive_thresholds_the_tuning_meets_are_met_on_both_platforms() {
    for platform in [X, A] {
        for id in [
            "tier0-1500ms",
            "tier0-4s-5mbit",
            "coverage-complete",
            "preview-2px",
            "preview-identity",
            "refine-5s",
            "pan-discard-20pct",
        ] {
            assert_eq!(label(id, platform), "PASS", "{id} on {platform}");
        }
    }
}

#[test]
fn the_soa_path_is_three_times_the_per_entity_records_on_both_platforms() {
    for platform in [X, A] {
        assert_eq!(label("soa-3x-record", platform), "PASS", "soa-3x-record on {platform}");
    }
}

#[test]
fn every_threshold_still_missed_is_listed_with_its_numbers_and_nothing_passing_is_listed() {
    let listed = section(&log(), "Still failing");
    let verdicts = committed_verdicts();
    for (id, platform, label, detail) in &verdicts {
        let line = format!("- {id} on {platform}: FAIL");
        if *label == "FAIL" {
            assert!(
                listed.contains(&line),
                "`{line}` is missed in the committed run and not listed under `Still failing`"
            );
            let short: String = detail.chars().take(60).collect();
            assert!(listed.contains(&short), "`{line}` is listed without its numbers `{short}`");
        } else {
            assert!(
                !listed.contains(&line),
                "`{line}` is listed as failing but the committed run says {label}"
            );
        }
    }
}

#[test]
fn every_item_of_issue_192_has_a_disposition_with_numbers() {
    let d = section(&log(), "Issue 192");
    for item in [
        "viewport p95",
        "index bytes moved",
        "block size",
        "SoA",
        "point_at",
        "tessellate",
        "conic halving",
        "0.250185",
    ] {
        let line = d
            .lines()
            .find(|l| l.contains(item))
            .unwrap_or_else(|| panic!("`## Issue 192` never mentions {item}"));
        assert!(
            ["resolved", "improved", "open"].iter().any(|w| line.contains(w)),
            "`{item}` has no disposition (resolved, improved or open): {line}"
        );
        assert!(line.chars().filter(char::is_ascii_digit).count() >= 3, "`{item}` has no numbers: {line}");
    }
    assert!(d.contains("decision for Rom"), "nothing is handed to the freeze as a decision");
}

#[test]
fn the_conformance_limit_and_capability_suites_were_re_run_on_every_row_and_none_was_edited() {
    let rows = table(&log(), "Re-runs");
    for r in &rows {
        assert_eq!(r.len(), 8, "{r:?}: row | P2.5 | P2.6 | P2.7 | P4.8 | P7.8 matrix | P7.8 fuzz | host");
        for cell in &r[1..7] {
            assert!(cell.starts_with("pass"), "row {}: `{cell}`", r[0]);
        }
    }
    let d = section(&log(), "Re-runs");
    assert!(
        d.contains("test files unchanged against main"),
        "the log does not say the suites were left alone"
    );
}
