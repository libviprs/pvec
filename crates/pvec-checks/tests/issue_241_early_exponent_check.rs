//! Issue 241: the exponent-number checks run first in the pre-push gate and in the release script, so a doc or test
//! literal like `1e-9` fails before the build and tag steps, not at the end of the full workspace tests.
#![allow(missing_docs)]

use pvec_checks::repo_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_early_check_runs_both_exponent_tests_by_name() {
    let s = read("tools/exponent-check.sh");
    assert!(s.contains("--test tolerance_literals"), "the code literal check is missing");
    assert!(s.contains("--test issue_67_docs"), "the doc literal check is missing");
    assert!(s.contains("only_the_numeric_policy_defines_a_tolerance"), "the doc check is not selected by name");
}

fn position_of_check_before(script: &str, first_build_step: &str) {
    let s = read(script);
    let check = s.find("exponent-check.sh").unwrap_or_else(|| panic!("{script} never runs tools/exponent-check.sh"));
    let build = s.find(first_build_step).unwrap_or_else(|| panic!("{script} has no `{first_build_step}`"));
    assert!(check < build, "{script} runs the exponent check after `{first_build_step}`");
}

#[test]
fn the_pre_push_gate_runs_it_before_fmt_clippy_and_tests() {
    position_of_check_before("tools/gate.sh", "cargo fmt");
}

#[test]
fn the_release_script_runs_it_before_the_wasm_build() {
    position_of_check_before("tools/wasm-release.sh", "cargo build --release");
}
