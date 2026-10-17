//! The documented stability rule of EntityId (issue P3.6).

use pvec_checks::*;
use std::fs;

fn doc() -> String {
    fs::read_to_string(repo_root().join("docs/ENTITY_ID.md")).expect("docs/ENTITY_ID.md exists")
}

#[test]
fn the_document_states_the_stability_rule_and_does_not_promise_more() {
    let d = doc();
    for needle in [
        "stable within a given artifact",
        "not stable across regenerations from a changed source",
        "reordering the input changes the ids",
        "same input and corpus revision",
        "dense",
        "the same `EntityId`",
    ] {
        assert!(d.contains(needle), "docs/ENTITY_ID.md does not say {needle:?}");
    }
    let lower = d.to_lowercase();
    assert!(!lower.contains("globally unique"), "EntityId is not globally unique");
    assert!(!lower.contains("across files"), "no cross-artifact identity is promised");
}

#[test]
fn source_identity_stays_out_of_the_id() {
    let d = doc();
    assert!(d.contains("provenance"), "the document says where source identity lives");
    assert!(d.contains("never encoded inside"), "the document forbids encoding source identity in the id");
}
