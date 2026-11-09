//! Issue #36: the evidence classes in the harness. The class is EPIC O's, read from the manifest;
//! each test below turns red for an implementation that gets the rule wrong.

use pvec_capabilities::{EvidenceClass as C, Registry as Capabilities};
use pvec_corpus::{Bundle, FixtureRegistry};
use pvec_harness::{
    Assertion as A, ConformanceError, Outcome, Registry, check_report, may_assert, render, tally,
};

fn registry() -> Registry {
    Registry::from_manifest(&[
        ("known-line", "KNOWN_TRUTH_EXACT"),
        ("dual-line", "DUAL_ORACLE_EQUIVALENT"),
        ("plan-a", "REAL_WORLD_DIFFERENTIAL"),
        ("odd-hatch", "QUARANTINED"),
    ])
    .expect("four classes register")
}

#[test]
fn issue_36_the_gate_map_lets_each_class_support_exactly_its_assertion() {
    let rows = [
        (C::KnownTruthExact, [true, false, false]),
        (C::DualOracleEquivalent, [false, true, false]),
        (C::RealWorldDifferential, [false, false, true]),
        (C::Quarantined, [false, false, false]),
    ];
    for (class, allowed) in rows {
        for (assertion, want) in
            [A::SourceIntent, A::ConverterAgreement, A::RegressionStability].into_iter().zip(allowed)
        {
            assert_eq!(may_assert(class, assertion), want, "{class:?} {assertion:?}");
        }
    }
}

#[test]
fn issue_36_the_class_is_read_only_and_an_upgrade_is_refused() {
    let r = registry();
    assert_eq!(r.class("plan-a"), Some(C::RealWorldDifferential));
    // Upgrading the real-world fixture to dual-oracle inside this repository is refused...
    match r.declare_class("plan-a", C::DualOracleEquivalent) {
        Err(ConformanceError::ClassIsReadOnly { fixture, manifest, claimed }) => {
            assert_eq!(
                (fixture.as_str(), manifest.as_str(), claimed.as_str()),
                ("plan-a", "REAL_WORLD_DIFFERENTIAL", "DUAL_ORACLE_EQUIVALENT")
            );
        }
        other => panic!("upgrade accepted: {other:?}"),
    }
    // ... and so is every other change, in both directions, while the true class is accepted.
    for (id, truth) in [
        ("known-line", C::KnownTruthExact),
        ("dual-line", C::DualOracleEquivalent),
        ("plan-a", C::RealWorldDifferential),
        ("odd-hatch", C::Quarantined),
    ] {
        for claimed in [C::KnownTruthExact, C::DualOracleEquivalent, C::RealWorldDifferential, C::Quarantined]
        {
            assert_eq!(r.declare_class(id, claimed).is_ok(), claimed == truth, "{id} as {claimed:?}");
        }
    }
    assert!(matches!(r.declare_class("nope", C::Quarantined), Err(ConformanceError::UnknownFixture(_))));
}

#[test]
fn issue_36_source_intent_on_a_real_world_fixture_is_rejected_at_registration() {
    let mut r = registry();
    match r.register("plan-a", A::SourceIntent) {
        Err(ConformanceError::AssertionNotSupported { fixture, class, assertion }) => {
            assert_eq!((fixture.as_str(), class.as_str()), ("plan-a", "REAL_WORLD_DIFFERENTIAL"));
            assert!(assertion.contains("source intent"));
        }
        other => panic!("registered: {other:?}"),
    }
    assert!(r.register("plan-a", A::RegressionStability).is_ok());
    assert!(r.register("dual-line", A::SourceIntent).is_err(), "agreement is not intent");
    assert!(r.register("known-line", A::SourceIntent).is_ok());
    assert!(r.register("dual-line", A::ConverterAgreement).is_ok());
    assert!(
        r.register("odd-hatch", A::RegressionStability).is_err(),
        "a quarantined fixture asserts nothing"
    );
    // A verdict needs a registered assertion.
    assert!(matches!(
        r.judge("known-line", A::ConverterAgreement, Outcome::Pass),
        Err(ConformanceError::NotRegistered { .. })
    ));
}

#[test]
fn issue_36_a_quarantined_fixture_runs_but_contributes_no_pass_and_no_failure() {
    let mut r = registry();
    r.register("known-line", A::SourceIntent).unwrap();
    // A quarantined fixture may still be executed; register the run through the one door that exists.
    let q_pass = r.judge_quarantined("odd-hatch", Outcome::Pass).unwrap();
    let q_fail = r.judge_quarantined("odd-hatch", Outcome::Fail).unwrap();
    assert!(!q_pass.counts && !q_fail.counts);
    let ok = r.judge("known-line", A::SourceIntent, Outcome::Pass).unwrap();
    assert!(ok.counts);
    assert_eq!(tally(&[ok.clone(), q_pass.clone(), q_fail.clone()]).passed, 1);
    assert_eq!(tally(&[q_pass.clone(), q_fail]).passed, 0, "quarantined passes must not satisfy a gate");
    assert_eq!(tally(&[q_pass]).failed, 0);
    assert_eq!(tally(&[ok]).uncounted, 0);
    assert!(
        r.judge_quarantined("known-line", Outcome::Pass).is_err(),
        "only a quarantined fixture takes that door"
    );
}

#[test]
fn issue_36_every_verdict_row_carries_its_class_and_a_line_without_one_fails_the_format_test() {
    let mut r = registry();
    r.register("known-line", A::SourceIntent).unwrap();
    r.register("dual-line", A::ConverterAgreement).unwrap();
    r.register("plan-a", A::RegressionStability).unwrap();
    let rows: Vec<String> = [
        r.judge("known-line", A::SourceIntent, Outcome::Pass).unwrap(),
        r.judge("dual-line", A::ConverterAgreement, Outcome::Fail).unwrap(),
        r.judge("plan-a", A::RegressionStability, Outcome::Pass).unwrap(),
        r.judge_quarantined("odd-hatch", Outcome::Pass).unwrap(),
    ]
    .iter()
    .map(render)
    .collect();
    for (row, class) in rows.iter().zip([
        "KNOWN_TRUTH_EXACT",
        "DUAL_ORACLE_EQUIVALENT",
        "REAL_WORLD_DIFFERENTIAL",
        "QUARANTINED",
    ]) {
        assert!(row.contains(class), "{row}");
    }
    check_report(&rows.join("\n")).expect("generated rows pass the format test");
    assert!(rows[2].contains("regression stability") && !rows[2].contains("source intent"), "{}", rows[2]);
    assert!(check_report("known-line: PASS").is_err(), "a verdict without a class");
    assert!(check_report("dual-line source intent: FAIL").is_err(), "a failure without a class");
    assert!(check_report("known-line [KNOWN_TRUTH_EXACT] source intent: PASS").is_ok());
}

#[test]
fn issue_36_generated_text_never_calls_converter_agreement_source_truth() {
    for bad in [
        "dual-line [DUAL_ORACLE_EQUIVALENT] converter agreement: PASS, so the source truth is confirmed",
        "dual-line [DUAL_ORACLE_EQUIVALENT] both converters agree: ground truth",
        "dual-line [DUAL_ORACLE_EQUIVALENT] agreement proves the original geometry",
    ] {
        assert!(check_report(bad).is_err(), "{bad}");
    }
    let mut r = registry();
    r.register("dual-line", A::ConverterAgreement).unwrap();
    let row = render(&r.judge("dual-line", A::ConverterAgreement, Outcome::Pass).unwrap());
    assert!(row.contains("converter agreement"), "{row}");
    check_report(&row).unwrap();
}

#[test]
fn issue_36_a_fifth_class_fails_validation() {
    for text in ["SYNTHETIC_GOLD", "known_truth_exact", "", "KNOWN_TRUTH_EXACT "] {
        match Registry::from_manifest(&[("x", text)]) {
            Err(ConformanceError::Unclassified { fixture, found }) => {
                assert_eq!((fixture.as_str(), found.as_str()), ("x", text))
            }
            other => panic!("{text:?} accepted: {other:?}"),
        }
    }
    // The vocabulary is EPIC O's: this crate spells no class of its own. Every SCREAMING_SNAKE literal
    // in the sources that looks like a class must be one of the four.
    let four = ["KNOWN_TRUTH_EXACT", "DUAL_ORACLE_EQUIVALENT", "REAL_WORLD_DIFFERENTIAL", "QUARANTINED"];
    let src = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/src")).unwrap();
    for entry in src.flatten() {
        let text = std::fs::read_to_string(entry.path()).unwrap();
        for lit in text.split('"').skip(1).step_by(2) {
            let classy = !lit.is_empty()
                && lit.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                && ["TRUTH", "ORACLE", "DIFFERENTIAL", "QUARANTINE", "EXACT", "EQUIVALENT"]
                    .iter()
                    .any(|w| lit.contains(w));
            assert!(
                !classy || four.contains(&lit),
                "{} spells a class of its own: {lit}",
                entry.path().display()
            );
        }
    }
}

#[test]
fn issue_36_the_vendored_corpus_registers_with_the_classes_the_manifest_gives() {
    let bundle = Bundle::vendored().unwrap();
    let profile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ISO_PROFILE.md")).unwrap();
    let corpus = FixtureRegistry::load(&bundle, &Capabilities::v1(), &profile).unwrap();
    let r = Registry::from_corpus(&corpus);
    for f in corpus.fixtures() {
        assert_eq!(r.class(&f.id), Some(f.class), "{}", f.id);
    }
    // r0-open is reader-mapping evidence only: its fixtures are known truth and nothing else.
    let mut r = r;
    let first = corpus.fixtures().iter().find(|f| f.class == C::KnownTruthExact).unwrap();
    assert!(r.register(&first.id, A::SourceIntent).is_ok());
}
