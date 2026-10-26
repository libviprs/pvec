//! Issue #38: structural exactness. Each test breaks one thing in an otherwise equal pair and
//! checks that exactly the right thing turns red.

mod common;

use pvec_capabilities::{EvidenceClass as C, Registry as Capabilities};
use pvec_conformance::structural::{Family, Knot, Report, Structural, Structure, compare, render};
use pvec_conformance::{Assertion, check_report};
use pvec_corpus::expected::EntityClass;
use pvec_corpus::{Bundle, FixtureRegistry};

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

fn run(class: C, a: &Structure, b: &Structure) -> Report {
    compare("nurbs-d2-offconic-001", class, a, b)
}

fn red_fields(r: &Report) -> Vec<&str> {
    r.rows.iter().filter(|row| !row.equal).map(|row| row.field.as_str()).collect()
}

#[test]
fn issue_38_an_identical_pair_is_exact_and_every_field_is_a_row() {
    let r = run(C::KnownTruthExact, &nurbs(), &nurbs());
    assert_eq!(r.result, Structural::Exact);
    assert!(red_fields(&r).is_empty());
    let fields: Vec<&str> = r.rows.iter().map(|row| row.field.as_str()).collect();
    for want in [
        "degree",
        "control_points.count",
        "control_points[4]",
        "knots.count",
        "knots[3].value",
        "knots[3].multiplicity",
        "weights.count",
        "weights[2]",
        "periodic",
        "closed_by_endpoint",
        "closed_by_knots",
        "domain.start",
        "domain.end",
    ] {
        assert!(fields.contains(&want), "no row for {want}: {fields:?}");
    }
}

#[test]
fn issue_38_an_affine_knot_reparameterisation_is_routed_not_exact_and_not_failed() {
    let mut b = nurbs();
    for k in &mut b.knots {
        k.value = k.value * 2.0 + 3.0;
    }
    b.domain = [3.0, 5.0];
    let r = run(C::KnownTruthExact, &nurbs(), &b);
    assert_eq!(r.result, Structural::RoutedToGeometric);
    assert_ne!(r.result, Structural::Exact);
    assert_ne!(r.result, Structural::Differs);
    assert!(r.rows.is_empty(), "a routed fixture records no failed field");
    assert!(r.reasons.iter().any(|m| m.contains("knot-vector normalisation")), "{:?}", r.reasons);
    let text = render(&r);
    assert!(text.contains("geometric") && !text.contains("FAIL"), "{text}");
}

#[test]
fn issue_38_the_other_preconditions_route_too() {
    let mut reversed = nurbs();
    reversed.reversed = true;
    let r = run(C::KnownTruthExact, &nurbs(), &reversed);
    assert_eq!(r.result, Structural::RoutedToGeometric);
    assert!(r.reasons.iter().any(|m| m.contains("orientation")), "{:?}", r.reasons);

    let mut plain = nurbs();
    plain.family = Family::BSpline;
    plain.weights = None;
    let r = run(C::KnownTruthExact, &nurbs(), &plain);
    assert_eq!(r.result, Structural::RoutedToGeometric);
    assert!(r.reasons.iter().any(|m| m.contains("family")), "{:?}", r.reasons);

    let mut unscalable = nurbs();
    unscalable.weights.as_mut().unwrap()[1] = 0.0;
    let r = run(C::KnownTruthExact, &nurbs(), &unscalable);
    assert_eq!(r.result, Structural::RoutedToGeometric);
    assert!(r.reasons.iter().any(|m| m.contains("weight scaling")), "{:?}", r.reasons);
}

#[test]
fn issue_38_weights_differing_by_a_common_factor_are_equal_and_one_scaled_weight_is_red() {
    for factor in [2.5, 0.25, 7.0, 1024.0] {
        let mut b = nurbs();
        for w in b.weights.as_mut().unwrap() {
            *w *= factor;
        }
        let r = run(C::KnownTruthExact, &nurbs(), &b);
        assert_eq!(r.result, Structural::Exact, "factor {factor}: {:?}", red_fields(&r));
    }
    for i in 0..5 {
        let mut b = nurbs();
        b.weights.as_mut().unwrap()[i] *= 1.1;
        let r = run(C::KnownTruthExact, &nurbs(), &b);
        assert_eq!(r.result, Structural::Differs, "weight {i}");
        // Scaling a weight that is not the normaliser moves exactly that weight; scaling the largest
        // moves the normaliser, and every other weight with it, which is still a failure.
        if i != 2 {
            assert_eq!(red_fields(&r), [format!("weights[{i}]")], "weight {i}");
        } else {
            assert!(red_fields(&r).iter().all(|f| f.starts_with("weights[")), "{:?}", red_fields(&r));
        }
    }
}

#[test]
fn issue_38_one_changed_knot_multiplicity_turns_exactly_one_row_red_and_names_it() {
    let mut b = nurbs();
    b.knots[1].multiplicity = 2;
    let r = run(C::KnownTruthExact, &nurbs(), &b);
    assert_eq!(r.result, Structural::Differs);
    assert_eq!(red_fields(&r), ["knots[1].multiplicity"]);
    let text = render(&r);
    assert!(text.lines().filter(|l| l.contains(": FAIL")).count() == 1, "{text}");
    assert!(text.contains("knots[1].multiplicity"), "{text}");
}

#[test]
fn issue_38_each_field_can_turn_its_own_row_red() {
    type Edit = fn(&mut Structure);
    let edits: [(&str, Edit); 8] = [
        ("degree", |s| s.degree = 3),
        ("control_points[2]", |s| s.control_points[2][1] += 0.5),
        ("knots[2].value", |s| s.knots[2].value = 0.75),
        ("periodic", |s| s.periodic = true),
        ("closed_by_endpoint", |s| s.closed_by_endpoint = true),
        ("closed_by_knots", |s| s.closed_by_knots = true),
        ("control_points.count", |s| s.control_points.push([6.0, 1.0, 0.0])),
        ("knots.count", |s| s.knots.push(Knot { value: 1.5, multiplicity: 1 })),
    ];
    for (field, edit) in edits {
        let mut b = nurbs();
        edit(&mut b);
        let r = run(C::KnownTruthExact, &nurbs(), &b);
        assert_eq!(r.result, Structural::Differs, "{field}");
        assert!(red_fields(&r).contains(&field), "{field} not red: {:?}", red_fields(&r));
    }
}

#[test]
fn issue_38_a_pass_means_what_the_evidence_class_allows_it_to_mean() {
    let cases = [
        (C::KnownTruthExact, Some(Assertion::SourceIntent), "source intent"),
        (C::DualOracleEquivalent, Some(Assertion::ConverterAgreement), "converter agreement"),
        (C::RealWorldDifferential, Some(Assertion::RegressionStability), "regression stability"),
        (C::Quarantined, None, "not counted"),
    ];
    for (class, assertion, words) in cases {
        let r = run(class, &nurbs(), &nurbs());
        assert_eq!(r.assertion, assertion, "{class:?}");
        let text = render(&r);
        assert!(text.contains(words), "{class:?}: {text}");
        assert!(text.contains(class.name()), "{text}");
        check_report(&text).unwrap_or_else(|e| panic!("{e}"));
    }
    let real = render(&run(C::RealWorldDifferential, &nurbs(), &nurbs()));
    assert!(!real.contains("source intent") && !real.contains("exactness"), "{real}");
}

#[test]
fn issue_38_every_row_records_degree_knot_count_control_point_count_rule_and_class() {
    let mut b = nurbs();
    b.knots[1].multiplicity = 2;
    b.control_points[0][0] = 9.0;
    let r = run(C::DualOracleEquivalent, &nurbs(), &b);
    assert!(!r.rows.is_empty());
    for row in &r.rows {
        assert_eq!((row.degree, row.knot_count, row.control_point_count), (2, 8, 5), "{}", row.field);
        assert!(row.rule.starts_with("NP-") || row.rule == "exact", "{}: {}", row.field, row.rule);
    }
    let text = render(&r);
    for line in text.lines() {
        for needle in ["degree=2", "knots=8", "control_points=5", "rule=", "DUAL_ORACLE_EQUIVALENT"] {
            assert!(line.contains(needle), "`{needle}` missing from {line}");
        }
    }
}

#[test]
fn issue_38_the_module_writes_no_tolerance_and_calls_the_policy() {
    common::assert_no_tolerance_literal("structural.rs");
}

#[test]
fn issue_38_the_vendored_spline_fixtures_read_and_match_themselves() {
    let bundle = Bundle::vendored().unwrap();
    let profile =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/ISO_PROFILE.md")).unwrap();
    let corpus = FixtureRegistry::load(&bundle, &Capabilities::v1(), &profile).unwrap();
    let mut seen = 0;
    for f in corpus.fixtures() {
        let Some(expected) = &f.expected else { continue };
        for e in &expected.entities {
            if !matches!(e.class, EntityClass::Bezier | EntityClass::Bspline | EntityClass::Nurbs) {
                continue;
            }
            let s = Structure::from_entity(&e.node).unwrap_or_else(|m| panic!("{}: {m}", f.id));
            let r = compare(&f.id, f.class, &s, &s);
            assert_eq!(r.result, Structural::Exact, "{}", f.id);
            seen += 1;
        }
    }
    assert!(seen >= 6, "only {seen} spline entities in r0-open");
    let weights = corpus.get("nurbs-d2-offconic-001").unwrap().expected.as_ref().unwrap();
    let s = Structure::from_entity(&weights.entities[0].node).unwrap();
    assert_eq!((s.family, s.degree, s.knots.len(), s.control_points.len()), (Family::Nurbs, 2, 4, 5));
    assert_eq!(s.weights.as_deref().map(<[f64]>::len), Some(5));
}
