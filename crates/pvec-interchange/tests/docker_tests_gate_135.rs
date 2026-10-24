//! pvec#135: the docker-backed tests run in the merge gate, so the `#[ignore]` reason and the helper
//! script have to point at the fleet stage that runs them rather than at a hand run on one host.

use std::fs;
use std::path::Path;

fn read(rel: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn the_ignore_reason_names_the_fleet_stage() {
    let t = read("tests/gml_21_schema.rs");
    let line = t.lines().find(|l| l.starts_with("#[ignore")).expect("an #[ignore] line");
    assert!(line.contains("docker-tests"), "the ignore reason should name the gate-fleet stage: {line}");
}

#[test]
fn the_schema_script_says_the_gate_runs_it() {
    let s = read("../../tools/schema-validate.sh");
    assert!(s.contains("gate-fleet.sh local-ci pvec"), "schema-validate.sh should point at the gate");
}
