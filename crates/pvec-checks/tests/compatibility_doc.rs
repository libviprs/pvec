//! The compatibility rules and the registry snapshot (issue P7.6).

use pvec_capabilities::{
    DEPRECATED_MIN_MINORS, Registry, Snapshot, WRITE_DISABLED_MIN_MINORS, lint_transition,
};
use pvec_checks::repo_root;
use std::fs;

fn doc() -> String {
    fs::read_to_string(repo_root().join("docs/COMPATIBILITY.md")).expect("docs/COMPATIBILITY.md exists")
}

#[test]
fn the_committed_snapshot_is_exactly_the_generated_one() {
    let path = repo_root().join("docs/capabilities.toml");
    let generated = Snapshot::from_registry(&Registry::v1()).render();
    if std::env::var_os("PVEC_BLESS_CAPABILITIES").is_some() {
        fs::write(&path, &generated).unwrap();
    }
    let committed = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "docs/capabilities.toml differs from the registry; regenerate it with PVEC_BLESS_CAPABILITIES=1 \
         cargo test -p pvec-checks --test compatibility_doc"
    );
}

#[test]
fn the_committed_snapshot_passes_the_lint_against_the_base_branch() {
    // CI hands the snapshot of the base branch in: git show origin/main:docs/capabilities.toml
    let committed = Snapshot::parse(&fs::read_to_string(repo_root().join("docs/capabilities.toml")).unwrap())
        .expect("the committed snapshot parses");
    assert_eq!(lint_transition(&committed, &committed), vec![]);
    if let Some(path) = std::env::var_os("PVEC_CAPABILITIES_BASE") {
        let base = Snapshot::parse(&fs::read_to_string(path).unwrap()).expect("the base snapshot parses");
        let findings = lint_transition(&base, &committed);
        assert!(findings.is_empty(), "the registry change breaks the compatibility rules: {findings:#?}");
    }
}

#[test]
fn the_document_gives_the_exact_behaviour_for_every_row_of_the_decision_table() {
    let d = doc();
    for needle in [
        "## Decision table",
        "Refusal::MissingRequired",
        "NEGOTIATE-01",
        "OpenDegraded",
        "SkipReason::UnknownBlockKind",
        "SkipReason::NotImplemented",
        "version_gaps",
        "VersionGap",
        "Refusal::IncompatibleMajor",
        "NEGOTIATE-02",
        "SkipReason::NewerMajor",
        "Refusal::ReaderTooOld",
        "NEGOTIATE-03",
        "minimum reader",
    ] {
        assert!(d.contains(needle), "docs/COMPATIBILITY.md does not mention {needle}");
    }
}

#[test]
fn the_document_states_the_lifecycle_the_lint_enforces() {
    let d = doc();
    for needle in [
        "active",
        "deprecated",
        "write-disabled",
        "removed",
        "major version",
        "`unstable`",
        "not a conformant v1 file",
    ] {
        assert!(d.contains(needle), "docs/COMPATIBILITY.md does not mention {needle}");
    }
    assert!(d.contains(&format!("at least {DEPRECATED_MIN_MINORS} minor releases as `deprecated`")));
    assert!(d.contains(&format!("at least {WRITE_DISABLED_MIN_MINORS} minor releases as `write-disabled`")));
}

#[test]
fn the_document_and_the_matrix_agree_on_the_raster_row() {
    let d = doc();
    for needle in ["Effect::Unrastered", "NEGOTIATE-06", "capability-owned", "`min_reader` at 1.0.0"] {
        assert!(d.contains(needle), "docs/COMPATIBILITY.md does not mention {needle}");
    }
    let conformance = fs::read_to_string(repo_root().join("docs/CONFORMANCE.md")).unwrap();
    assert!(
        conformance.contains("tests/compat/raster-degraded.pvec") && conformance.contains("capability 21"),
        "the forward-compatible gate names the raster fixture"
    );
    assert!(repo_root().join("tests/compat/raster-degraded.pvec").is_file(), "the fixture is checked in");
    let matrix = fs::read_to_string(repo_root().join("tests/compat/matrix.txt")).unwrap();
    assert!(
        matrix.contains("case old-reader-raster-file-open-degraded"),
        "the 1.0.0 reader, raster file row"
    );
    let pairs = matrix.lines().filter(|l| l.starts_with("case ")).count();
    let words = ["eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen"];
    assert!(
        d.contains(&format!("outcome of {} reader and file pairs", words[pairs - 11])),
        "the document counts {pairs} pairs differently"
    );
}

#[test]
fn the_committed_snapshot_has_entry_21_raster_optional_at_1_1_0() {
    let text = fs::read_to_string(repo_root().join("docs/capabilities.toml")).unwrap();
    assert!(text.starts_with("# ") && text.contains("format_version = \"1.2.0\""));
    let s = Snapshot::parse(&text).unwrap();
    let e = s.entries.iter().find(|e| e.id == 21).expect("entry 21");
    assert_eq!((e.name.as_str(), e.class.as_str(), e.requirement.as_str()), ("raster", "raster", "optional"));
    assert_eq!(e.min_reader.to_string(), "1.1.0");
    assert_eq!(e.status_since.to_string(), "1.1.0");
}

#[test]
fn the_committed_snapshot_has_entry_22_calibration_optional_at_1_2_0() {
    let text = fs::read_to_string(repo_root().join("docs/capabilities.toml")).unwrap();
    let s = Snapshot::parse(&text).unwrap();
    assert_eq!(s.format_version.to_string(), "1.2.0");
    let e = s.entries.iter().find(|e| e.id == 22).expect("entry 22");
    assert_eq!(
        (e.name.as_str(), e.class.as_str(), e.requirement.as_str()),
        ("calibration", "calibration", "optional")
    );
    assert_eq!(
        (e.min_reader.to_string(), e.status_since.to_string()),
        ("1.2.0".to_string(), "1.2.0".to_string())
    );
}

#[test]
fn the_document_has_the_calibration_row_and_the_matrix_its_three_cases() {
    let d = doc();
    for needle in [
        "kind 11, capability 22",
        "Effect::Uncalibrated",
        "`FORMAT_VERSION`, 1.2.0 today",
        "calibration) cannot be declared required",
    ] {
        assert!(d.contains(needle), "docs/COMPATIBILITY.md does not say {needle:?}");
    }
    let matrix = fs::read_to_string(repo_root().join("tests/compat/matrix.txt")).unwrap();
    for case in [
        "case old-reader-calibrated-file-open-degraded",
        "case raster-reader-calibrated-raster-file-open-degraded",
        "case calibration-declared-required-is-refused",
    ] {
        assert!(matrix.contains(case), "the matrix has no `{case}`");
    }
}
