//! Issue #95: a scalar may declare its precision (significant decimal digits, O1.4). A declaration
//! is enforced: a decimal that carries more digits than declared, or a declaration that is fewer
//! than the hex needs, fails the load. No declaration is accepted, as in `r0-open`.

use pvec_corpus::json;
use pvec_corpus::scalar::{Scalar, to_hex};
use pvec_corpus::{CorpusError, ExpectedMath};

fn scalar(decimal: &str, hex: &str, precision: Option<&str>) -> Result<Scalar, CorpusError> {
    let p = precision.map(|p| format!(r#", "precision": {p}"#)).unwrap_or_default();
    let v = json::parse(format!(r#"{{"decimal": "{decimal}", "f64_hex": "{hex}"{p}}}"#).as_bytes()).unwrap();
    Scalar::from_json("t", &v)
}

fn hex(x: f64) -> String {
    to_hex(x.to_bits())
}

#[test]
fn issue_95_an_undeclared_precision_is_accepted_and_reported_as_none() {
    let s = scalar("0.5", &hex(0.5), None).unwrap();
    assert_eq!(s.precision(), None);
}

#[test]
fn issue_95_a_declared_precision_is_kept() {
    let s = scalar("0.30000000000000004", &hex(0.1 + 0.2), Some("17")).unwrap();
    assert_eq!(s.precision(), Some(17));
    let s = scalar("0.5", &hex(0.5), Some("1")).unwrap();
    assert_eq!(s.precision(), Some(1));
}

#[test]
fn issue_95_more_digits_than_declared_fails_the_load() {
    // 17 significant digits written, 15 declared.
    match scalar("0.30000000000000004", &hex(0.1 + 0.2), Some("15")) {
        Err(CorpusError::PrecisionExceeded { digits, declared, .. }) => assert_eq!((digits, declared), (17, 15)),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        scalar("0.50", &hex(0.5), Some("1")),
        Err(CorpusError::PrecisionExceeded { digits: 2, declared: 1, .. })
    ));
}

#[test]
fn issue_95_a_declaration_below_what_the_hex_needs_fails_the_load() {
    // 0.1+0.2 needs 17 digits to round-trip; declaring 16 cannot be honest.
    match scalar("0.30000000000000004", &hex(0.1 + 0.2), Some("16")) {
        Err(CorpusError::PrecisionTooLow { needed, declared, .. }) => assert_eq!((needed, declared), (17, 16)),
        Err(CorpusError::PrecisionExceeded { .. }) => {}
        other => panic!("{other:?}"),
    }
    // A short decimal that does not reach the hex fails whatever it declares.
    assert!(matches!(
        scalar("0.3", &hex(0.1 + 0.2), Some("17")),
        Err(CorpusError::DecimalMismatch { .. })
    ));
}

#[test]
fn issue_95_a_declaration_that_is_not_a_digit_count_is_refused() {
    for p in ["0", "-1", "18", "1000", "\"17\"", "true", "null"] {
        assert!(scalar("0.5", &hex(0.5), Some(p)).is_err(), "precision {p}");
    }
}

#[test]
fn issue_95_significant_digits_are_counted_as_written() {
    for (decimal, x, digits) in [
        ("0.5", 0.5, 1),
        ("0.050", 0.05, 2),
        ("1500", 1500.0, 2),
        ("1.500", 1.5, 4),
        ("-12.25", -12.25, 4),
        ("1e3", 1000.0, 1),
        ("1.25E-3", 0.00125, 3),
        ("0", 0.0, 1),
    ] {
        // Declare exactly the count: it loads. One fewer: it does not.
        assert!(scalar(decimal, &hex(x), Some(&digits.to_string())).is_ok(), "{decimal} at {digits}");
        if digits > 1 {
            assert!(scalar(decimal, &hex(x), Some(&(digits - 1).to_string())).is_err(), "{decimal} at {}", digits - 1);
        }
    }
}

#[test]
fn issue_95_the_error_names_the_place_and_both_counts() {
    let err = scalar("0.50", &hex(0.5), Some("1")).unwrap_err().to_string();
    assert!(err.contains("t") && err.contains("0.50") && err.contains('2') && err.contains('1'), "{err}");
}

#[test]
fn issue_95_a_declared_precision_is_enforced_inside_a_record_too() {
    let text = r#"{"center": {"decimal": "0.50", "f64_hex": "0x3fe0000000000000", "precision": 1}}"#;
    let err = ExpectedMath::parse_record(text.as_bytes()).unwrap_err();
    assert!(matches!(err, CorpusError::PrecisionExceeded { .. }), "{err:?}");
    let ok = r#"{"center": {"decimal": "0.5", "f64_hex": "0x3fe0000000000000", "precision": 1}}"#;
    assert!(ExpectedMath::parse_record(ok.as_bytes()).is_ok());
}

#[test]
fn issue_95_the_vendored_bundle_still_loads_with_no_declarations() {
    let bundle = pvec_corpus::Bundle::vendored().unwrap();
    assert!(bundle.paths().count() > 0);
}
