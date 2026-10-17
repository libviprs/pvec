//! Extension governance (P3.5): a custom geometry extension has to earn its place. The checker names
//! each missing artifact on its own, extensions are optional unless an ADR is listed, and STEP
//! behaviour may not be recorded as exact without a passing round-trip fixture.
#![allow(missing_docs)]

use pvec_capabilities::*;

/// A complete proposal, section by section, so a test can drop one at a time.
const SECTIONS: [(&str, &str); 7] = [
    (
        "Part 42 constructs considered",
        "- `hyperbola`: a hyperbola is an unbounded conic and this arc is bounded by construction, so it would need a trim the construct does not define\n- `b_spline_curve_with_knots`: a rational spline approximates a hyperbolic arc only to a tolerance, which fails the exact-locus claim of this extension\n",
    ),
    (
        "Mathematical definition",
        "The curve is the branch of the hyperbola x^2/a^2 - y^2/b^2 = 1 on x > 0, parameterised by t in the interval from minus T to T as (a cosh t, b sinh t), placed by an axis placement 2D.\n",
    ),
    (
        "Bounds and evaluation rules",
        "Bounds are the closed form extremes of cosh and sinh over the interval, taken at the end points because both are monotone away from t = 0. Evaluation uses the closed form with no iteration.\n",
    ),
    (
        "Tessellation rules",
        "A tessellator receives the exact curve and subdivides by the sagitta bound, never the other way round, with the subdivision count chosen from the second derivative of the curve.\n",
    ),
    (
        "STEP behaviour",
        "There is no exact representation in the profile; bounded approximation with error reported is written through the rational B-spline form.\n",
    ),
    ("GIS behaviour", "Densified into line work within the reported error budget. fidelity class: display\n"),
    (
        "Conformance fixtures",
        "- hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT\n- hyperbola-arc-dual: DUAL_ORACLE_EQUIVALENT\n",
    ),
];

fn proposal_without(skip: Option<&str>) -> String {
    let mut s = String::from("# Extension proposal: hyperbolic arc\n\nSample text for the checker.\n\n");
    for (h, body) in SECTIONS {
        if Some(h) == skip {
            continue;
        }
        s.push_str(&format!("## {h}\n\n{body}\n"));
    }
    s
}

fn problems(text: &str) -> Vec<ProposalProblem> {
    check_proposal(text).expect_err("this proposal must be refused")
}

#[test]
fn a_complete_proposal_passes_and_reports_its_claims() {
    let s = check_proposal(&proposal_without(None)).unwrap();
    assert!(!s.step_exact && !s.gis_exact);
    assert_eq!(s.fixtures.len(), 2);
    assert_eq!(s.fixtures[0].class, EvidenceClass::KnownTruthExact);
    assert_eq!(s.fixtures[1].class, EvidenceClass::DualOracleEquivalent);
}

#[test]
fn deleting_any_one_required_section_is_caught_on_its_own() {
    let expected = [
        ("Part 42 constructs considered", MissingArtifact::Part42Argument),
        ("Mathematical definition", MissingArtifact::MathematicalDefinition),
        ("Bounds and evaluation rules", MissingArtifact::BoundsAndEvaluation),
        ("Tessellation rules", MissingArtifact::TessellationRules),
        ("STEP behaviour", MissingArtifact::StepBehaviour),
        ("GIS behaviour", MissingArtifact::GisBehaviour),
        ("Conformance fixtures", MissingArtifact::Fixtures),
    ];
    for (heading, artifact) in expected {
        let p = problems(&proposal_without(Some(heading)));
        assert_eq!(p, vec![ProposalProblem::Missing(artifact)], "without '{heading}'");
    }
}

#[test]
fn an_empty_or_placeholder_section_counts_as_missing() {
    let base = proposal_without(None);
    let emptied = base.replace(SECTIONS[1].1, "\n");
    assert_eq!(problems(&emptied), vec![ProposalProblem::Missing(MissingArtifact::MathematicalDefinition)]);
    let todo = base.replace(
        SECTIONS[2].1,
        "TODO: write the bounds rules here and then also the evaluation rules and the derivatives\n",
    );
    assert_eq!(problems(&todo), vec![ProposalProblem::Missing(MissingArtifact::BoundsAndEvaluation)]);
    let thin = base.replace(SECTIONS[3].1, "none\n");
    assert_eq!(problems(&thin), vec![ProposalProblem::Missing(MissingArtifact::TessellationRules)]);
}

#[test]
fn the_part_42_argument_must_name_a_construct_and_say_why_it_fails() {
    let base = proposal_without(None);
    // a section with prose and no construct entry
    let prose = base.replace(SECTIONS[0].1, "No existing construct is suitable for this curve at all.\n");
    assert_eq!(problems(&prose), vec![ProposalProblem::Missing(MissingArtifact::Part42Argument)]);
    // a construct with no reason
    let bare = base.replace(SECTIONS[0].1, "- `hyperbola`:\n");
    assert_eq!(problems(&bare), vec![ProposalProblem::Missing(MissingArtifact::Part42Argument)]);
}

#[test]
fn fixture_classes_are_epic_os_four_and_a_fifth_is_refused() {
    let base = proposal_without(None);
    let fifth = base.replace("hyperbola-arc-dual: DUAL_ORACLE_EQUIVALENT", "hyperbola-arc-dual: SPECULATIVE");
    assert_eq!(problems(&fifth), vec![ProposalProblem::UnknownFixtureClass("SPECULATIVE".into())]);
    for (text, class) in [
        ("KNOWN_TRUTH_EXACT", EvidenceClass::KnownTruthExact),
        ("DUAL_ORACLE_EQUIVALENT", EvidenceClass::DualOracleEquivalent),
        ("REAL_WORLD_DIFFERENTIAL", EvidenceClass::RealWorldDifferential),
        ("QUARANTINED", EvidenceClass::Quarantined),
    ] {
        assert_eq!(EvidenceClass::parse(text), Some(class));
        assert_eq!(class.name(), text);
    }
    assert_eq!(EvidenceClass::parse("known_truth_exact"), None, "the vocabulary is spelled exactly");
}

#[test]
fn quarantined_fixtures_alone_do_not_support_an_extension() {
    let base = proposal_without(None);
    let only = base
        .replace("hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT", "a: QUARANTINED")
        .replace("hyperbola-arc-dual: DUAL_ORACLE_EQUIVALENT", "b: QUARANTINED");
    assert_eq!(problems(&only), vec![ProposalProblem::OnlyQuarantinedFixtures]);
}

#[test]
fn step_may_not_be_recorded_as_exact_without_a_passing_round_trip_fixture() {
    let base = proposal_without(None);
    let exact = base.replace(
        SECTIONS[4].1,
        "The curve is written as an exact HYPERBOLA entity and read back to the same locus.\n",
    );
    assert_eq!(problems(&exact), vec![ProposalProblem::ExactStepWithoutRoundTrip]);
    // with a fixture that records a passing round trip the exact claim is accepted
    let with_proof = exact.replace(
        "hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT",
        "hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT; round-trip: passing",
    );
    let s = check_proposal(&with_proof).unwrap();
    assert!(s.step_exact);
    assert!(s.fixtures[0].round_trip_passing && !s.fixtures[1].round_trip_passing);
}

#[test]
fn gis_behaviour_that_is_not_exact_names_a_fidelity_class() {
    let base = proposal_without(None);
    let unnamed = base.replace("fidelity class: display", "");
    assert_eq!(problems(&unnamed), vec![ProposalProblem::Missing(MissingArtifact::GisBehaviour)]);
}

// ---------------------------------------------------------------- the registration gate

fn extension(id: u32, name: &'static str) -> Capability {
    Capability {
        id: CapabilityId::new(id),
        name,
        class: Class::Extension,
        version: Version::new(0, 1, 0),
        iso_construct: "",
        profile_section: 0,
        requirement: Requirement::Optional,
        step: StepBehaviour::BoundedApproximation,
        gis: GisBehaviour::Densified,
        min_reader: Version::new(1, 0, 0),
        restriction: "sample extension",
        adr: None,
    }
}

fn next_id(r: &Registry) -> u32 {
    r.entries().len() as u32 + r.retired().len() as u32 + 1
}

#[test]
fn a_complete_proposal_registers_an_optional_extension_with_the_next_id() {
    let mut r = Registry::v1();
    let id = next_id(&r);
    let before = r.entries().len();
    r.register_extension(extension(id, "hyperbola_arc"), &proposal_without(None)).unwrap();
    assert_eq!(r.entries().len(), before + 1);
    let e = r.by_name("hyperbola_arc").unwrap();
    assert_eq!((e.class, e.requirement, e.id.get()), (Class::Extension, Requirement::Optional, id));
}

#[test]
fn each_missing_artifact_blocks_registration_individually() {
    for (heading, _) in SECTIONS {
        let mut r = Registry::v1();
        let id = next_id(&r);
        let err = r
            .register_extension(extension(id, "hyperbola_arc"), &proposal_without(Some(heading)))
            .unwrap_err();
        match err {
            ExtensionError::Proposal(p) => assert_eq!(p.len(), 1, "without '{heading}': {p:?}"),
            other => panic!("without '{heading}': {other:?}"),
        }
        assert!(r.by_name("hyperbola_arc").is_none(), "nothing was registered");
    }
}

#[test]
fn an_extension_cannot_be_required_without_an_adr_listed_in_its_entry() {
    let mut r = Registry::v1();
    let id = next_id(&r);
    let mut e = extension(id, "needs_adr");
    e.requirement = Requirement::Required;
    assert_eq!(
        r.register_extension(e.clone(), &proposal_without(None)).unwrap_err(),
        ExtensionError::RequiredWithoutAdr("needs_adr")
    );
    e.adr = Some("");
    assert_eq!(
        r.register_extension(e.clone(), &proposal_without(None)).unwrap_err(),
        ExtensionError::RequiredWithoutAdr("needs_adr")
    );
    e.adr = Some("docs/adr/0099-hyperbolic-arc.md");
    r.register_extension(e, &proposal_without(None)).unwrap();
    assert_eq!(r.by_name("needs_adr").unwrap().requirement, Requirement::Required);
}

#[test]
fn the_entry_must_be_an_extension_with_the_next_id_and_a_new_name() {
    let mut r = Registry::v1();
    let id = next_id(&r);
    let mut geometry = extension(id, "g");
    geometry.class = Class::Geometry;
    assert_eq!(
        r.register_extension(geometry, &proposal_without(None)).unwrap_err(),
        ExtensionError::NotAnExtension
    );
    assert_eq!(
        r.register_extension(extension(id + 1, "skips"), &proposal_without(None)).unwrap_err(),
        ExtensionError::IdNotNext { expected: CapabilityId::new(id) }
    );
    assert_eq!(
        r.register_extension(extension(1, "reuses_an_id"), &proposal_without(None)).unwrap_err(),
        ExtensionError::IdNotNext { expected: CapabilityId::new(id) }
    );
    assert_eq!(
        r.register_extension(extension(id, "line"), &proposal_without(None)).unwrap_err(),
        ExtensionError::DuplicateName("line")
    );
}

#[test]
fn the_registered_step_behaviour_must_agree_with_the_proposal() {
    let mut r = Registry::v1();
    let id = next_id(&r);
    let mut e = extension(id, "claims_exact");
    e.step = StepBehaviour::ExactForm;
    // the proposal says bounded approximation, the entry says exact form
    assert_eq!(
        r.register_extension(e.clone(), &proposal_without(None)).unwrap_err(),
        ExtensionError::StepClaimMismatch("claims_exact")
    );
    // an exact entry needs an exact proposal with a passing fixture
    let exact = proposal_without(None)
        .replace(
            SECTIONS[4].1,
            "The curve is written as an exact HYPERBOLA entity and read back to the same locus.\n",
        )
        .replace(
            "hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT",
            "hyperbola-arc-synthetic: KNOWN_TRUTH_EXACT; round-trip: passing",
        );
    r.register_extension(e, &exact).unwrap();
}

#[test]
fn extension_ids_stay_dense_and_an_extension_names_no_construct() {
    let mut r = Registry::v1();
    let id = next_id(&r);
    r.register_extension(extension(id, "hyperbola_arc"), &proposal_without(None)).unwrap();
    let ids: Vec<u32> = r.entries().iter().map(|c| c.id.get()).collect();
    assert_eq!(ids, (1..=ids.len() as u32).collect::<Vec<_>>());
    let mut named = extension(id + 1, "names_a_construct");
    named.iso_construct = "line";
    let mut r2 = r.clone();
    assert_eq!(
        r2.register_extension(named, &proposal_without(None)).unwrap_err(),
        ExtensionError::ExtensionNamesConstruct("names_a_construct")
    );
}

// ---------------------------------------------------------------- negotiation with extensions

fn reader(version: Version, implemented: &[CapabilityId]) -> Reader {
    Reader { version, implemented: implemented.to_vec() }
}

#[test]
fn an_optional_extension_is_skipped_by_a_reader_without_it_and_a_file_cannot_declare_it_required() {
    let mut r = Registry::v1();
    let id = r.next_id();
    r.register_extension(extension(id.get(), "hyperbola_arc"), &proposal_without(None)).unwrap();
    let rdr = reader(Version::new(1, 0, 0), &[ids::LINE]);
    let optional = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![Declared {
            id,
            requirement: Requirement::Optional,
            version: Version::new(0, 1, 0),
        }],
    };
    assert_eq!(r.negotiate(&rdr, &optional).unwrap().skipped, vec![id]);
    let required = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![Declared {
            id,
            requirement: Requirement::Required,
            version: Version::new(0, 1, 0),
        }],
    };
    assert_eq!(r.negotiate(&rdr, &required).unwrap_err(), NegotiationError::ExtensionDeclaredRequired(id));
}

#[test]
fn a_required_extension_with_an_adr_refuses_a_reader_that_lacks_it() {
    let mut r = Registry::v1();
    let id = r.next_id();
    let mut e = extension(id.get(), "needed");
    e.requirement = Requirement::Required;
    e.adr = Some("docs/adr/0099-needed.md");
    r.register_extension(e, &proposal_without(None)).unwrap();
    let rdr = reader(Version::new(1, 0, 0), &[ids::LINE]);
    let file = Declaration {
        min_reader: Version::new(1, 0, 0),
        capabilities: vec![Declared {
            id,
            requirement: Requirement::Optional,
            version: Version::new(0, 1, 0),
        }],
    };
    assert_eq!(
        r.negotiate(&rdr, &file).unwrap_err(),
        NegotiationError::MissingRequired { id, name: Some("needed") }
    );
    assert!(r.negotiate(&reader(Version::new(1, 0, 0), &[id]), &file).is_ok());
}

#[test]
fn registry_new_refuses_a_required_extension_without_an_adr_too() {
    let mut e = extension(1, "x");
    e.requirement = Requirement::Required;
    assert_eq!(
        Registry::new(vec![e.clone()], vec![]).unwrap_err(),
        RegistryError::ExtensionRequiredWithoutAdr("x")
    );
    e.adr = Some("docs/adr/0099-x.md");
    assert!(Registry::new(vec![e], vec![]).is_ok());
    let mut named = extension(1, "y");
    named.iso_construct = "line";
    let r = Registry::new(vec![named], vec![]).unwrap();
    assert_eq!(r.validate_against_profile(&[]).unwrap_err(), RegistryError::ExtensionNamesConstruct("y"));
}
