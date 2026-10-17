//! Acceptance checks for the phase 1 ADRs and the benchmark summary (issue P1.8).

use pvec_checks::*;
use std::fs;

const ADRS: [&str; 8] = [
    "0001-semantic-profile",
    "0002-numeric-policy",
    "0003-physical-container",
    "0004-internal-data-layout",
    "0005-spatial-index",
    "0006-compression-and-checksum",
    "0007-progressive-tiers",
    "0008-extension-rules",
];

fn adr(name: &str) -> String {
    fs::read_to_string(repo_root().join(format!("docs/adr/{name}.md")))
        .unwrap_or_else(|_| panic!("docs/adr/{name}.md exists"))
}

/// The text of `## heading` up to the next `## ` heading.
fn section<'a>(text: &'a str, heading: &str) -> &'a str {
    let start = text.find(&format!("\n## {heading}")).unwrap_or_else(|| panic!("no '## {heading}' section"));
    let rest = &text[start + 1..];
    let body = &rest[rest.find('\n').unwrap_or(0)..];
    body.split("\n## ").next().unwrap_or(body)
}

#[test]
fn eight_adrs_exist_with_the_required_sections() {
    for name in ADRS {
        let t = adr(name);
        assert!(t.starts_with("# ADR 000"), "{name} has no ADR title");
        assert!(t.contains("\nStatus: "), "{name} has no status");
        for h in ["Context", "Options", "Decision", "Consequences", "Reversal threshold"] {
            assert!(t.contains(&format!("\n## {h}")), "{name} has no '{h}' section");
        }
    }
}

#[test]
fn every_options_section_has_a_measured_number_and_a_platform_label() {
    for name in ADRS {
        let t = adr(name);
        let options = section(&t, "Options");
        assert!(options.chars().any(|c| c.is_ascii_digit()), "{name}: no number in Options");
        let lower = options.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
        let labelled = lower.contains("native x64") || lower.contains("arm64");
        // ADR 0001 and 0002 cite platform-labelled evidence in the same section; 0008 does too.
        assert!(labelled, "{name}: Options names no platform (native x64 or arm64)");
    }
}

#[test]
fn every_adr_names_a_reversal_threshold_that_contains_a_condition() {
    for name in ADRS {
        let t = adr(name);
        let threshold = section(&t, "Reversal threshold");
        assert!(threshold.len() > 80, "{name}: reversal threshold is a stub");
        assert!(threshold.chars().any(|c| c.is_ascii_digit()), "{name}: reversal threshold has no number");
    }
}

#[test]
fn every_adr_is_accepted_and_the_numeric_policy_is_accepted_before_the_container() {
    for name in ADRS {
        assert!(adr(name).contains("\nStatus: Accepted"), "{name} is not Accepted");
    }
    let container = adr("0003-physical-container");
    assert!(
        container.contains("ADR 0002") && container.contains("already accepted"),
        "container ADR does not state its precondition"
    );
}

#[test]
fn the_container_adr_cites_both_tables_on_one_fixture_revision() {
    let c = adr("0003-physical-container");
    for needle in ["docs/spikes/01-4-step-hdf.md", "docs/spikes/01-5-layouts.md", "pvec-p1-fixtures-r1"] {
        assert!(c.contains(needle), "container ADR does not cite {needle}");
    }
    let doc4 = fs::read_to_string(repo_root().join("docs/spikes/01-4-step-hdf.md")).unwrap();
    let doc5 = fs::read_to_string(repo_root().join("docs/spikes/01-5-layouts.md")).unwrap();
    assert!(
        doc4.contains("pvec-p1-fixtures-r1") && doc5.contains("pvec-p1-fixtures-r1"),
        "the two spike documents are on different revisions"
    );
}

#[test]
fn the_compression_adr_says_the_codec_is_not_final() {
    let c = adr("0006-compression-and-checksum");
    assert!(c.contains("P4.6") && c.contains("P8.3") && c.contains("not final"));
}

#[test]
fn the_summary_lists_every_fixture_and_labels_its_platform() {
    let s = fs::read_to_string(repo_root().join("docs/spikes/01-8-benchmark-summary.md")).unwrap();
    for f in ["small-synthetic", "spline-heavy", "million", "straddler"] {
        assert!(s.contains(&format!("#### {f}")), "summary has no table for {f}");
    }
    for c in [
        "STEP-HDF tables (H1)",
        "HDF5 SoA (H2)",
        "A FlatBuffers SoA",
        "B custom packed",
        "C rkyv SoA",
        "baseline: FlatBuffers per entity",
    ] {
        assert!(s.matches(c).count() >= 4, "candidate '{c}' is not in every fixture table");
    }
    assert!(
        s.contains("pvec-p1-fixtures-r1")
            && s.contains("native x64")
            && s.contains("No number here was measured on arm64")
    );
}

#[test]
fn no_adr_makes_an_iso_claim_beyond_the_approved_sentence() {
    for name in ADRS {
        assert!(forbidden_claims(&adr(name)).is_empty(), "{name} claims ISO conformance");
    }
}
