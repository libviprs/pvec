//! Issue #120: the decision on instance and transform storage is recorded, in ADR 0017.

use pvec_checks::*;
use std::fs;

const SEVEN: [&str; 7] = [
    "nested-blocks-001",
    "nonuniform-scale-circle-001",
    "nonuniform-scale-rotated-001",
    "reflection-001",
    "rotation-irrational-001",
    "translation-001",
    "uniform-scale-001",
];

fn adr() -> String {
    fs::read_to_string(repo_root().join("docs/adr/0017-instance-and-transform-storage.md"))
        .expect("docs/adr/0017-instance-and-transform-storage.md exists")
}

#[test]
fn issue_120_adr_0017_has_the_required_sections_and_a_status() {
    let t = adr();
    assert!(t.starts_with("# ADR 0017"), "title");
    assert!(t.contains("\nStatus: "), "status");
    for h in ["Context", "Options", "Decision", "Consequences", "Reversal threshold"] {
        assert!(t.contains(&format!("\n## {h}")), "no '{h}' section");
    }
}

#[test]
fn issue_120_adr_0017_decides_one_way_and_names_the_alternatives_it_weighed() {
    let t = adr();
    let lower = t.to_lowercase();
    assert!(lower.contains("curve-only"), "the decision is stated as curve-only or its opposite");
    assert!(lower.contains("instance stream"), "the issue's own alternative A is weighed");
    assert!(lower.contains("component stream"), "the issue's alternative B is weighed");
    assert!(t.contains("nested-blocks-001") && lower.contains("three levels"), "nesting is the deciding case");
    assert!(t.contains("#49") && t.contains("#37"), "it builds on the transform ADR and the no-silent-approximation rule");
}

#[test]
fn issue_120_adr_0017_says_what_happens_to_each_of_the_seven_fixtures() {
    let t = adr();
    for id in SEVEN {
        assert!(t.contains(id), "{id} is not accounted for");
    }
    assert!(t.contains("WORLD_SPACE_RESULT") && t.contains("TRANSFORM_PRESERVED"), "both assertions are named");
    assert!(t.contains("libviprs/pvec-gold-fixtures"), "the re-tag lands in the corpus repository");
}

#[test]
fn issue_120_adr_0017_has_a_reversal_threshold_with_a_condition() {
    let t = adr();
    let rest = &t[t.find("\n## Reversal threshold").expect("section")..];
    assert!(rest.len() > 120, "stub");
    assert!(rest.contains("Reopen") && rest.contains(" if "), "no condition");
}
