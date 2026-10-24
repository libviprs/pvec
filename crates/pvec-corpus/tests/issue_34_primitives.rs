//! Issue #34: the JSON, SHA-256, tar and scalar layers the loader stands on. They are written in this
//! crate so the suite needs no dependency and no process, which makes them part of what has to be
//! trusted, so they are tested against known answers and against hostile input.

mod common;

use common::{header, tar_of, vendored_tar};
use pvec_corpus::json::{self, Value};
use pvec_corpus::scalar::{self, Scalar};
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{CorpusError, tar};

fn scalar_json(decimal: &str, hex: &str) -> Value {
    json::parse(format!(r#"{{"decimal": "{decimal}", "f64_hex": "{hex}"}}"#).as_bytes()).unwrap()
}

// ---- sha256 ----------------------------------------------------------------------------------

#[test]
fn issue_34_sha256_matches_the_fips_vectors() {
    assert_eq!(hex_digest(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hex_digest(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hex_digest(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    assert_eq!(
        hex_digest(&vec![b'a'; 1_000_000]),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
}

#[test]
fn issue_34_sha256_of_the_vendored_bundle_is_the_published_pin() {
    // The digest the fixture repository publishes for this tar, written out here so replacing the tar
    // and the pin file together does not go unnoticed.
    assert_eq!(
        hex_digest(&vendored_tar()),
        "901370036bbffbbb8cb30471c2bf90f171b4f966c5bdfea6b140591cd840ca50"
    );
}

// ---- json ------------------------------------------------------------------------------------

#[test]
fn issue_34_json_reads_objects_arrays_strings_and_integers() {
    let v = json::parse(br#" {"a": [1, -2, true, null, "x\u00e9\n\ud83d\ude00"], "b": {"c": 0}} "#).unwrap();
    let a = v.get("a").unwrap().as_arr().unwrap();
    assert_eq!(a[0].as_int(), Some(1));
    assert_eq!(a[1].as_int(), Some(-2));
    assert_eq!(a[2].as_bool(), Some(true));
    assert_eq!(a[3], Value::Null);
    assert_eq!(a[4].as_str(), Some("x\u{e9}\n\u{1f600}"));
    assert_eq!(v.get("b").unwrap().get("c").unwrap().as_int(), Some(0));
    assert_eq!(v.as_obj().unwrap()[0].0, "a", "object order is kept");
}

#[test]
fn issue_34_json_refuses_a_bare_float() {
    for text in ["1.5", "1e3", "-0.0", "[0.25]", r#"{"x": 2E-3}"#] {
        assert!(
            matches!(json::parse(text.as_bytes()), Err(CorpusError::Json { .. })),
            "{text} must be refused"
        );
    }
}

#[test]
fn issue_34_json_refuses_what_is_not_json() {
    let deep = format!("{}{}", "[".repeat(json::MAX_DEPTH + 1), "]".repeat(json::MAX_DEPTH + 1));
    for text in [
        "",
        "{",
        "[1,]",
        "{\"a\":1,}",
        "{\"a\":1,\"a\":2}",
        "01",
        "+1",
        "1 2",
        "nul",
        "\"\\x\"",
        "\"\\ud800\"",
        "99999999999999999999",
        "[1] x",
        "'a'",
        &deep,
    ] {
        assert!(json::parse(text.as_bytes()).is_err(), "{text:?} must be refused");
    }
    assert!(json::parse(&[b'"', 0xff, b'"']).is_err(), "invalid UTF-8 is refused");
    let ok = format!("{}{}", "[".repeat(json::MAX_DEPTH), "]".repeat(json::MAX_DEPTH));
    assert!(json::parse(ok.as_bytes()).is_ok(), "the depth limit itself is allowed");
}

// ---- tar -------------------------------------------------------------------------------------

#[test]
fn issue_34_tar_reads_the_vendored_bundle() {
    let bytes = vendored_tar();
    let entries = tar::parse(&bytes).unwrap();
    assert!(entries.len() > 200, "{} entries", entries.len());
    assert!(entries.iter().all(|e| e.path.starts_with("r0-open/")));
    assert!(entries.iter().any(|e| e.path == "r0-open/manifest.json"));
    let manifest = entries.iter().find(|e| e.path == "r0-open/manifest.json").unwrap();
    assert_eq!(hex_digest(manifest.data), "de3903df3e4d7bc0556a2d593298aaea34dcdbdcdec9015d753df14f1566bf21");
}

#[test]
fn issue_34_tar_round_trips_odd_sizes() {
    let files: Vec<(String, Vec<u8>)> =
        [0usize, 1, 511, 512, 513, 1500].iter().map(|n| (format!("r0-open/f{n}"), vec![7u8; *n])).collect();
    let bytes = tar_of(&files);
    let got = tar::parse(&bytes).unwrap();
    assert_eq!(got.len(), files.len());
    for (g, (name, data)) in got.iter().zip(&files) {
        assert_eq!(&g.path, name);
        assert_eq!(g.data, data.as_slice());
    }
}

#[test]
fn issue_34_tar_refuses_hostile_archives() {
    let good = tar_of(&[("r0-open/a".to_string(), b"hello".to_vec())]);
    assert!(tar::parse(&good).is_ok());

    let mut bad_sum = good.clone();
    bad_sum[10] ^= 1;
    assert!(tar::parse(&bad_sum).is_err(), "a header whose checksum is wrong");

    assert!(tar::parse(&good[..300]).is_err(), "a truncated header");
    assert!(tar::parse(&good[..512 + 3]).is_err(), "a body that runs past the end");

    let mut lying = Vec::new();
    lying.extend_from_slice(&header("r0-open/a", 1 << 30, b'0'));
    lying.extend_from_slice(&[0u8; 1024]);
    assert!(tar::parse(&lying).is_err(), "a size larger than the archive");

    for name in ["../escape", "r0-open/../escape", "/abs", "r0-open//x", "r0-open\\x", ""] {
        let t = tar_of(&[(name.to_string(), b"x".to_vec())]);
        assert!(tar::parse(&t).is_err(), "path {name:?}");
    }
    let dup = tar_of(&[("r0-open/a".to_string(), vec![]), ("r0-open/a".to_string(), vec![])]);
    assert!(tar::parse(&dup).is_err(), "duplicate path");

    for flag in *b"21xgL3" {
        let mut t = Vec::new();
        t.extend_from_slice(&header("r0-open/a", 0, flag));
        t.extend_from_slice(&[0u8; 1024]);
        assert!(tar::parse(&t).is_err(), "typeflag {}", flag as char);
    }
}

#[test]
fn issue_34_tar_bounds_the_entry_count() {
    let mut t = Vec::new();
    for i in 0..=tar::MAX_ENTRIES {
        t.extend_from_slice(&header(&format!("r0-open/{i}"), 0, b'0'));
    }
    t.extend_from_slice(&[0u8; 1024]);
    assert!(tar::parse(&t).is_err());
}

// ---- scalar ----------------------------------------------------------------------------------

#[test]
fn issue_34_hex_round_trips_every_bit_pattern_it_is_given() {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut samples =
        vec![0, 1, 0x8000_0000_0000_0000, 0x7ff0_0000_0000_0000, 0x7ff8_0000_0000_0001, u64::MAX];
    for _ in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        samples.push(x);
    }
    for bits in samples {
        let hex = scalar::to_hex(bits);
        assert_eq!(hex.len(), 18);
        assert_eq!(scalar::parse_hex(&hex), Some(bits), "{hex}");
        assert_eq!(f64::from_bits(bits).to_bits(), bits);
    }
}

#[test]
fn issue_34_hex_parsing_is_strict() {
    assert_eq!(scalar::parse_hex("0x3fe6a09e667f3bcd"), Some(0x3fe6_a09e_667f_3bcd));
    for bad in [
        "3fe6a09e667f3bcd",
        "0X3fe6a09e667f3bcd",
        "0x3FE6A09E667F3BCD",
        "0x3fe6a09e667f3bc",
        "0x3fe6a09e667f3bcde",
        "0x",
        "0xg",
        "",
    ] {
        assert_eq!(scalar::parse_hex(bad), None, "{bad:?}");
    }
}

#[test]
fn issue_34_a_scalar_whose_decimal_matches_its_bits_loads() {
    let s = Scalar::from_json("t", &scalar_json("0.70710678118654757", "0x3fe6a09e667f3bcd")).unwrap();
    assert_eq!(s.bits(), 0x3fe6_a09e_667f_3bcd);
    assert_eq!(s.value(), 0.5f64.sqrt());
    assert_eq!(s.decimal(), "0.70710678118654757");
}

#[test]
fn issue_34_a_shortened_decimal_is_refused_with_its_place_named() {
    let v = scalar_json("0.707106781186548", "0x3fe6a09e667f3bcd");
    match Scalar::from_json("entities[0].geometry.radius", &v) {
        Err(CorpusError::DecimalMismatch { at, decimal, hex }) => {
            assert_eq!(at, "entities[0].geometry.radius");
            assert_eq!(decimal, "0.707106781186548");
            assert_eq!(hex, "0x3fe6a09e667f3bcd");
        }
        other => panic!("expected a decimal mismatch, got {other:?}"),
    }
    // The pair quoted in the epic is one ulp off, and is refused for the same reason.
    assert!(Scalar::from_json("t", &scalar_json("0.70710678118654757", "0x3fe6a09e667f3bcc")).is_err());
}

#[test]
fn issue_34_routing_through_a_short_decimal_loses_bits_and_seventeen_digits_do_not() {
    let x = 0.5f64.sqrt();
    assert_ne!(scalar::through_decimal(x, 15).to_bits(), x.to_bits());
    let mut seed: u64 = 12345;
    for _ in 0..5000 {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let v = f64::from_bits((seed >> 12) | 0x3ff0_0000_0000_0000) * 1000.0 - 1500.0;
        assert_eq!(scalar::through_decimal(v, 17).to_bits(), v.to_bits(), "{v}");
    }
}

#[test]
fn issue_34_a_scalar_must_have_exactly_the_two_fields_in_canonical_form() {
    let good_hex = "0x3ff0000000000000";
    for (decimal, hex) in [
        ("+1.0", good_hex),
        (".5", good_hex),
        ("1.", good_hex),
        ("01.0", good_hex),
        ("NaN", good_hex),
        ("1.0", "0X3ff0000000000000"),
        ("1.0", "0x3FF0000000000000"),
        ("1.0", "3ff0000000000000"),
        ("", good_hex),
        ("1.0", "0x7ff8000000000000"),
    ] {
        assert!(Scalar::from_json("t", &scalar_json(decimal, hex)).is_err(), "{decimal:?} {hex:?}");
    }
    for text in [
        r#"{"decimal": "1.0"}"#,
        r#"{"f64_hex": "0x3ff0000000000000"}"#,
        r#"{"decimal": "1.0", "f64_hex": "0x3ff0000000000000", "extra": 1}"#,
        r#"{"decimal": 1, "f64_hex": "0x3ff0000000000000"}"#,
        r#"["1.0"]"#,
    ] {
        let v = json::parse(text.as_bytes()).unwrap();
        assert!(Scalar::from_json("t", &v).is_err(), "{text}");
    }
}

#[test]
fn issue_34_a_non_finite_scalar_is_refused_under_np_01() {
    for (decimal, hex) in [
        ("1e999", "0x7ff0000000000000"),
        ("-1e999", "0xfff0000000000000"),
        ("1.7976931348623159e308", "0x7ff0000000000000"),
        ("0.0", "0x7ff8000000000000"),
        ("0.0", "0x7ff8000000000001"),
        ("0.0", "0xfff8000000000000"),
        ("0.0", "0x7ff0000000000001"),
    ] {
        match Scalar::from_json("t", &scalar_json(decimal, hex)) {
            Err(CorpusError::Invalid { why, .. }) => {
                assert!(why.contains("finite"), "{decimal} {hex}: {why}")
            }
            other => panic!("{decimal} {hex}: expected a finiteness refusal, got {other:?}"),
        }
    }
}
