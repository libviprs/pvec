//! Issue #34: the fixture registry. Every fixture is named, carries one of the four evidence classes,
//! and exercises capabilities that exist in the registry and in `docs/ISO_PROFILE.md`. What a fixture
//! may be cited for follows from its class and its oracles, and r0-open has no oracle.

mod common;

use common::*;
use pvec_capabilities::{EvidenceClass, ids};
use pvec_corpus::{Bundle, Claim, CorpusError, FixtureRegistry};

fn vendored_registry() -> FixtureRegistry {
    let bundle = Bundle::vendored().unwrap();
    FixtureRegistry::load(&bundle, &pvec_capabilities::Registry::v1(), &profile()).expect("r0-open registers")
}

#[test]
fn issue_34_all_thirty_accepted_fixtures_register_and_the_three_exclusions_are_quarantined() {
    let r = vendored_registry();
    assert_eq!(r.fixtures().len(), 33);
    let known: Vec<_> = r.fixtures().iter().filter(|f| f.class == EvidenceClass::KnownTruthExact).collect();
    let quarantined: Vec<_> = r.fixtures().iter().filter(|f| f.class == EvidenceClass::Quarantined).collect();
    assert_eq!(known.len(), 30);
    assert_eq!(quarantined.len(), 3);
    let mut q: Vec<&str> = quarantined.iter().map(|f| f.id.as_str()).collect();
    q.sort_unstable();
    assert_eq!(q, ["general-affine-001", "shear-001", "singular-001"]);
    for f in quarantined {
        assert!(f.dwg_path.is_none() && f.expected.is_none(), "{} has no file to load", f.id);
    }
    for f in known {
        assert!(f.expected.is_some(), "{}", f.id);
        let dwg = f.dwg_path.as_deref().unwrap();
        assert!(
            Bundle::vendored().unwrap().file(dwg).is_some_and(|b| b.starts_with(b"AC1032")),
            "{dwg} is an AC1032 DWG"
        );
        assert_eq!(f.scope, "READER_MAPPING_ONLY", "{}", f.id);
        assert_eq!(f.oracles, "none", "{}", f.id);
    }
}

#[test]
fn issue_34_no_class_carries_more_than_the_evidence_behind_it() {
    let r = vendored_registry();
    for f in r.fixtures() {
        assert!(!f.supports(Claim::StepExactness), "{}: there is no STEP oracle in r0-open", f.id);
        assert!(!f.supports(Claim::DualOracleAgreement), "{}: there is no converter pair in r0-open", f.id);
        let mapping = f.class == EvidenceClass::KnownTruthExact;
        assert_eq!(f.supports(Claim::ReaderMapping), mapping, "{}", f.id);
    }
}

#[test]
fn issue_34_near_singular_is_the_only_fixture_a_mapper_must_not_draw() {
    let r = vendored_registry();
    for f in r.fixtures() {
        assert_eq!(f.must_not_emit_curve, f.id == "near-singular-001", "{}", f.id);
    }
}

#[test]
fn issue_34_corpus_capability_names_resolve_to_registry_ids() {
    let r = vendored_registry();
    let caps = |id: &str| r.get(id).unwrap_or_else(|| panic!("{id} is registered")).capabilities.clone();
    assert_eq!(caps("line-001"), [ids::LINE]);
    assert_eq!(caps("circle-001"), [ids::CIRCLE]);
    assert_eq!(caps("ellipse-001"), [ids::ELLIPSE]);
    assert_eq!(caps("arc-seam-001"), [ids::TRIMMED_CURVE]);
    assert_eq!(caps("elliptical-arc-001"), [ids::TRIMMED_CURVE]);
    assert_eq!(caps("bulge-arc-001"), [ids::COMPOSITE_CURVE]);
    assert_eq!(caps("polyline-001"), [ids::POLYLINE]);
    assert_eq!(caps("bezier-d3-001"), [ids::BEZIER_CURVE]);
    assert_eq!(caps("bspline-d5-001"), [ids::B_SPLINE_CURVE]);
    assert_eq!(caps("nurbs-d3-001"), [ids::RATIONAL_B_SPLINE_CURVE]);
    assert_eq!(caps("reflection-001"), [ids::AFFINE_TRANSFORM_2D]);
    assert_eq!(r.get("bezier-d3-001").unwrap().corpus_capabilities, ["bezier"]);
    // layer-state-001 exercises nothing the 2D profile has: its manifest says so in a note.
    assert!(caps("layer-state-001").is_empty());
}

#[test]
fn issue_34_a_fixture_with_no_evidence_class_fails_registration() {
    let (tar, pin) = mutated(|f| {
        edit_manifest_entry(f, "circle-001", "evidence_class", "evidence_class", "evidence_clazz")
    });
    match register(&tar, &pin) {
        Err(CorpusError::Unclassified { fixture, found: None }) => assert_eq!(fixture, "circle-001"),
        other => panic!("expected Unclassified, got {other:?}"),
    }
}

#[test]
fn issue_34_a_fixture_with_a_class_outside_the_four_fails_registration() {
    let (tar, pin) = mutated(|f| {
        edit_manifest_entry(f, "circle-001", "evidence_class", "KNOWN_TRUTH_EXACT", "MOSTLY_EXACT")
    });
    match register(&tar, &pin) {
        Err(e @ CorpusError::Unclassified { .. }) => {
            assert!(e.to_string().contains("circle-001") && e.to_string().contains("MOSTLY_EXACT"), "{e}");
        }
        other => panic!("expected Unclassified, got {other:?}"),
    }
}

#[test]
fn issue_34_a_class_the_evidence_cannot_carry_fails_registration() {
    for class in ["DUAL_ORACLE_EQUIVALENT", "REAL_WORLD_DIFFERENTIAL"] {
        let (tar, pin) =
            mutated(|f| edit_manifest_entry(f, "circle-001", "evidence_class", "KNOWN_TRUTH_EXACT", class));
        match register(&tar, &pin) {
            Err(CorpusError::ClassWithoutEvidence { fixture, class: c, .. }) => {
                assert_eq!((fixture.as_str(), c.as_str()), ("circle-001", class));
            }
            other => panic!("{class}: expected ClassWithoutEvidence, got {other:?}"),
        }
    }
}

#[test]
fn issue_34_a_manifest_class_the_record_contradicts_fails_registration() {
    let (tar, pin) = mutated(|f| {
        edit_manifest_entry(f, "circle-001", "evidence_class", "KNOWN_TRUTH_EXACT", "QUARANTINED")
    });
    // A quarantined entry that still ships a record must not be silently treated as known truth.
    match register(&tar, &pin) {
        Err(CorpusError::Invalid { what, .. }) => assert!(what.contains("circle-001"), "{what}"),
        other => panic!("expected the contradiction to be refused, got {other:?}"),
    }
}

#[test]
fn issue_34_a_capability_the_registry_does_not_know_fails_registration_naming_it() {
    let (tar, pin) =
        mutated(|f| edit_manifest_entry(f, "circle-001", "capabilities", "\"circle\"", "\"spline_surface\""));
    match register(&tar, &pin) {
        Err(e @ CorpusError::UnknownCapability { .. }) => {
            let CorpusError::UnknownCapability { fixture, capability } = &e else { unreachable!() };
            assert_eq!((fixture.as_str(), capability.as_str()), ("circle-001", "spline_surface"));
            assert!(e.to_string().contains("spline_surface"), "{e}");
        }
        other => panic!("expected UnknownCapability, got {other:?}"),
    }
}

#[test]
fn issue_34_a_capability_absent_from_the_iso_profile_fails_registration_naming_it() {
    let bundle = Bundle::vendored().unwrap();
    let caps = pvec_capabilities::Registry::v1();
    let doctored = profile().replacen("\n## 10. ", "\n## ten. ", 1);
    assert_ne!(doctored, profile(), "section 10 (Bezier) was found");
    match FixtureRegistry::load(&bundle, &caps, &doctored) {
        Err(e @ CorpusError::CapabilityNotInProfile { .. }) => {
            let msg = e.to_string();
            assert!(msg.contains("bezier-d3-001") && msg.contains("bezier"), "{msg}");
        }
        other => panic!("expected CapabilityNotInProfile, got {other:?}"),
    }
}

#[test]
fn issue_34_every_fixture_id_is_unique_and_matches_its_directory() {
    let r = vendored_registry();
    let mut ids: Vec<&str> = r.fixtures().iter().map(|f| f.id.as_str()).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n);
    for f in r.fixtures().iter().filter_map(|f| f.dwg_path.as_ref().map(|p| (f, p))) {
        assert_eq!(f.1, &format!("fixtures/{}/source.dwg", f.0.id));
    }
}
