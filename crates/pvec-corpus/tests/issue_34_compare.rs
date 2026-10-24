//! Issue #34: expected values are compared under the P1.2 rules (`docs/NUMERIC_POLICY.md`), in the
//! mode each field was recorded with. The corpus ships the reader results and the verdicts the
//! fixture repository reached; this suite recomputes them with `pvec-numeric` and requires the same.

mod common;

use pvec_corpus::compare::{Mode, compare};
use pvec_corpus::json::{self, Value};
use pvec_corpus::scalar::{self, Scalar, through_decimal};
use pvec_corpus::{Bundle, ExpectedMath};

fn scalar_of(x: f64) -> Scalar {
    let text = format!(r#"{{"decimal": "{x:?}", "f64_hex": "{}"}}"#, scalar::to_hex(x.to_bits()));
    Scalar::from_json("t", &json::parse(text.as_bytes()).unwrap()).unwrap()
}

#[test]
fn issue_34_exact_mode_means_equal_bits_and_nothing_looser() {
    let want = scalar_of(0.5f64.sqrt());
    assert!(compare(Mode::Exact, &want, 0.5f64.sqrt(), 1.0));
    assert!(!compare(Mode::Exact, &want, f64::from_bits(want.bits() + 1), 1.0), "one ulp is not exact");
    assert!(!compare(Mode::Exact, &want, f64::NAN, 1.0));
    assert!(!compare(Mode::Exact, &want, f64::INFINITY, 1.0));
    // A huge scale does not widen an exact comparison.
    assert!(!compare(Mode::Exact, &want, 0.7072, f64::MAX));
}

#[test]
fn issue_34_a_value_routed_through_a_shortened_decimal_fails_the_exact_comparison() {
    let x = 0.5f64.sqrt();
    let want = scalar_of(x);
    let lossy = through_decimal(x, 15);
    assert_ne!(lossy.to_bits(), x.to_bits());
    assert!(!compare(Mode::Exact, &want, lossy, 1.0));
    assert!(compare(Mode::Exact, &want, through_decimal(x, 17), 1.0), "seventeen digits is lossless");
}

#[test]
fn issue_34_negative_zero_is_positive_zero_under_np_03() {
    let zero = scalar_of(0.0);
    assert!(compare(Mode::Exact, &zero, -0.0, 1.0));
    let neg = scalar_of(-0.0);
    assert!(compare(Mode::Exact, &neg, 0.0, 1.0));
    assert!(compare(Mode::Policy, &zero, -0.0, 1.0));
}

#[test]
fn issue_34_policy_mode_is_the_tolerance_triple_of_np_04() {
    let want = scalar_of(1.0);
    let off_by_four_ulps = f64::from_bits(want.bits() + 4);
    assert!(compare(Mode::Policy, &want, off_by_four_ulps, 1.0));
    // 0.00000001 is above REL * 1 and below REL * 1000000: the scale term is the feature size.
    assert!(!compare(Mode::Policy, &want, 1.00000001, 1.0));
    assert!(compare(Mode::Policy, &want, 1.00000001, 1000000.0));
    assert!(!compare(Mode::Policy, &want, 1.001, 1.0));
    assert!(!compare(Mode::Policy, &want, f64::NAN, 1.0));
    // The compare function is the pvec-numeric rule, not a copy of it.
    let p = pvec_numeric::Policy::V1;
    for (a, scale) in [(1.00000001, 1.0), (1.00000001, 1000000.0), (1.0000000000001, 1.0)] {
        assert_eq!(compare(Mode::Policy, &want, a, scale), p.near(1.0, a, scale));
    }
}

#[test]
fn issue_34_mode_names_are_the_manifest_spellings_and_nothing_else() {
    assert_eq!(Mode::parse("EXACT"), Some(Mode::Exact));
    assert_eq!(Mode::parse("POLICY"), Some(Mode::Policy));
    for bad in ["exact", "Policy", "TOLERANT", "", "EXACT "] {
        assert_eq!(Mode::parse(bad), None, "{bad:?}");
    }
}

fn hex_of(v: &Value) -> Option<u64> {
    scalar::parse_hex(v.as_str()?)
}

/// `a`, `a[1]` or `a[2][0]`, looked up in an entity's geometry.
fn lookup<'a>(geometry: &'a pvec_corpus::expected::Node, field: &str) -> Option<&'a Scalar> {
    let mut parts = field.split('[');
    let mut node = geometry.get(parts.next()?)?;
    for p in parts {
        let i: usize = p.strip_suffix(']')?.parse().ok()?;
        node = node.as_list()?.get(i)?;
    }
    node.as_scalar()
}

#[test]
fn issue_34_every_recorded_verdict_is_reproduced_by_the_rust_rules() {
    let bundle = Bundle::vendored().unwrap();
    let ids: Vec<String> = bundle
        .manifest()
        .get("fixtures")
        .unwrap()
        .as_arr()
        .unwrap()
        .iter()
        .map(|f| f.get("id").unwrap().as_str().unwrap().to_string())
        .collect();
    let (mut exact, mut policy, mut linked) = (0usize, 0usize, 0usize);
    for id in &ids {
        let cmp = json::parse(bundle.file(&format!("fixtures/{id}/comparison.json")).unwrap()).unwrap();
        assert_eq!(
            cmp.get("numeric_policy_version").and_then(Value::as_str),
            Some("PVec numeric policy v1"),
            "{id}"
        );
        assert_eq!(cmp.get("verdict").and_then(Value::as_str), Some("AGREE"), "{id}");
        assert_eq!(
            cmp.get("reader_vs_reader_differences").and_then(Value::as_arr).map(<[Value]>::len),
            Some(0),
            "{id}"
        );
        let expected =
            ExpectedMath::parse(bundle.file(&format!("fixtures/{id}/expected.math.json")).unwrap()).unwrap();
        for (side, checks) in cmp.get("checks").unwrap().as_obj().unwrap() {
            for c in checks.as_arr().unwrap() {
                let field = c.get("field").unwrap().as_str().unwrap();
                let ok = matches!(c.get("ok"), Some(Value::Bool(true)));
                let mode = Mode::parse(c.get("mode").unwrap().as_str().unwrap())
                    .unwrap_or_else(|| panic!("{id} {field}: mode"));
                let (want, got) = (c.get("expected").unwrap(), c.get("read").unwrap());
                let verdict = match (hex_of(want), hex_of(got)) {
                    (Some(w), Some(g)) => {
                        let w = scalar_of(f64::from_bits(w));
                        // Scale 0 is the strictest reading of NP-04: only the absolute and ulp terms.
                        // A verdict that holds there holds for any feature size.
                        compare(mode, &w, f64::from_bits(g), 0.0)
                    }
                    _ => {
                        assert_eq!(mode, Mode::Exact, "{id} {field}: a non-numeric field can only be exact");
                        want == got
                    }
                };
                assert_eq!(verdict, ok, "{id} {side} {field}: recorded ok={ok}");
                match mode {
                    Mode::Exact => exact += 1,
                    Mode::Policy => policy += 1,
                }
                // Link the verdict to the authored record: a verbatim field must carry the bits
                // that expected.math.json states.
                if mode == Mode::Exact {
                    if let (Some(w), Some(s)) = (hex_of(want), lookup(&expected.entities[0].geometry, field))
                    {
                        assert_eq!(
                            w,
                            s.bits(),
                            "{id} {side} {field}: comparison.json disagrees with expected.math.json"
                        );
                        linked += 1;
                    }
                }
            }
        }
    }
    assert_eq!((exact, policy), (1936, 136));
    assert!(linked > 400, "only {linked} comparison fields were tied back to expected.math.json");
}
