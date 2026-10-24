//! Issue #34: the loader parses every `expected.math.json` in the pinned revision, and numbers
//! recorded as IEEE-754 hex survive bit for bit.

mod common;

use pvec_capabilities::EvidenceClass;
use pvec_corpus::expected::EntityClass;
use pvec_corpus::json;
use pvec_corpus::scalar::{self, through_decimal};
use pvec_corpus::{Bundle, CorpusError, ExpectedMath};

fn manifest_ids(bundle: &Bundle) -> Vec<String> {
    bundle
        .manifest()
        .get("fixtures")
        .and_then(|f| f.as_arr())
        .expect("fixtures array")
        .iter()
        .map(|f| f.get("id").and_then(|i| i.as_str()).expect("id").to_string())
        .collect()
}

fn load_all(bundle: &Bundle) -> Vec<(String, ExpectedMath)> {
    manifest_ids(bundle)
        .into_iter()
        .map(|id| {
            let path = format!("fixtures/{id}/expected.math.json");
            let bytes = bundle.file(&path).unwrap_or_else(|| panic!("{path} is in the bundle"));
            let parsed = ExpectedMath::parse(bytes).unwrap_or_else(|e| panic!("{path}: {e}"));
            (id, parsed)
        })
        .collect()
}

#[test]
fn issue_34_every_expected_math_json_in_the_pinned_revision_parses() {
    let bundle = Bundle::vendored().unwrap();
    let all = load_all(&bundle);
    assert_eq!(all.len(), 30, "r0-open has 30 accepted fixtures");
    // And none is missed: every expected.math.json in the bundle is one of them.
    let files: Vec<&str> =
        bundle.paths().into_iter().filter(|p| p.ends_with("/expected.math.json")).collect();
    assert_eq!(files.len(), all.len());
    for (id, e) in &all {
        assert_eq!(&e.fixture_id, id);
        assert_eq!(e.evidence_class, EvidenceClass::KnownTruthExact);
        assert_eq!(e.units.name, "metre");
        assert_eq!(e.units.scale_to_canonical.bits(), 1.0f64.to_bits());
        assert!(!e.entities.is_empty());
    }
}

#[test]
fn issue_34_the_entity_classes_are_the_nine_epic_o_curves() {
    let bundle = Bundle::vendored().unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for (_, e) in load_all(&bundle) {
        for entity in &e.entities {
            seen.insert(format!("{:?}", entity.class));
        }
    }
    let want = [
        "Bezier",
        "Bspline",
        "Circle",
        "CircularArc",
        "Ellipse",
        "EllipticalArc",
        "Line",
        "Nurbs",
        "Polyline",
    ];
    assert_eq!(seen.into_iter().collect::<Vec<_>>(), want);
    // Geometry fields are present as the record's table says they must be.
    let circle = ExpectedMath::parse(bundle.file("fixtures/circle-001/expected.math.json").unwrap()).unwrap();
    let c = &circle.entities[0];
    assert_eq!(c.class, EntityClass::Circle);
    assert_eq!(c.geometry.get("radius").and_then(|n| n.as_scalar()).map(|s| s.value()), Some(2.75));
    assert_eq!(c.geometry.get("center").and_then(|n| n.as_list()).map(|l| l.len()), Some(3));
}

#[test]
fn issue_34_every_scalar_in_the_corpus_round_trips_bit_exactly() {
    let bundle = Bundle::vendored().unwrap();
    let mut count = 0usize;
    for (id, e) in load_all(&bundle) {
        for (path, s) in e.scalars() {
            let hex = scalar::to_hex(s.bits());
            assert_eq!(scalar::parse_hex(&hex), Some(s.bits()), "{id} {path}");
            assert_eq!(f64::from_bits(s.bits()).to_bits(), s.bits(), "{id} {path}");
            assert_eq!(
                s.decimal().parse::<f64>().unwrap().to_bits(),
                s.bits(),
                "{id} {path}: decimal vs hex"
            );
            count += 1;
        }
    }
    // The corpus is pinned, so its size is too: a loader that silently skipped a subtree would show here.
    assert_eq!(count, 886, "scalars visited across the 30 records");
}

#[test]
fn issue_34_the_bit_exact_check_is_not_vacuous_on_this_corpus() {
    // The check turns red for a shortened decimal only if some value in the corpus actually needs more
    // than fifteen digits. Count them, so a corpus of round numbers could not pass this suite silently.
    let bundle = Bundle::vendored().unwrap();
    let mut lossy = 0usize;
    for (_, e) in load_all(&bundle) {
        for (_, s) in e.scalars() {
            if through_decimal(s.value(), 15).to_bits() != s.bits() {
                lossy += 1;
            }
        }
    }
    assert!(lossy >= 10, "only {lossy} scalars lose bits through 15 digits");
}

#[test]
fn issue_34_routing_a_corpus_value_through_a_shortened_decimal_turns_the_load_red() {
    let bundle = Bundle::vendored().unwrap();
    let path = "fixtures/rotation-irrational-001/expected.math.json";
    let original = String::from_utf8(bundle.file(path).unwrap().to_vec()).unwrap();
    let parsed = ExpectedMath::parse(original.as_bytes()).unwrap();
    let (place, victim) = parsed
        .scalars()
        .into_iter()
        .find(|(_, s)| through_decimal(s.value(), 15).to_bits() != s.bits())
        .expect("rotation-irrational-001 carries a value that 15 digits cannot hold");
    let short = format!("{:.14e}", victim.value());
    let doctored = original.replacen(
        &format!("\"decimal\": \"{}\"", victim.decimal()),
        &format!("\"decimal\": \"{short}\""),
        1,
    );
    assert_ne!(doctored, original, "the doctored copy differs");
    match ExpectedMath::parse(doctored.as_bytes()) {
        Err(CorpusError::DecimalMismatch { decimal, hex, .. }) => {
            assert_eq!(decimal, short);
            assert_eq!(hex, scalar::to_hex(victim.bits()));
        }
        other => panic!("a shortened decimal at {place} must be refused, got {other:?}"),
    }
}

#[test]
fn issue_34_a_value_changed_in_its_hex_but_not_its_decimal_turns_the_load_red() {
    let bundle = Bundle::vendored().unwrap();
    let original =
        String::from_utf8(bundle.file("fixtures/circle-001/expected.math.json").unwrap().to_vec()).unwrap();
    // 2.75 is 0x4006000000000000; move it one ulp.
    let doctored = original.replacen("0x4006000000000000", "0x4006000000000001", 1);
    assert_ne!(doctored, original);
    assert!(matches!(ExpectedMath::parse(doctored.as_bytes()), Err(CorpusError::DecimalMismatch { .. })));
}

#[test]
fn issue_34_a_bare_float_anywhere_in_a_record_fails_the_load() {
    let bundle = Bundle::vendored().unwrap();
    let original =
        String::from_utf8(bundle.file("fixtures/circle-001/expected.math.json").unwrap().to_vec()).unwrap();
    let doctored = original.replacen(
        "\"schema_version\": \"pvec-gold/expected.math/1\"",
        "\"schema_version\": \"pvec-gold/expected.math/1\",\n  \"x\": 0.5",
        1,
    );
    assert!(matches!(ExpectedMath::parse(doctored.as_bytes()), Err(CorpusError::Json { .. })));
}

#[test]
fn issue_34_a_record_for_another_schema_or_class_is_refused() {
    let bundle = Bundle::vendored().unwrap();
    let original =
        String::from_utf8(bundle.file("fixtures/circle-001/expected.math.json").unwrap().to_vec()).unwrap();
    for (from, to) in [
        ("pvec-gold/expected.math/1", "pvec-gold/expected.math/2"),
        ("\"fixture_id\": \"circle-001\"", "\"fixture_id\": \"Circle 001\""),
        ("\"class\": \"circle\"", "\"class\": \"spiral\""),
        ("\"canonical\": \"metre\"", "\"canonical\": \"foot\""),
    ] {
        let doctored = original.replacen(from, to, 1);
        assert_ne!(doctored, original, "{from}");
        assert!(ExpectedMath::parse(doctored.as_bytes()).is_err(), "{from} -> {to}");
    }
    // The record is not an evidence class of its own choosing: only KNOWN_TRUTH_EXACT is authored.
    let doctored = original.replacen(
        "\"evidence_class\": \"KNOWN_TRUTH_EXACT\"",
        "\"evidence_class\": \"QUARANTINED\"",
        1,
    );
    assert!(ExpectedMath::parse(doctored.as_bytes()).is_err());
    assert!(json::parse(original.as_bytes()).is_ok());
}
