//! The extension governance document, the proposal template and the committed sample (issue P3.5).

use pvec_capabilities::*;
use pvec_checks::*;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|_| panic!("{rel} exists"))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The sample with one `## heading` section removed.
fn without_section(text: &str, heading: &str) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            skipping = h.trim() == heading;
        }
        if !skipping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

#[test]
fn the_governance_document_states_the_rules() {
    let d = flat(&read("docs/EXTENSIONS.md"));
    for needle in [
        "The profile rule",
        "A distinct DWG (or PDF, or any source) entity class is not a reason for a distinct PVec mathematical type",
        "no suitable ISO 10303-42 representation exists",
        "naming each Part 42 construct considered",
        "mathematical definition",
        "bounds and evaluation rules",
        "tessellation rules",
        "STEP and GIS behaviour",
        "KNOWN_TRUTH_EXACT",
        "DUAL_ORACLE_EQUIVALENT",
        "REAL_WORLD_DIFFERENTIAL",
        "QUARANTINED",
        "optional capabilities by default",
        "an accompanying ADR",
        "may not be recorded as exact without a passing round-trip fixture",
        "no exact representation; bounded approximation with error reported",
        "docs/extensions/TEMPLATE.md",
        "docs/extensions/sample-proposal.md",
    ] {
        assert!(d.contains(needle), "docs/EXTENSIONS.md does not say {needle:?}");
    }
    assert!(forbidden_claims(&read("docs/EXTENSIONS.md")).is_empty());
}

#[test]
fn the_committed_sample_proposal_passes_the_checker_end_to_end() {
    let sample = read("docs/extensions/sample-proposal.md");
    let summary = check_proposal(&sample).unwrap_or_else(|p| panic!("the sample is incomplete: {p:?}"));
    assert!(!summary.fixtures.is_empty());
    assert!(sample.contains("SAMPLE"), "the sample says it is a sample and registers nothing");
}

#[test]
fn deleting_any_one_section_of_the_sample_makes_it_fail_with_that_section_named() {
    let sample = read("docs/extensions/sample-proposal.md");
    for (heading, artifact) in [
        ("Part 42 constructs considered", MissingArtifact::Part42Argument),
        ("Mathematical definition", MissingArtifact::MathematicalDefinition),
        ("Bounds and evaluation rules", MissingArtifact::BoundsAndEvaluation),
        ("Tessellation rules", MissingArtifact::TessellationRules),
        ("STEP behaviour", MissingArtifact::StepBehaviour),
        ("GIS behaviour", MissingArtifact::GisBehaviour),
        ("Conformance fixtures", MissingArtifact::Fixtures),
    ] {
        let cut = without_section(&sample, heading);
        assert_ne!(cut, sample, "the sample has a '{heading}' section");
        assert_eq!(check_proposal(&cut).unwrap_err(), vec![ProposalProblem::Missing(artifact)], "without '{heading}'");
    }
}

#[test]
fn the_template_has_every_section_and_does_not_pass_until_it_is_filled_in() {
    let template = read("docs/extensions/TEMPLATE.md");
    for h in [
        "Part 42 constructs considered",
        "Mathematical definition",
        "Bounds and evaluation rules",
        "Tessellation rules",
        "STEP behaviour",
        "GIS behaviour",
        "Conformance fixtures",
    ] {
        assert!(template.contains(&format!("## {h}")), "the template has no '{h}' section");
    }
    assert!(check_proposal(&template).is_err(), "an unfilled template must not pass the gate");
}

#[test]
fn the_registry_with_the_sample_registered_still_validates_against_the_profile() {
    let text = read("docs/ISO_PROFILE.md");
    let sections: Vec<ProfileSection> = parse_profile(&text)
        .into_iter()
        .map(|e| ProfileSection {
            number: e.title.split('.').next().unwrap().parse().unwrap(),
            constructs: e
                .fields
                .iter()
                .find(|(k, _)| k == "ISO 10303-42:2025 construct")
                .map(|(_, v)| v.clone())
                .unwrap_or_default(),
        })
        .collect();
    let mut r = Registry::v1();
    let id = r.entries().len() as u32 + 1;
    let cap = Capability {
        id: CapabilityId::new(id),
        name: "sample_extension",
        class: Class::Extension,
        version: Version::new(0, 1, 0),
        iso_construct: "",
        profile_section: 0,
        requirement: Requirement::Optional,
        step: StepBehaviour::BoundedApproximation,
        gis: GisBehaviour::Densified,
        min_reader: Version::new(1, 0, 0),
        restriction: "sample",
        adr: None,
    };
    r.register_extension(cap, &read("docs/extensions/sample-proposal.md")).unwrap();
    r.validate_against_profile(&sections).unwrap();
}
