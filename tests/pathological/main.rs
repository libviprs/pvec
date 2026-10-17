//! The pathological suite (issue P1.2). It is written before any evaluator exists.
//!
//! Two kinds of test live here:
//!
//! * policy cases, which run against `pvec-numeric` today and must pass, and which must change
//!   their answer if any tolerance is widened by an order of magnitude;
//! * evaluator probes, which cannot pass until the evaluator lands (P3.1). They are `#[ignore]`d so
//!   the ordinary gate stays usable, and `tools/gate.sh` runs them with `--ignored` and fails if
//!   they ever go green without this file being updated, so they cannot silently rot.

mod policy_cases;
mod probes;

use policy_cases::cases;
use probes::*;
use pvec_numeric::*;

#[test]
fn every_case_names_a_rule_that_the_policy_document_defines() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/NUMERIC_POLICY.md"))
        .expect("docs/NUMERIC_POLICY.md exists");
    let rules: Vec<&str> = cases().iter().map(|c| c.rule).chain(probes().iter().map(|p| p.rule)).collect();
    for r in rules {
        assert!(r.starts_with("NP-"), "{r} is not a rule id");
        assert!(doc.contains(&format!("### {r}")), "{r} has no section in NUMERIC_POLICY.md");
    }
}

#[test]
fn case_ids_are_unique() {
    let mut ids: Vec<&str> = cases().iter().map(|c| c.id).collect();
    ids.extend(probes().iter().map(|p| p.id));
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn v1_policy_passes_every_policy_case() {
    let failed: Vec<String> = cases()
        .iter()
        .filter(|c| !(c.run)(&Policy::V1))
        .map(|c| format!("{} [{}]: {}", c.id, c.rule, c.what))
        .collect();
    assert!(failed.is_empty(), "policy cases failing under V1:\n{}", failed.join("\n"));
}

/// Widening any one tolerance by a factor of ten must turn at least one case red.
#[test]
fn widening_each_tolerance_by_one_order_of_magnitude_turns_a_case_red() {
    let v = Policy::V1;
    let widened = [
        ("abs", Policy { abs: v.abs * 10.0, ..v }),
        ("rel", Policy { rel: v.rel * 10.0, ..v }),
        ("ulp", Policy { ulp: v.ulp * 10, ..v }),
        ("weight_floor", Policy { weight_floor: v.weight_floor * 10.0, ..v }),
        ("axis_ratio_max", Policy { axis_ratio_max: v.axis_ratio_max * 10.0, ..v }),
        ("all", v.widened_for_mutation_test(10.0)),
    ];
    for (name, p) in widened {
        let flipped: Vec<&str> = cases().iter().filter(|c| !(c.run)(&p)).map(|c| c.id).collect();
        assert!(!flipped.is_empty(), "widening {name} by 10x turned no case red");
        println!("widening {name} x10 turns red: {}", flipped.join(", "));
    }
}

/// Narrowing a tolerance by ten must also be noticed, so the constants are pinned from both sides.
#[test]
fn narrowing_each_tolerance_by_one_order_of_magnitude_turns_a_case_red() {
    let v = Policy::V1;
    let narrowed = [
        ("abs", Policy { abs: v.abs / 10.0, ..v }),
        ("rel", Policy { rel: v.rel / 10.0, ..v }),
        ("ulp", Policy { ulp: v.ulp / 10, ..v }),
        ("weight_floor", Policy { weight_floor: v.weight_floor / 10.0, ..v }),
        ("axis_ratio_max", Policy { axis_ratio_max: v.axis_ratio_max / 10.0, ..v }),
    ];
    for (name, p) in narrowed {
        let flipped: Vec<&str> = cases().iter().filter(|c| !(c.run)(&p)).map(|c| c.id).collect();
        assert!(!flipped.is_empty(), "narrowing {name} by 10x turned no case red");
    }
}

#[test]
fn a_two_sided_derivative_at_a_full_multiplicity_interior_knot_is_refused() {
    // Cubic, interior knot of multiplicity 3 (= degree): only one-sided limits exist for order 1.
    assert_eq!(check_derivative_request(1, 3, 3, Side::Both), Err(NumericError::TwoSidedAtBreak));
    assert_eq!(max_two_sided_order(3, 3), Some(0));
}

/// No evaluator exists yet. P3.1 replaces this with the production one; the geometry spike
/// (P1.3) supplies its prototype from its own test.
fn evaluator() -> Option<Box<dyn Evaluator>> {
    None
}

#[test]
#[ignore = "red until P3.1: no evaluator exists"]
fn evaluator_probes() {
    let ev = evaluator().expect("no evaluator exists yet (P3.1)");
    let failures: Vec<String> = probes().iter().filter_map(|p| run_probe(ev.as_ref(), p).err()).collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
