//! Issue #120 and #200: the generator's refusal of an instance record points at the recorded decision,
//! and it only flattens an instance when the record itself says the structure is checked in the source mapping.

mod common;
use common::*;
use pvec_corpus::ExpectedMath;
use pvec_testkit::generate::{Cause, generate_all, generate_record};

const INSTANCE_FIXTURES: [&str; 7] = [
    "nested-blocks-001",
    "nonuniform-scale-circle-001",
    "nonuniform-scale-rotated-001",
    "reflection-001",
    "rotation-irrational-001",
    "translation-001",
    "uniform-scale-001",
];

#[test]
fn issue_200_the_seven_instance_fixtures_of_the_pinned_bundle_are_emitted_as_their_world_space_curve() {
    let outcomes = generate_all(&bundle()).unwrap();
    for id in INSTANCE_FIXTURES {
        let o = outcomes.iter().find(|o| o.fixture == id).unwrap();
        let a = o.result.as_ref().unwrap_or_else(|r| panic!("{id} is refused: {r}"));
        assert!(!a.bytes.is_empty(), "{id}");
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
