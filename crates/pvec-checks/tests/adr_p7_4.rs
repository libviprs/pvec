//! Issue #55 (P7.4): the C/FFI strategy is a decision, and a decision with no numbers fails review.

use pvec_checks::repo_root;
use std::fs;

fn adr() -> String {
    fs::read_to_string(repo_root().join("docs/adr/ADR-P7.4-c-ffi-strategy.md"))
        .expect("docs/adr/ADR-P7.4-c-ffi-strategy.md exists")
}

#[test]
fn issue_55_the_adr_has_the_sections_the_issue_asks_for() {
    let t = adr();
    for h in ["Context", "Options", "Decision", "Consequences", "Reversal threshold"] {
        assert!(t.contains(&format!("\n## {h}")), "no '{h}' section");
    }
    assert!(t.contains("\nStatus: "));
}

#[test]
fn issue_55_all_four_candidates_are_measured_with_numbers_and_a_platform() {
    let t = adr();
    for c in ["(a)", "(b)", "(c)", "(d)"] {
        assert!(t.contains(c), "candidate {c} is missing");
    }
    for needle in ["cbindgen", "#[repr(C)]", "exported C symbols", "native", "x86_64", "arm64"] {
        assert!(t.contains(needle), "the ADR does not mention {needle}");
    }
    // the table row of symbol counts carries a number for every candidate
    let row = t.lines().find(|l| l.contains("exported C symbols")).expect("the symbol count row");
    assert!(row.matches(|c: char| c.is_ascii_digit()).count() >= 4, "{row}");
}

#[test]
fn issue_55_one_sentence_says_whether_v1_ships_a_c_header() {
    let t = adr();
    let said = t.contains("v1 does not ship a C header.") || t.contains("v1 ships a C header.");
    assert!(said, "no sentence says whether v1 ships a C header");
}

#[test]
fn issue_55_a_deferral_names_its_trigger_and_the_enforcement_of_the_no_rust_layout_rule() {
    let t = adr();
    if t.contains("v1 does not ship a C header.") {
        let threshold = t.split("\n## Reversal threshold").nth(1).expect("a reversal threshold");
        assert!(threshold.contains("named in an"), "the deferral has no triggering condition");
        assert!(t.contains("P8.7"), "the deferral is not tied to the v1 freeze");
    }
    assert!(t.contains("improper_ctypes_definitions"), "the no-Rust-layout rule has no enforcement");
}

#[test]
fn issue_55_the_prototypes_and_their_raw_results_are_in_the_tree() {
    for f in [
        "spikes/p7-4-ffi/run.sh",
        "spikes/p7-4-ffi/results/mars-amd64-native.txt",
        "spikes/p7-4-ffi/results/mercury-arm64-native.txt",
    ] {
        assert!(repo_root().join(f).exists(), "{f} is missing");
    }
}
