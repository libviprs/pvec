//! Issue #41: the five gates, the runner and the report schema.

mod common;

use pvec_capabilities::EvidenceClass as C;
use pvec_conformance::gates::{Gate, GateVerdict as V, Runner};
use pvec_conformance::geometric::{Break, Evaluator, Plan, Source};
use pvec_conformance::structural::{Family, Knot, Structure};
use pvec_conformance::{ConformanceError, check_report, geometric, report, structural};
use pvec_numeric::Side;
use std::f64::consts::TAU;

const ALL_CLASSES: [C; 4] =
    [C::KnownTruthExact, C::DualOracleEquivalent, C::RealWorldDifferential, C::Quarantined];

fn nurbs() -> Structure {
    let k = |value, multiplicity| Knot { value, multiplicity };
    Structure {
        family: Family::Nurbs,
        degree: 2,
        control_points: vec![
            [0.0, 0.0, 0.0],
            [1.0, 1.5, 0.0],
            [2.5, 2.0, 0.0],
            [3.5, 0.5, 0.0],
            [5.0, 1.0, 0.0],
        ],
        knots: vec![k(0.0, 3), k(0.4, 1), k(0.7, 1), k(1.0, 3)],
        weights: Some(vec![1.0, 0.6, 1.7, 0.9, 1.0]),
        periodic: false,
        closed_by_endpoint: false,
        closed_by_knots: false,
        domain: [0.0, 1.0],
        reversed: false,
    }
}

struct Circle(bool);
impl Evaluator for Circle {
    fn source(&self) -> Source {
        if self.0 { Source::Own } else { Source::Independent }
    }
    fn domain(&self) -> [f64; 2] {
        [0.0, TAU]
    }
    fn breaks(&self) -> Vec<Break> {
        vec![]
    }
    fn point(&self, t: f64) -> [f64; 3] {
        [t.cos(), t.sin(), 0.0]
    }
    fn tangent(&self, t: f64, _: Side) -> Option<[f64; 3]> {
        Some([-t.sin(), t.cos(), 0.0])
    }
    fn curvature(&self, _: f64, _: Side) -> Option<f64> {
        Some(1.0)
    }
    fn arc_length(&self) -> f64 {
        TAU
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        None
    }
}

fn geometric_report(class: C) -> geometric::Report {
    geometric::compare(
        "circle-001",
        class,
        &Circle(false),
        &Circle(true),
        Plan { seed: 3, samples: 16, budget: 17 },
    )
    .unwrap()
}

#[test]
fn issue_41_there_are_five_gates_and_each_lists_the_classes_that_may_satisfy_it() {
    assert_eq!(Gate::ALL.len(), 5);
    let names: Vec<&str> = Gate::ALL.iter().map(|g| g.name()).collect();
    assert_eq!(
        names,
        ["structural exact", "geometric exact", "display approximate", "format valid", "forward compatible"]
    );
    assert_eq!(Gate::StructuralExact.classes(), [C::KnownTruthExact, C::DualOracleEquivalent]);
    assert_eq!(Gate::GeometricExact.classes(), [C::KnownTruthExact, C::DualOracleEquivalent]);
    for gate in Gate::ALL {
        assert!(
            !gate.classes().contains(&C::Quarantined),
            "{}: a quarantined fixture satisfies no gate",
            gate.name()
        );
        assert!(!gate.classes().is_empty());
    }
    assert!(!Gate::StructuralExact.classes().contains(&C::RealWorldDifferential));
    assert!(!Gate::GeometricExact.classes().contains(&C::RealWorldDifferential));
}

#[test]
fn issue_41_the_runner_refuses_a_gate_the_class_cannot_satisfy() {
    let mut r = Runner::new();
    // Claiming structural exactness for a real-world fixture is refused, by claim and by report.
    match r.claim(Gate::StructuralExact, "plan-a", C::RealWorldDifferential, V::Pass, &[]) {
        Err(ConformanceError::GateRefused { gate, fixture, class }) => {
            assert_eq!(
                (gate.as_str(), fixture.as_str(), class.as_str()),
                ("structural exact", "plan-a", "REAL_WORLD_DIFFERENTIAL")
            );
        }
        other => panic!("accepted: {other:?}"),
    }
    let s = nurbs();
    let rwd = structural::compare("plan-a", C::RealWorldDifferential, &s, &s);
    assert!(matches!(r.structural(&rwd), Err(ConformanceError::GateRefused { .. })));
    assert!(matches!(
        r.geometric(&geometric_report(C::RealWorldDifferential)),
        Err(ConformanceError::GateRefused { .. })
    ));
    assert!(r.rows().is_empty(), "a refused claim leaves no row");
    for gate in Gate::ALL {
        assert!(
            r.claim(gate, "odd", C::Quarantined, V::Pass, &[]).is_err(),
            "{} took a quarantined fixture",
            gate.name()
        );
        for class in ALL_CLASSES {
            let allowed = gate.classes().contains(&class);
            assert_eq!(
                r.claim(gate, "x", class, V::Pass, &[]).is_ok(),
                allowed,
                "{} for {class:?}",
                gate.name()
            );
        }
    }
}

#[test]
fn issue_41_a_display_result_cannot_be_rendered_with_the_word_exact() {
    for verdict in [V::Pass, V::Fail, V::Routed] {
        let word = Gate::DisplayApproximate.word(verdict);
        assert!(!word.to_lowercase().contains("exact"), "{word}");
    }
    assert!(Gate::StructuralExact.word(V::Pass).contains("exact"), "the exact gates may say exact");
    for class in [C::KnownTruthExact, C::DualOracleEquivalent, C::RealWorldDifferential] {
        for (screen, world) in
            [((0.2, 0.5), (0.001, 0.01)), ((0.0, 0.5), (0.0, 0.01)), ((0.9, 0.5), (0.5, 0.01))]
        {
            let mut r = Runner::new();
            r.display("circle-001", class, screen, world).unwrap();
            // Everything but the class token and the fixture id, which are not the formatter's words.
            let text = r.render().replace(class.name(), "").replace("circle-001", "").to_lowercase();
            assert!(!text.contains("exact"), "{text}");
            assert!(text.contains("approximate") && text.contains("budget"), "{text}");
        }
    }
    let mut r = Runner::new();
    r.display("circle-001", C::KnownTruthExact, (0.9, 0.5), (0.001, 0.01)).unwrap();
    assert_eq!(r.rows()[0].verdict, V::Fail, "an error over its budget fails");
    let names: Vec<&str> = r.rows()[0].numbers.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["screen_error", "screen_budget", "world_error", "world_budget"]);
}

#[test]
fn issue_41_the_gates_are_independent_a_fixture_can_pass_geometric_and_fail_structural() {
    let mut r = Runner::new();
    let mut moved = nurbs();
    moved.knots[1].multiplicity = 2;
    r.structural(&structural::compare("circle-001", C::KnownTruthExact, &nurbs(), &moved)).unwrap();
    r.geometric(&geometric_report(C::KnownTruthExact)).unwrap();
    assert_eq!(r.verdicts("circle-001"), [(Gate::StructuralExact, V::Fail), (Gate::GeometricExact, V::Pass)]);
    let text = r.render();
    assert_eq!(text.lines().count(), 2, "{text}");
    assert!(text.lines().any(|l| l.contains("structural exact: FAIL")));
    assert!(text.lines().any(|l| l.contains("geometric exact: PASS")));
    check_report(&text).expect("the runner's lines pass the report format test");
    // A reparameterised fixture is routed, which is neither of the two.
    let mut reparameterised = nurbs();
    reparameterised.domain = [3.0, 5.0];
    let mut r = Runner::new();
    r.structural(&structural::compare("spline-001", C::DualOracleEquivalent, &nurbs(), &reparameterised))
        .unwrap();
    assert_eq!(r.verdicts("spline-001"), [(Gate::StructuralExact, V::Routed)]);
    assert!(r.render().contains("ROUTED"));
}

#[test]
fn issue_41_every_row_carries_gate_class_and_numbers() {
    let mut r = Runner::new();
    r.structural(&structural::compare("spline-001", C::KnownTruthExact, &nurbs(), &nurbs())).unwrap();
    r.geometric(&geometric_report(C::DualOracleEquivalent)).unwrap();
    let names = |i: usize| -> Vec<&str> { r.rows()[i].numbers.iter().map(|(n, _)| n.as_str()).collect() };
    for want in ["degree", "knot_count", "control_point_count"] {
        assert!(names(0).contains(&want), "{want}: {:?}", names(0));
    }
    for want in ["max_deviation", "worst_parameter", "tolerance", "arc_length_deviation"] {
        assert!(names(1).contains(&want), "{want}: {:?}", names(1));
    }
    for line in r.render().lines() {
        assert!(ALL_CLASSES.iter().any(|c| line.contains(c.name())), "{line}");
        assert!(Gate::ALL.iter().any(|g| line.contains(g.name())), "{line}");
    }
}

#[test]
fn issue_41_the_report_schema_is_versioned_and_an_added_field_does_not_break_an_older_consumer() {
    let mut r = Runner::new();
    r.structural(&structural::compare("spline-001", C::KnownTruthExact, &nurbs(), &nurbs())).unwrap();
    r.geometric(&geometric_report(C::DualOracleEquivalent)).unwrap();
    r.claim(Gate::FormatValid, "container-001", C::KnownTruthExact, V::Pass, &[("cases", 12.0)]).unwrap();
    let json = report::write(&r);
    assert!(
        json.contains(r#""schema_version": "pvec/conformance-report/1""#)
            || json.contains(r#""schema_version":"pvec/conformance-report/1""#),
        "{json}"
    );

    let rows = report::read(&json).expect("a version 1 report reads");
    assert_eq!(rows.len(), 3);
    for (read, written) in rows.iter().zip(r.rows()) {
        assert_eq!(
            (read.gate, &read.fixture, read.class, read.verdict),
            (written.gate, &written.fixture, written.class, written.verdict)
        );
        assert_eq!(read.numbers.len(), written.numbers.len());
        for ((rn, rv), (wn, wv)) in read.numbers.iter().zip(&written.numbers) {
            assert_eq!(rn, wn);
            assert_eq!(rv.to_bits(), wv.to_bits(), "{wn} lost bits");
        }
    }

    // A later minor revision adds a field to the document and to every row; the old consumer reads on.
    let extended = json
        .replacen("{", r#"{"generator":{"name":"later","build":7},"#, 1)
        .replace(r#""gate""#, r#""added_in_a_later_revision":[1,2,3],"gate""#);
    assert_ne!(extended, json);
    assert_eq!(report::read(&extended).expect("an added field must not break an older consumer"), rows);

    // A new major version is refused rather than half-read, and so is a row missing a field.
    let v2 = json.replace("pvec/conformance-report/1", "pvec/conformance-report/2");
    assert!(matches!(report::read(&v2), Err(ConformanceError::BadReport(_))));
    let headless = json.replacen(r#""fixture""#, r#""fixtur""#, 1);
    assert!(matches!(report::read(&headless), Err(ConformanceError::BadReport(_))));
}

#[test]
fn issue_41_the_gate_module_writes_no_tolerance() {
    common::assert_no_tolerance_literal("gates.rs", false);
    common::assert_no_tolerance_literal("report.rs", false);
}

fn document() -> String {
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CONFORMANCE.md"))
        .expect("docs/CONFORMANCE.md exists")
}

#[test]
fn issue_41_the_document_defines_the_five_gates_with_the_classes_the_code_uses() {
    let doc = document();
    for gate in Gate::ALL {
        let row = doc
            .lines()
            .find(|l| l.starts_with('|') && l.to_lowercase().contains(&format!("| {} |", gate.name())))
            .unwrap_or_else(|| panic!("no table row for `{}`", gate.name()));
        for class in ALL_CLASSES {
            assert_eq!(
                row.contains(class.name()),
                gate.classes().contains(&class),
                "{}: {}",
                gate.name(),
                class.name()
            );
        }
    }
    for needle in
        ["required evidence", "required tests", "required report fields", "pvec/conformance-report/1"]
    {
        assert!(doc.to_lowercase().contains(needle), "the document never says `{needle}`");
    }
}

#[test]
fn issue_41_the_document_cites_the_profile_and_the_policy_and_introduces_no_tolerance() {
    let doc = document();
    assert!(doc.contains("docs/ISO_PROFILE.md") || doc.contains("ISO_PROFILE.md"));
    assert!(doc.contains("NUMERIC_POLICY.md"));
    // A tolerance is a number with an exponent or a run of leading zeros after the point.
    let b: Vec<char> = doc.chars().collect();
    for i in 1..b.len().saturating_sub(1) {
        let exponent = (b[i] == 'e' || b[i] == 'E')
            && b[i - 1].is_ascii_digit()
            && (b[i + 1] == '-' || b[i + 1].is_ascii_digit());
        assert!(
            !exponent,
            "an exponent-form number near `{}`",
            &doc[i.saturating_sub(10)..(i + 10).min(doc.len())]
        );
    }
    assert!(!doc.contains("0.00"), "a tolerance-shaped decimal");
    assert!(
        doc.to_lowercase().contains("introduces no tolerance")
            || doc.to_lowercase().contains("no new tolerance")
    );
}

#[test]
fn issue_41_the_claim_is_the_approved_sentence_and_nothing_stronger() {
    // The wording below is built at runtime and never written out, so the repository scan for
    // forbidden claims does not trip over the samples that prove the check bites.
    let doc = document();
    assert!(pvec_checks::forbidden_claims(&doc).is_empty(), "{:?}", pvec_checks::forbidden_claims(&doc));
    assert!(
        doc.contains("aligned to named ISO 10303-42:2025 constructs and validated through STEP interchange"),
        "the approved claim is missing"
    );
    let (stem_a, stem_b) = (["conf", "orms to"].concat(), ["compl", "iant with"].concat());
    for bad in [format!("PVec {stem_a} ISO 10303-42."), format!("PVec is {stem_b} ISO 10303-42.")] {
        assert!(!pvec_checks::forbidden_claims(&bad).is_empty(), "{bad}");
    }
}
