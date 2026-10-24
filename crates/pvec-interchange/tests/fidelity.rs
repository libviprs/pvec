//! The fidelity taxonomy (issue P6.1, pvec#18): construction rules, the conformance mapping, and the
//! versioned serialized form.

use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;
use pvec_interchange::*;
use std::fs;
use std::path::PathBuf;

fn eid(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn native(name: &str) -> TargetConstruct {
    TargetConstruct::new(name, Representation::Native).unwrap()
}

fn tess(name: &str) -> TargetConstruct {
    TargetConstruct::new(name, Representation::Tessellated).unwrap()
}

fn bound(v: f64) -> Fidelity {
    Fidelity::BoundedApproximation(ErrorBound::new(v).unwrap())
}

fn rec(n: u32, cap: CapabilityId, t: TargetConstruct, f: Fidelity, note: &str) -> FidelityRecord {
    FidelityRecord::new(eid(n), cap, t, f, note).unwrap()
}

// ---- the five values and their conformance mapping -------------------------------------------------

#[test]
fn there_are_exactly_five_values_with_stable_names() {
    let all = [
        (FidelityKind::Exact, "Exact"),
        (FidelityKind::ExactReparameterized, "ExactReparameterized"),
        (FidelityKind::EquivalentCompound, "EquivalentCompound"),
        (FidelityKind::BoundedApproximation, "BoundedApproximation"),
        (FidelityKind::Unsupported, "Unsupported"),
    ];
    for (k, name) in all {
        assert_eq!(k.name(), name);
        assert_eq!(FidelityKind::parse(name), Some(k));
    }
    assert_eq!(FidelityKind::parse("exact"), None);
    assert_eq!(FidelityKind::parse("Approximate"), None);
}

#[test]
fn the_kind_of_a_value_matches_its_variant() {
    assert_eq!(Fidelity::Exact.kind(), FidelityKind::Exact);
    assert_eq!(Fidelity::ExactReparameterized.kind(), FidelityKind::ExactReparameterized);
    assert_eq!(Fidelity::EquivalentCompound.kind(), FidelityKind::EquivalentCompound);
    assert_eq!(bound(0.5).kind(), FidelityKind::BoundedApproximation);
    assert_eq!(
        Fidelity::Unsupported(Reason::new("no spline surface").unwrap()).kind(),
        FidelityKind::Unsupported
    );
}

#[test]
fn the_conformance_mapping_is_the_one_in_the_issue() {
    let c = |k: FidelityKind| {
        let x = k.claims();
        (x.structural, x.geometric, x.display)
    };
    assert_eq!(c(FidelityKind::Exact), (true, true, false));
    assert_eq!(c(FidelityKind::ExactReparameterized), (true, true, false));
    assert_eq!(c(FidelityKind::EquivalentCompound), (false, true, false));
    assert_eq!(c(FidelityKind::BoundedApproximation), (false, false, true));
    assert_eq!(c(FidelityKind::Unsupported), (false, false, false));
}

// ---- the error bound type rule ----------------------------------------------------------------------

#[test]
fn a_bounded_approximation_cannot_be_built_without_a_finite_positive_bound() {
    for bad in [0.0, -0.0, -1.0, -f64::MIN_POSITIVE, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(ErrorBound::new(bad).is_err(), "{bad} must be refused");
    }
    assert_eq!(ErrorBound::new(f64::NAN), Err(ErrorBoundError::NotFinite));
    assert_eq!(ErrorBound::new(f64::INFINITY), Err(ErrorBoundError::NotFinite));
    assert_eq!(ErrorBound::new(0.0), Err(ErrorBoundError::NotPositive));
    assert_eq!(ErrorBound::new(-2.0), Err(ErrorBoundError::NotPositive));
    // The smallest positive subnormal is still a finite positive bound.
    assert_eq!(ErrorBound::new(f64::from_bits(1)).unwrap().get().to_bits(), 1);
    assert_eq!(ErrorBound::new(f64::MAX).unwrap().get(), f64::MAX);
}

#[test]
fn a_reason_cannot_be_blank() {
    assert!(Reason::new("").is_err());
    assert!(Reason::new("   \t\n").is_err());
    assert_eq!(Reason::new("hatch pattern has no target").unwrap().as_str(), "hatch pattern has no target");
}

#[test]
fn a_target_construct_must_be_named() {
    assert!(TargetConstruct::new("", Representation::Native).is_err());
    assert!(TargetConstruct::new("  ", Representation::Tessellated).is_err());
}

// ---- the tessellation rule ---------------------------------------------------------------------------

#[test]
fn a_tessellated_output_cannot_be_recorded_as_a_geometric_exactness_claim() {
    for f in [Fidelity::Exact, Fidelity::ExactReparameterized, Fidelity::EquivalentCompound] {
        let kind = f.kind();
        let r = FidelityRecord::new(eid(1), ids::CIRCLE, tess("POLYLINE"), f, "");
        assert_eq!(r.unwrap_err(), RecordError::TessellatedClaimedExact(kind));
    }
}

#[test]
fn a_tessellated_output_may_be_bounded_or_unsupported() {
    assert!(FidelityRecord::new(eid(1), ids::CIRCLE, tess("POLYLINE"), bound(0.01), "64 segments").is_ok());
    let u = Fidelity::Unsupported(Reason::new("target has no arcs").unwrap());
    assert!(FidelityRecord::new(eid(1), ids::CIRCLE, tess("POLYLINE"), u, "").is_ok());
}

#[test]
fn a_native_output_may_claim_any_value() {
    for f in [Fidelity::Exact, Fidelity::ExactReparameterized, Fidelity::EquivalentCompound, bound(0.0625)] {
        assert!(FidelityRecord::new(eid(1), ids::CIRCLE, native("CIRCLE"), f, "").is_ok());
    }
}

#[test]
fn a_record_must_name_a_source_capability() {
    let r = FidelityRecord::new(eid(1), CapabilityId::new(0), native("CIRCLE"), Fidelity::Exact, "");
    assert_eq!(r.unwrap_err(), RecordError::NoCapability);
}

#[test]
fn the_error_bound_of_a_record_is_the_one_of_its_value() {
    let b = rec(1, ids::ELLIPSE, tess("POLYLINE"), bound(0.25), "");
    assert_eq!(b.error_bound().unwrap().get(), 0.25);
    assert!(rec(1, ids::ELLIPSE, native("ELLIPSE"), Fidelity::Exact, "").error_bound().is_none());
}

// ---- the ledger --------------------------------------------------------------------------------------

fn sample() -> FidelityLedger {
    let mut l = FidelityLedger::new();
    l.record(rec(0, ids::LINE, native("LINE"), Fidelity::Exact, ""));
    l.record(rec(1, ids::CIRCLE, native("CIRCLE"), Fidelity::ExactReparameterized, "start angle moved to 0"));
    l.record(rec(
        2,
        ids::ELLIPSE,
        native("COMPOSITE_CURVE"),
        Fidelity::EquivalentCompound,
        "two arcs\nand a line",
    ));
    l.record(rec(3, ids::B_SPLINE_CURVE, tess("POLYLINE"), bound(0.1 + 0.2), "tab\there \\ backslash\r"));
    l.record(rec(
        4,
        ids::RATIONAL_B_SPLINE_CURVE,
        native("none"),
        Fidelity::Unsupported(Reason::new("target has no rational splines").unwrap()),
        "",
    ));
    l
}

#[test]
fn the_ledger_accumulates_in_order_and_counts_by_value() {
    let l = sample();
    assert_eq!(l.records().len(), 5);
    assert_eq!(l.records()[3].entity_id(), eid(3));
    assert_eq!(l.count(FidelityKind::Exact), 1);
    assert_eq!(l.count(FidelityKind::BoundedApproximation), 1);
    assert_eq!(l.count(FidelityKind::Unsupported), 1);
    assert_eq!(FidelityLedger::new().count(FidelityKind::Exact), 0);
}

#[test]
fn the_ledger_names_capabilities_the_registry_does_not_know() {
    let mut l = sample();
    assert!(l.unregistered(&Registry::v1()).is_empty());
    l.record(rec(9, CapabilityId::new(999_999), native("X"), Fidelity::Exact, ""));
    assert_eq!(l.unregistered(&Registry::v1()), vec![CapabilityId::new(999_999)]);
}

// ---- the serialized form -----------------------------------------------------------------------------

#[test]
fn the_serialized_form_starts_with_a_version_header() {
    let t = sample().to_text();
    assert_eq!(t.lines().next().unwrap(), format!("pvec-fidelity-ledger {FORMAT_MAJOR}.{FORMAT_MINOR}"));
}

#[test]
fn a_round_trip_preserves_every_field() {
    let l = sample();
    let back = FidelityLedger::from_text(&l.to_text()).unwrap();
    assert_eq!(back, l);
    assert_eq!(back.to_text(), l.to_text(), "serialization is deterministic");
    let n = &back.records()[2];
    assert_eq!(n.note(), "two arcs\nand a line");
    assert_eq!(n.target_construct().name(), "COMPOSITE_CURVE");
    assert_eq!(back.records()[3].target_construct().representation(), Representation::Tessellated);
}

#[test]
fn a_round_trip_preserves_the_exact_bit_pattern_of_the_error_bound() {
    let patterns: [u64; 6] = [
        (0.1f64 + 0.2).to_bits(),
        1, // smallest subnormal
        f64::MAX.to_bits(),
        f64::MIN_POSITIVE.to_bits(),
        0x3FF0_0000_0000_0001, // one ulp above 1.0
        0x7FEF_FFFF_FFFF_FFFF,
    ];
    for bits in patterns {
        let mut l = FidelityLedger::new();
        l.record(rec(0, ids::B_SPLINE_CURVE, tess("POLYLINE"), bound(f64::from_bits(bits)), ""));
        let back = FidelityLedger::from_text(&l.to_text()).unwrap();
        assert_eq!(back.records()[0].error_bound().unwrap().get().to_bits(), bits);
    }
}

#[test]
fn an_empty_ledger_round_trips() {
    let l = FidelityLedger::new();
    assert_eq!(FidelityLedger::from_text(&l.to_text()).unwrap(), l);
}

#[test]
fn a_record_missing_its_capability_or_target_fails_to_read() {
    let good = sample().to_text();
    for field in ["capability", "target"] {
        let broken: String =
            good.lines().filter(|l| !l.starts_with(&format!("{field} "))).map(|l| format!("{l}\n")).collect();
        let e = FidelityLedger::from_text(&broken).unwrap_err();
        assert!(matches!(e, ParseError::MissingField { field: f, .. } if f == field), "{field}: {e:?}");
    }
}

#[test]
fn a_zero_capability_id_fails_to_read() {
    let t = sample().to_text().replacen("capability 4\n", "capability 0\n", 1);
    assert!(FidelityLedger::from_text(&t).is_err());
}

#[test]
fn a_tessellated_record_claiming_exactness_fails_to_read() {
    let mut l = FidelityLedger::new();
    l.record(rec(0, ids::CIRCLE, tess("POLYLINE"), bound(0.5), ""));
    let t = l.to_text();
    let forged = t
        .replace("fidelity BoundedApproximation\n", "fidelity Exact\n")
        .replace(&t.lines().find(|l| l.starts_with("error_bound ")).map(|l| format!("{l}\n")).unwrap(), "");
    assert!(FidelityLedger::from_text(&forged).is_err());
}

#[test]
fn a_bounded_record_without_a_valid_bound_fails_to_read() {
    let mut l = FidelityLedger::new();
    l.record(rec(0, ids::CIRCLE, tess("POLYLINE"), bound(0.5), ""));
    let t = l.to_text();
    let bound_line = t.lines().find(|l| l.starts_with("error_bound ")).unwrap().to_string();
    for replacement in [
        String::new(),
        format!("error_bound 0x{:016x}\n", 0.0f64.to_bits()),
        format!("error_bound 0x{:016x}\n", f64::NAN.to_bits()),
        format!("error_bound 0x{:016x}\n", f64::INFINITY.to_bits()),
        format!("error_bound 0x{:016x}\n", (-1.0f64).to_bits()),
        "error_bound 0.5\n".to_string(),
    ] {
        let forged = t.replace(&format!("{bound_line}\n"), &replacement);
        assert!(FidelityLedger::from_text(&forged).is_err(), "accepted {replacement:?}");
    }
}

#[test]
fn an_unsupported_record_without_a_reason_fails_to_read() {
    let t = sample().to_text();
    let forged: String = t.lines().filter(|l| !l.starts_with("reason ")).map(|l| format!("{l}\n")).collect();
    assert!(FidelityLedger::from_text(&forged).is_err());
}

#[test]
fn an_exact_record_carrying_a_bound_fails_to_read() {
    let mut l = FidelityLedger::new();
    l.record(rec(0, ids::LINE, native("LINE"), Fidelity::Exact, ""));
    let t = l.to_text().replace("fidelity Exact\n", "fidelity Exact\nerror_bound 0x3fe0000000000000\n");
    assert!(FidelityLedger::from_text(&t).is_err());
}

#[test]
fn an_unknown_fidelity_name_is_refused_rather_than_guessed() {
    let t = sample().to_text().replacen("fidelity Exact\n", "fidelity NearlyExact\n", 1);
    assert!(FidelityLedger::from_text(&t).is_err());
}

#[test]
fn a_newer_minor_version_with_extra_fields_is_still_readable() {
    let l = sample();
    let t = l.to_text().replacen(
        &format!("pvec-fidelity-ledger {FORMAT_MAJOR}.{FORMAT_MINOR}"),
        &format!("pvec-fidelity-ledger {FORMAT_MAJOR}.{}", FORMAT_MINOR + 7),
        1,
    );
    // A later build adds a field to every record and a header-level line.
    let t = t.replace("\nend\n", "\nprovenance exporter-9\nend\n").replacen(
        "record\n",
        "future-header some value\nrecord\n",
        1,
    );
    assert_eq!(FidelityLedger::from_text(&t).unwrap(), l);
}

#[test]
fn a_newer_major_version_is_refused_by_number() {
    let t = sample().to_text().replacen(
        &format!("pvec-fidelity-ledger {FORMAT_MAJOR}."),
        &format!("pvec-fidelity-ledger {}.", FORMAT_MAJOR + 1),
        1,
    );
    assert_eq!(FidelityLedger::from_text(&t).unwrap_err(), ParseError::UnsupportedMajor(FORMAT_MAJOR + 1));
}

#[test]
fn garbage_and_truncation_are_refused() {
    assert_eq!(FidelityLedger::from_text("").unwrap_err(), ParseError::BadHeader);
    assert_eq!(FidelityLedger::from_text("hello 1.0\n").unwrap_err(), ParseError::BadHeader);
    assert_eq!(
        FidelityLedger::from_text("pvec-fidelity-ledger one.zero\n").unwrap_err(),
        ParseError::BadHeader
    );
    let t = sample().to_text();
    for cut in [t.len() - 1, t.len() / 2, 30] {
        assert!(FidelityLedger::from_text(&t[..cut]).is_err(), "accepted a ledger cut at {cut}");
    }
    // A trailer whose count disagrees with the records is a truncated or spliced report.
    let spliced = t.replace("ledger-end 5", "ledger-end 6");
    assert!(FidelityLedger::from_text(&spliced).is_err());
}

#[test]
fn the_serialized_form_is_pinned() {
    let mut l = FidelityLedger::new();
    l.record(rec(7, ids::CIRCLE, tess("POLYLINE"), bound(0.5), "64 segments"));
    l.record(rec(8, ids::LINE, native("LINE"), Fidelity::Exact, ""));
    let expected = "\
pvec-fidelity-ledger 1.0
record
entity 7
capability 6
target Tessellated POLYLINE
fidelity BoundedApproximation
error_bound 0x3fe0000000000000
note 64 segments
end
record
entity 8
capability 4
target Native LINE
fidelity Exact
note
end
ledger-end 2
";
    assert_eq!(l.to_text(), expected);
}

// ---- the document ------------------------------------------------------------------------------------

fn doc() -> String {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::read_to_string(root.join("docs/FIDELITY_TAXONOMY.md")).expect("docs/FIDELITY_TAXONOMY.md exists")
}

#[test]
fn the_document_defines_all_five_values_with_evidence() {
    let d = doc();
    for k in [
        FidelityKind::Exact,
        FidelityKind::ExactReparameterized,
        FidelityKind::EquivalentCompound,
        FidelityKind::BoundedApproximation,
        FidelityKind::Unsupported,
    ] {
        let h = format!("\n## {}\n", k.name());
        let start = d.find(&h).unwrap_or_else(|| panic!("no section for {}", k.name()));
        let body = &d[start + 1..];
        let body = &body[body.find('\n').unwrap()..];
        let body = body.split("\n## ").next().unwrap();
        for part in ["**Claim:**", "**Must be true:**", "**Evidence:**"] {
            assert!(body.contains(part), "{}: missing {part}", k.name());
        }
    }
}

#[test]
fn the_document_states_the_conformance_mapping_and_the_neighbouring_documents() {
    let d = doc();
    for needle in
        ["structural", "geometric", "display", "CONFORMANCE.md", "NUMERIC_POLICY.md", "CAPABILITIES.md"]
    {
        assert!(d.contains(needle), "the document never mentions {needle}");
    }
}
