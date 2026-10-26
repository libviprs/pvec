//! Issue P7.7: the `pvec` command line tool. Each test is named for the acceptance criterion it holds.

mod support;

use pvec_geometry::Curve;
use support::json::{V, validate};
use support::*;

fn check_schema(run: &Run) -> V {
    let doc = run.json();
    validate(&schema(), &doc).unwrap_or_else(|e| panic!("{e}\n{}", run.stdout));
    assert_eq!(doc.get("report_schema_version").num(), 1.0);
    doc
}

fn fixture() -> String {
    file("good.pvec", &good_file())
}

#[test]
fn validate_on_a_known_good_fixture_exits_0_and_its_json_validates_against_the_checked_in_schema() {
    let run = pvec(&["validate", "--format", "json", &fixture()]);
    assert_eq!((run.code, run.stderr.as_str()), (0, ""), "{}", run.stdout);
    let doc = check_schema(&run);
    assert_eq!(doc.get("command").str(), "validate");
    assert_eq!(doc.at("result.format_valid"), &V::Bool(true));
    assert_eq!(doc.at("result.entities").num(), 3.0);
}

#[test]
fn every_subcommand_has_a_json_report_that_validates_against_the_schema() {
    let good = fixture();
    let cmp = file("compare.pvec", &compare_file());
    let (c, _): (Vec<Curve>, Vec<u32>) = curves().into_iter().map(|(i, c)| (c, i)).unzip();
    let first = file("first.txt", evaluator_report(&c, 1.0).as_bytes());
    let step = file("model.step", b"ISO-10303-21;\nEND-ISO-10303-21;\n");
    let runs = [
        pvec(&["inspect", "--format", "json", &good]),
        pvec(&["validate", "--format", "json", &good]),
        pvec(&["dump-manifest", "--format", "json", &good]),
        pvec(&["capabilities", "--format", "json", &good]),
        pvec(&["tessellate", "--error-px", "0.25", "--format", "json", &good]),
        pvec(&["bench-open", "--iterations", "3", "--format", "json", &good]),
        pvec(&["compare-step", "--format", "json", "--evaluator-report", &first, "--requests", &cmp, &step]),
    ];
    for run in &runs {
        assert_eq!(run.code, 0, "{} {}", run.stdout, run.stderr);
        check_schema(run);
    }
    let commands: Vec<String> = runs.iter().map(|r| r.json().get("command").str().to_string()).collect();
    assert_eq!(
        commands,
        ["inspect", "validate", "dump-manifest", "capabilities", "tessellate", "bench-open", "compare-step"]
    );
}

#[test]
fn inspect_and_dump_manifest_say_what_is_in_the_file() {
    let good = fixture();
    let inspect = pvec(&["inspect", "--format", "json", &good]).json();
    assert_eq!(inspect.at("result.header.container_major").num(), 1.0);
    assert!(inspect.at("result.directory.blocks").num() >= 3.0);
    let manifest = pvec(&["dump-manifest", "--format", "json", &good]).json();
    let ids: Vec<f64> = manifest.at("result.capabilities").arr().iter().map(|c| c.get("id").num()).collect();
    assert_eq!(ids, [5.0, 6.0, 11.0]);
    assert_eq!(manifest.at("result.metadata.producer_id").str(), "pvec-conformance-tests");
}

#[test]
fn the_exit_codes_are_the_contract() {
    let good = fixture();
    assert_eq!(pvec(&["validate", &good]).code, 0);
    let bad = file("junk.pvec", &[7u8; 300]);
    let refused = pvec(&["validate", "--format", "json", &bad]);
    assert_eq!(refused.code, 2, "{}", refused.stdout);
    check_schema(&refused);
    assert_eq!(refused.json().at("result.format_valid"), &V::Bool(false));
    assert_eq!(pvec(&["validate", "/nonexistent/file.pvec"]).code, 65);
    assert_eq!(pvec(&["frobnicate"]).code, 64);
    assert_eq!(pvec(&["validate"]).code, 64);
    assert_eq!(pvec(&["validate", "--format", "xml", &good]).code, 64);
    assert_eq!(pvec(&["tessellate", "--error-px", "-1", &good]).code, 64);
}

#[test]
fn a_file_requiring_an_unsupported_capability_exits_3_and_capabilities_lists_its_id() {
    use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
    let needs = rebuild(&good_file(), |m, _| {
        m.capabilities.push(Declared {
            id: CapabilityId::new(99),
            requirement: Requirement::Required,
            version: Version::new(2, 1, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
    });
    let path = file("needs99.pvec", &needs);
    let v = pvec(&["validate", "--format", "json", &path]);
    assert_eq!(v.code, 3, "{}", v.stdout);
    check_schema(&v);
    assert_eq!(v.json().at("error.code").str(), "NEGOTIATE-01");
    let c = pvec(&["capabilities", "--format", "json", &path]);
    assert_eq!(c.code, 3, "{}", c.stdout);
    let doc = check_schema(&c);
    let ids: Vec<f64> =
        doc.at("result.unsupported_required").arr().iter().map(|d| d.get("id").num()).collect();
    assert_eq!(ids, [99.0]);
    assert_eq!(doc.at("result.supported"), &V::Bool(false));
    let human = pvec(&["capabilities", &path]);
    assert!(human.stdout.contains("99"), "{}", human.stdout);
    // the same file is still inspectable: reading what it says is not opening it
    assert_eq!(pvec(&["inspect", &path]).code, 0);
}

#[test]
fn capabilities_reports_a_newer_minor_as_a_version_gap_and_an_unknown_optional_as_skipped() {
    use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
    let newer = rebuild(&good_file(), |m, _| {
        m.capabilities.retain(|d| d.id.get() != 6);
        m.capabilities.push(Declared {
            id: CapabilityId::new(6),
            requirement: Requirement::Required,
            version: Version::new(1, 4, 0),
        });
        m.capabilities.push(Declared {
            id: CapabilityId::new(77),
            requirement: Requirement::Optional,
            version: Version::new(1, 0, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
    });
    let run = pvec(&["capabilities", "--format", "json", &file("newer.pvec", &newer)]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let doc = check_schema(&run);
    assert_eq!(doc.at("result.supported"), &V::Bool(true));
    assert_eq!(doc.at("result.version_gaps").arr()[0].get("id").num(), 6.0);
    assert_eq!(doc.at("result.skipped_optional").arr()[0].get("id").num(), 77.0);
}

fn compare(stretch: f64, bend: f64) -> Run {
    let cs: Vec<Curve> = curves().into_iter().map(|(_, c)| c).collect();
    let pv = file("compare.pvec", &compare_file());
    let step = file("model.step", b"ISO-10303-21;\nEND-ISO-10303-21;\n");
    let first_text = evaluator_report(&cs, stretch);
    let first = file("first.txt", first_text.as_bytes());
    let requests =
        pvec(&["compare-step", "--format", "json", "--evaluator-report", &first, "--requests", &pv, &step]);
    assert_eq!(requests.code, 0, "{}", requests.stdout);
    let req = requests.json().at("result.requests").str().to_string();
    let second = file("second.txt", answer(&first_text, &req, &cs, stretch, bend).as_bytes());
    pvec(&["compare-step", "--format", "json", "--evaluator-report", &second, &pv, &step])
}

#[test]
fn compare_step_on_a_known_truth_fixture_reports_structural_and_geometric_in_separate_fields() {
    let run = compare(1.0, 0.0);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let doc = check_schema(&run);
    let v = doc.at("result.verdict");
    assert_eq!(v.at("structural_exactness.status").str(), "pass");
    assert_eq!(v.at("geometric_exactness.status").str(), "pass");
    assert_eq!(v.at("display_fidelity.status").str(), "not_assessed");
    assert!(v.at("structural_exactness.checked").num() > 0.0);
    assert!(v.at("geometric_exactness.checked").num() > 0.0);
    assert!(!doc.keys().contains(&"conforms"), "no single boolean");
}

#[test]
fn compare_step_on_a_reparameterised_input_is_geometric_exactness_with_a_structural_difference() {
    let run = compare(2.0, 0.0);
    assert_eq!(run.code, 1, "{}", run.stdout);
    let doc = check_schema(&run);
    let v = doc.at("result.verdict");
    assert_eq!(v.at("geometric_exactness.status").str(), "pass", "{}", run.stdout);
    assert_eq!(v.at("structural_exactness.status").str(), "differs", "{}", run.stdout);
    let first = &v.at("structural_exactness.disagreements").arr()[0];
    assert_eq!(first.get("entity").num(), 2.0);
    assert!(first.get("pvec").str().contains("knots"), "{first:?}");
}

#[test]
fn compare_step_with_a_displaced_locus_fails_the_geometric_class_and_not_the_structural_one() {
    let run = compare(1.0, 0.5);
    assert_eq!(run.code, 1, "{}", run.stdout);
    let v = run.json().at("result.verdict").clone();
    assert_eq!(v.at("geometric_exactness.status").str(), "fail");
    assert_eq!(v.at("structural_exactness.status").str(), "pass");
}

#[test]
fn compare_step_refuses_a_report_that_does_not_belong_to_the_file() {
    let pv = file("compare.pvec", &compare_file());
    let step = file("model.step", b"ISO-10303-21;\nEND-ISO-10303-21;\n");
    let one = file("one-edge.txt", b"EVALUATOR freecad=0.20.2 occt=7.6.3 edges=0\n");
    let run = pvec(&["compare-step", "--format", "json", "--evaluator-report", &one, &pv, &step]);
    assert_eq!(run.code, 2, "{}", run.stdout);
    assert_eq!(run.json().at("error.code").str(), "STEP-REPORT");
    assert_eq!(pvec(&["compare-step", &pv, &step]).code, 64, "the report is required");
}

#[test]
fn tessellate_reports_display_fidelity_only_and_asserts_no_exactness() {
    let run = pvec(&[
        "tessellate",
        "--error-px",
        "0.25",
        "--units-per-pixel",
        "0.1",
        "--format",
        "json",
        &fixture(),
    ]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let doc = check_schema(&run);
    let r = doc.at("result");
    assert_eq!(r.at("display_fidelity.status").str(), "pass");
    assert!(r.at("display_fidelity.max_deviation_px").num() <= 0.25);
    assert_eq!(r.get("error_budget_px").num(), 0.25);
    let mut keys = Vec::new();
    doc.all_keys(&mut keys);
    assert!(keys.iter().all(|k| !k.to_lowercase().contains("exact")), "{keys:?}");
    assert!(!run.stdout.to_lowercase().contains("exact"), "no word claiming exactness: {}", run.stdout);
    let entities = r.get("entities").arr();
    assert_eq!(entities.len(), 3);
    assert!(entities[0].get("points").num() >= 2.0);
    // a tighter budget never gives fewer points
    let tight = pvec(&[
        "tessellate",
        "--error-px",
        "0.01",
        "--units-per-pixel",
        "0.1",
        "--format",
        "json",
        &fixture(),
    ]);
    let total = |d: &V| d.at("result.entities").arr().iter().map(|e| e.get("points").num()).sum::<f64>();
    assert!(total(&tight.json()) > total(&doc));
}

#[test]
fn two_runs_on_the_same_input_write_byte_identical_json() {
    let good = fixture();
    let cmp = file("compare.pvec", &compare_file());
    let step = file("model.step", b"ISO-10303-21;\nEND-ISO-10303-21;\n");
    let cs: Vec<Curve> = curves().into_iter().map(|(_, c)| c).collect();
    let first = file("first.txt", evaluator_report(&cs, 1.0).as_bytes());
    for args in [
        vec!["inspect", "--format", "json", good.as_str()],
        vec!["validate", "--format", "json", good.as_str()],
        vec!["dump-manifest", "--format", "json", good.as_str()],
        vec!["capabilities", "--format", "json", good.as_str()],
        vec!["tessellate", "--error-px", "0.5", "--format", "json", good.as_str()],
        vec![
            "compare-step",
            "--format",
            "json",
            "--evaluator-report",
            first.as_str(),
            "--requests",
            cmp.as_str(),
            step.as_str(),
        ],
    ] {
        let (a, b) = (pvec(&args), pvec(&args));
        assert_eq!(a.stdout, b.stdout, "{args:?}");
        assert_eq!((a.code, b.code), (0, 0), "{args:?}");
    }
}

#[test]
fn bench_open_lists_its_volatile_fields_and_the_document_names_them() {
    let run = pvec(&["bench-open", "--iterations", "5", "--format", "json", &fixture()]);
    let doc = check_schema(&run);
    let volatile: Vec<&str> = doc.at("result.volatile").arr().iter().map(V::str).collect();
    assert_eq!(volatile, ["timings_ns"]);
    assert_eq!(doc.at("result.iterations").num(), 5.0);
    let cli = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CLI.md")).unwrap();
    assert!(cli.contains("timings_ns"), "docs/CLI.md lists the volatile field");
}

#[test]
fn the_cli_document_names_every_subcommand_exit_code_and_result_field() {
    let cli = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/CLI.md")).unwrap();
    for sub in
        ["inspect", "validate", "dump-manifest", "capabilities", "compare-step", "tessellate", "bench-open"]
    {
        assert!(cli.contains(&format!("### `{sub}`")), "docs/CLI.md has no section for {sub}");
    }
    for code in ["| 0 |", "| 1 |", "| 2 |", "| 3 |", "| 64 |", "| 65 |", "| 70 |"] {
        assert!(cli.contains(code), "docs/CLI.md does not give exit code {code}");
    }
    let good = fixture();
    for sub in ["inspect", "validate", "dump-manifest", "capabilities", "tessellate"] {
        let doc = pvec(&[sub, "--format", "json", &good]).json();
        for key in doc.get("result").keys() {
            assert!(cli.contains(&format!("`{key}`")), "docs/CLI.md does not document {sub} field {key}");
        }
    }
}
