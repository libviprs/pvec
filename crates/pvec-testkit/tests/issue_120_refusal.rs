//! Issue #120 and #200: the generator's refusal of an instance record points at the recorded decision,
//! and it only flattens an instance when the record itself says the structure is checked in the source mapping.

mod common;
use common::*;
use pvec_corpus::ExpectedMath;
use pvec_testkit::generate::{Cause, generate_all, generate_record};

const INSTANCE_FIXTURES: [&str; 6] = [
    "nested-blocks-001",
    "nonuniform-scale-circle-001",
    "nonuniform-scale-rotated-001",
    "reflection-001",
    "rotation-irrational-001",
    "uniform-scale-001",
];

#[test]
fn issue_200_the_six_in_plane_instance_fixtures_of_the_pinned_bundle_are_emitted_as_their_world_space_curve() {
    let outcomes = generate_all(&bundle()).unwrap();
    for id in INSTANCE_FIXTURES {
        let o = outcomes.iter().find(|o| o.fixture == id).unwrap();
        let a = o.result.as_ref().unwrap_or_else(|r| panic!("{id} is refused: {r}"));
        assert!(!a.bytes.is_empty(), "{id}");
    }
}

#[test]
fn issue_200_translation_001_stays_refused_because_its_world_curve_is_off_the_drawing_plane() {
    let outcomes = generate_all(&bundle()).unwrap();
    let o = outcomes.iter().find(|o| o.fixture == "translation-001").unwrap();
    let r = o.result.as_ref().expect_err("a line at z = 2 is not held by a 2D profile");
    match &r.cause {
        Cause::MissingCapability { capability, what } => {
            assert_eq!(capability, "line");
            assert!(what.contains("off the drawing plane"), "{what}");
        }
        other => panic!("{other:?}"),
    }
}

/// translation-001 from the bundle, with its source-mapping assertion put back to the old structural one.
fn translation_with_old_assertion() -> ExpectedMath {
    let b = bundle();
    let bytes = b.file("fixtures/translation-001/expected.math.json").expect("the record is in the bundle");
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.contains("\"SOURCE_MAPPING\""), "the pinned record asserts the structure in the source mapping");
    ExpectedMath::parse(text.replace("\"SOURCE_MAPPING\"", "\"STRUCTURAL\"").as_bytes()).unwrap()
}

#[test]
fn issue_120_an_instance_record_that_still_asserts_the_structure_in_the_artifact_is_refused_citing_adr_0017() {
    let math = translation_with_old_assertion();
    let r = generate_record(&math, REVISION).expect_err("the artifact cannot hold an instance plus a transform");
    match &r.cause {
        Cause::MissingCapability { capability, what } => {
            assert_eq!(capability, "affine_transform_2d");
            assert!(what.contains("ADR 0017"), "{what}");
        }
        other => panic!("{other:?}"),
    }
}

fn doc(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

#[test]
fn issue_200_the_refusal_of_translation_001_cites_the_decision_that_makes_it_final() {
    let outcomes = generate_all(&bundle()).unwrap();
    let o = outcomes.iter().find(|o| o.fixture == "translation-001").unwrap();
    let text = o.result.as_ref().expect_err("refused").to_string();
    assert!(text.contains("ADR 0017"), "the message points at the recorded decision: {text}");
}

#[test]
fn issue_200_exactly_two_of_the_thirty_registered_fixtures_are_refused_and_both_are_by_design() {
    let outcomes = generate_all(&bundle()).unwrap();
    let registered: Vec<_> = outcomes.iter().filter(|o| !matches!(&o.result, Err(r) if r.cause == Cause::Quarantined)).collect();
    assert_eq!(registered.len(), 30);
    let refused: Vec<&str> = registered.iter().filter(|o| o.result.is_err()).map(|o| o.fixture.as_str()).collect();
    assert_eq!(refused, ["near-singular-001", "translation-001"]);
    assert_eq!(registered.len() - refused.len(), 28);
}

#[test]
fn issue_200_the_two_refusals_by_design_are_recorded_in_adr_0017_and_in_the_corpus_chapter() {
    let adr = doc("adr/0017-instance-and-transform-storage.md");
    assert!(adr.contains("## Amendment 2026-10-31: translation-001 is refused by design"), "ADR 0017 records the decision");
    assert!(adr.contains("ISO_PROFILE.md"), "the decision rests on the profile's no-promotion rule");
    let corpus = doc("CORPUS.md");
    assert!(corpus.contains("## Refusals by design"), "CORPUS.md has the section");
    let section = corpus.split("## Refusals by design").nth(1).unwrap();
    for needle in ["28 of 30", "near-singular-001", "translation-001", "maplibre-pvec"] {
        assert!(section.contains(needle), "the section names {needle}");
    }
}
