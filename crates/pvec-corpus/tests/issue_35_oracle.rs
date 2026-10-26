//! Issue #35: the dual STEP conversion oracle adapter. Every test builds a small, internally
//! consistent artifact set and breaks exactly one thing, so each assertion fails for a wrong adapter.

use pvec_capabilities::EvidenceClass;
use pvec_corpus::oracle::{Converter, Files, OracleFixture, check_report_wording};
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{CorpusError, Result};

const ID: &str = "dual-line-001";

#[derive(Clone)]
struct Set {
    class: &'static str,
    secondary: bool,
    primary_eval: [&'static str; 3],
    secondary_eval: [&'static str; 3],
    primary_host: &'static str,
    secondary_driver: &'static str,
    product_version: &'static str,
}

impl Set {
    fn good() -> Set {
        Set {
            class: "DUAL_ORACLE_EQUIVALENT",
            secondary: true,
            primary_eval: ["step_eval", "1.4.0", "7.6.3"],
            secondary_eval: ["step_eval", "1.4.0", "7.6.3"],
            primary_host: "x86_64-native",
            secondary_driver: "tools/nas.sh",
            product_version: "3.20.0",
        }
    }

    fn verification(e: [&str; 3]) -> String {
        format!(
            r#"{{"schema_version":"pvec-gold/verification/1","tool":{{"name":"{}","evaluator_version":"{}","occt_version":"{}"}},"numeric_policy_version":"PVec numeric policy v1","units":{{"name":"millimetre"}},"entities":[]}}"#,
            e[0], e[1], e[2]
        )
    }

    fn files(&self) -> Files {
        let mut f = Files::new();
        let mut put = |name: &str, body: String| {
            f.insert(format!("{ID}/{name}"), body.into_bytes());
        };
        put("primary.step", "ISO-10303-21; primary".to_string());
        put("primary.verification.json", Set::verification(self.primary_eval));
        put("signature.json", format!(r#"{{"schema_version":"pvec-gold/signature/1","fixture_id":"{ID}"}}"#));
        let h = |f: &Files, n: &str| hex_digest(&f[&format!("{ID}/{n}")]);
        let art = |f: &Files, stem: &str, conv: &str, name: &str, host: &str, driver: &str, ver: &str| {
            format!(
                r#"{{"converter":"{conv}","product":{{"name":"{name}","version":"{ver}"}},"host":{{"class":"{host}","machine":"mars","driver":"{driver}"}},"step":{{"path":"{stem}.step","sha256":"{}"}},"verification":{{"path":"{stem}.verification.json","sha256":"{}"}}}}"#,
                h(f, &format!("{stem}.step")),
                h(f, &format!("{stem}.verification.json"))
            )
        };
        let mut secondary = String::new();
        let mut extra: Vec<(String, String)> = Vec::new();
        if self.secondary {
            extra.push(("secondary.step".into(), "ISO-10303-21; secondary".into()));
            extra.push(("secondary.verification.json".into(), Set::verification(self.secondary_eval)));
        }
        for (n, b) in &extra {
            f.insert(format!("{ID}/{n}"), b.clone().into_bytes());
        }
        if self.secondary {
            secondary = format!(
                r#","secondary":{}"#,
                art(&f, "secondary", "oda", "ODA SDK", "x86_64-native", self.secondary_driver, "26.4")
            );
        }
        let primary = art(
            &f,
            "primary",
            "cadex",
            "CAD Exchanger SDK",
            self.primary_host,
            "tools/nas.sh",
            self.product_version,
        );
        let manifest = format!(
            r#"{{"schema_version":"pvec/oracle-artifacts/1","fixture_id":"{ID}","evidence_class":"{}","signature":{{"path":"signature.json","sha256":"{}"}},"primary":{primary}{secondary}}}"#,
            self.class,
            h(&f, "signature.json")
        );
        f.insert(format!("{ID}/oracle.json"), manifest.into_bytes());
        f
    }
}

fn load(set: &Set) -> Result<OracleFixture> {
    OracleFixture::load(&set.files(), ID)
}

#[test]
fn issue_35_a_complete_set_loads_with_both_artifacts_and_one_evaluator() {
    let fx = load(&Set::good()).expect("loads");
    assert_eq!(fx.class, EvidenceClass::DualOracleEquivalent);
    assert_eq!(fx.primary.converter, Converter::CadExchanger);
    let dual = fx.dual().expect("dual-oracle evidence is available");
    assert_eq!(dual.secondary.converter, Converter::Oda);
    assert_eq!(dual.evaluator.name, "step_eval");
    assert_eq!(dual.primary.evaluator, dual.secondary.evaluator);
    assert_eq!(fx.signature_schema, "pvec-gold/signature/1");
}

#[test]
fn issue_35_a_missing_secondary_makes_dual_oracle_unavailable_and_is_not_single_sourced() {
    // Registered as REAL_WORLD_DIFFERENTIAL, the one-sided fixture loads, but it carries no dual evidence.
    let one = Set { class: "REAL_WORLD_DIFFERENTIAL", secondary: false, ..Set::good() };
    let fx = load(&one).expect("a real-world fixture may have one STEP output");
    assert!(fx.secondary.is_none());
    match fx.dual() {
        Err(CorpusError::DualOracleUnavailable { fixture, route }) => {
            assert_eq!(fixture, ID);
            assert_eq!(route, "REAL_WORLD_DIFFERENTIAL");
        }
        other => panic!("expected the dual oracle to be unavailable, got {other:?}"),
    }
    // QUARANTINED is the other legal home.
    let q = Set { class: "QUARANTINED", secondary: false, ..Set::good() };
    assert!(matches!(
        load(&q).unwrap().dual(),
        Err(CorpusError::DualOracleUnavailable { route, .. }) if route == "QUARANTINED"
    ));
    // Claiming DUAL_ORACLE_EQUIVALENT with one output is refused at load: the downgrade is not silent.
    let bad = Set { class: "DUAL_ORACLE_EQUIVALENT", secondary: false, ..Set::good() };
    assert!(matches!(load(&bad), Err(CorpusError::DualOracleUnavailable { .. })), "{:?}", load(&bad));
}

#[test]
fn issue_35_diverging_evaluator_identity_fails_the_run() {
    for (which, e) in [
        ("name", ["step_evil", "1.4.0", "7.6.3"]),
        ("version", ["step_eval", "1.5.0", "7.6.3"]),
        ("kernel", ["step_eval", "1.4.0", "7.7.0"]),
    ] {
        let set = Set { secondary_eval: e, ..Set::good() };
        match load(&set) {
            Err(CorpusError::EvaluatorMismatch { fixture, .. }) => assert_eq!(fixture, ID, "{which}"),
            other => panic!("diverging evaluator {which} was accepted: {other:?}"),
        }
    }
}

#[test]
fn issue_35_every_provenance_field_is_required_and_a_missing_one_fails() {
    let good = Set::good().files();
    let key = format!("{ID}/oracle.json");
    let manifest = String::from_utf8(good[&key].clone()).unwrap();
    // Remove each recorded field in turn from the primary artifact (fields appear once per artifact;
    // the primary comes first).
    for field in [
        r#""converter":"cadex","#,
        r#""product":{"name":"CAD Exchanger SDK","version":"3.20.0"},"#,
        r#""host":{"class":"x86_64-native","machine":"mars","driver":"tools/nas.sh"},"#,
    ] {
        assert!(manifest.contains(field), "{field}");
        let mut f = good.clone();
        f.insert(key.clone(), manifest.replacen(field, "", 1).into_bytes());
        assert!(OracleFixture::load(&f, ID).is_err(), "removing {field} was accepted");
    }
    for (from, to) in [
        (r#""version":"3.20.0""#, r#""version":"""#),
        (r#""name":"CAD Exchanger SDK""#, r#""name":"""#),
        (r#""machine":"mars""#, r#""machine":"""#),
        (r#""class":"x86_64-native""#, r#""class":"arm64-emulated""#),
        (r#""driver":"tools/nas.sh""#, r#""driver":"cargo run""#),
        (r#""converter":"cadex""#, r#""converter":"oda""#),
    ] {
        let mut f = good.clone();
        f.insert(key.clone(), manifest.replacen(from, to, 1).into_bytes());
        assert!(OracleFixture::load(&f, ID).is_err(), "{from} -> {to} was accepted");
    }
    // The secondary is held to the same rule.
    let set = Set { secondary_driver: "python3 convert.py", ..Set::good() };
    assert!(load(&set).is_err());
    let set = Set { primary_host: "arm64-native", ..Set::good() };
    assert!(load(&set).is_err());
}

#[test]
fn issue_35_a_file_that_does_not_match_its_digest_is_refused() {
    for name in ["primary.step", "secondary.step", "primary.verification.json", "signature.json"] {
        let mut f = Set::good().files();
        f.get_mut(&format!("{ID}/{name}")).unwrap().push(b' ');
        match OracleFixture::load(&f, ID) {
            Err(CorpusError::HashMismatch { what, .. }) => assert!(what.contains(name), "{what}"),
            other => panic!("{name} tampered: {other:?}"),
        }
    }
}

#[test]
fn issue_35_the_class_is_read_from_the_manifest_and_must_be_one_of_the_four() {
    let set = Set { class: "GOLD_STANDARD", ..Set::good() };
    assert!(matches!(load(&set), Err(CorpusError::Unclassified { .. })));
    let mut f = Set::good().files();
    let key = format!("{ID}/oracle.json");
    let text = String::from_utf8(f[&key].clone())
        .unwrap()
        .replace(&format!(r#""fixture_id":"{ID}""#), r#""fixture_id":"other""#);
    f.insert(key, text.into_bytes());
    assert!(OracleFixture::load(&f, ID).is_err(), "a manifest for another fixture was accepted");
}

#[test]
fn issue_35_the_report_line_says_agreement_and_never_source_truth() {
    let fx = load(&Set::good()).unwrap();
    let line = fx.report_line();
    assert!(line.contains(ID) && line.contains("agreement") && line.contains("step_eval 1.4.0"), "{line}");
    assert!(line.contains("DUAL_ORACLE_EQUIVALENT"), "{line}");
    check_report_wording(&line).expect("the emitted line passes its own check");
    for bad in [
        "both converters agree, so this is the source truth",
        "converter agreement confirms the ground truth of the drawing",
        "Agreement proves the original DWG intent",
        "the two STEP files are true to the source",
    ] {
        assert!(check_report_wording(bad).is_err(), "{bad:?} was accepted");
    }
    check_report_wording("converter agreement under step_eval 1.4.0; not source truth")
        .expect_err("even a negated phrase is refused: the wording check is a ban list, not a parser");
}
