//! Issue P8.7 (pvec#66): the v1 freeze record. `docs/FREEZE-v1.md` says what is frozen and what is not yet, and
//! these tests turn red when a frozen constant or the registry snapshot moves, or when a row claims a freeze
//! its gate does not support.
#![allow(missing_docs)]

use pvec_capabilities::{FORMAT_VERSION, Version};
use pvec_container::Header;
use std::fs;
use std::path::PathBuf;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn freeze() -> String {
    read("docs/FREEZE-v1.md")
}

/// The rows of the artefact table: artefact, ADR, gate, result file, status.
fn rows() -> Vec<[String; 5]> {
    freeze()
        .lines()
        .filter(|l| l.starts_with("| ") && !l.starts_with("| Frozen artefact") && !l.starts_with("|---"))
        .filter_map(|l| {
            let cells: Vec<String> = l.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect();
            <[String; 5]>::try_from(cells).ok()
        })
        .collect()
}

#[test]
fn the_table_has_one_row_per_frozen_artefact_and_each_names_an_adr_and_a_result() {
    let artefacts = [
        "semantic profile",
        "numeric policy",
        "physical container",
        "internal layout",
        "spatial index",
        "progressive tiers",
        "extension rules",
        "capability registry",
        "security and resource limits",
        "fixture corpus revision",
        "C/FFI answer",
    ];
    let rows = rows();
    for want in artefacts {
        let row = rows.iter().find(|r| r[0].starts_with(want)).unwrap_or_else(|| panic!("no row for {want}"));
        assert!(
            row[1].contains("ADR") || row[1].contains("revision pin"),
            "{want}: no deciding ADR in `{}`",
            row[1]
        );
        assert!(!row[3].is_empty(), "{want}: no benchmark result file");
        assert!(["FROZEN", "NOT FROZEN"].contains(&row[4].as_str()), "{want}: status `{}`", row[4]);
    }
    assert_eq!(rows.len(), artefacts.len());
}

#[test]
fn a_row_is_frozen_only_when_its_result_file_exists_and_records_no_failure() {
    for row in rows() {
        let files: Vec<&str> = row[3]
            .split(';')
            .map(str::trim)
            .filter(|f| f.starts_with("docs/") || f.starts_with("tests/") || f.starts_with("crates/"))
            .collect();
        if row[4] == "FROZEN" {
            assert!(!files.is_empty(), "{}: frozen with no checkable result file", row[0]);
            for f in files {
                // a benchmark report records its verdicts as text; a test file is a gate by passing
                assert!(repo().join(f).exists(), "{}: {f} does not exist", row[0]);
                if f.starts_with("docs/bench/") {
                    assert!(!read(f).contains(": FAIL"), "{}: frozen, but {f} records a failed gate", row[0]);
                }
            }
        } else {
            assert!(row[3].len() > 3, "{}: not frozen, and the row does not say why", row[0]);
        }
    }
}

#[test]
fn the_five_bench_reports_are_listed_in_a_checklist_with_their_state() {
    let doc = freeze();
    for report in ["STEP_COMPARISON.md", "LAYOUT.md", "GEOMETRY.md", "PROGRESSIVE.md", "TUNING.md"] {
        let line = doc.lines().find(|l| l.starts_with("- [") && l.contains(report));
        let line = line.unwrap_or_else(|| panic!("the checklist never mentions {report}"));
        let exists = repo().join("docs/bench").join(report).exists();
        assert_eq!(line.starts_with("- [x]"), exists, "{report}: checklist box disagrees with the file");
    }
}

#[test]
fn the_v1_constants_are_the_frozen_values() {
    assert_eq!((Header::MAJOR, Header::MINOR), (1, 0), "container version");
    assert_eq!(FORMAT_VERSION, Version::new(1, 3, 0), "capability format version");
    let wasm = read("crates/pvec-wasm/src/abi.rs");
    assert!(wasm.contains("pub const WASM_ABI_VERSION: u32 = 2;"), "wasm_abi_version");
    let doc = freeze();
    assert!(doc.contains("container 1.0"));
    assert!(doc.contains("`wasm_abi_version` 2"));
}

#[test]
fn the_registry_snapshot_matches_the_digest_recorded_in_the_freeze_record() {
    let digest = pvec_corpus::sha256::hex_digest(read("docs/capabilities.toml").as_bytes());
    assert!(
        freeze().contains(&digest),
        "docs/capabilities.toml changed after the freeze record: its digest is now {digest}"
    );
}

#[test]
fn the_corpus_lock_matches_the_digest_recorded_in_the_freeze_record() {
    let digest =
        pvec_corpus::sha256::hex_digest(read("crates/pvec-bench/benches/corpus/corpus.lock.json").as_bytes());
    assert!(freeze().contains(&digest), "corpus.lock.json digest is now {digest}");
}

#[test]
fn the_record_makes_no_claim_of_formal_iso_conformance_and_says_what_is_deferred() {
    let doc = freeze();
    assert!(doc.contains(
        "aligned to named ISO 10303-42:2025 constructs, validated through ISO 10303-21 interchange"
    ));
    assert!(doc.contains("no claim of formal Part 42 conformance"));
    for banned in ["conformant", "compliant", "fully conforms", "is certified"] {
        assert!(!doc.to_lowercase().contains(banned), "the record says `{banned}`");
    }
    assert!(doc.contains("Unsupported Part 42 features"), "the unsupported-feature list is not reproduced");
    assert!(doc.contains("no C header"), "the C/FFI deferral is not recorded");
    assert!(doc.contains("pvec#190"), "the outstanding 4-hour fuzz run is not recorded");
    assert!(doc.contains("pvec#192"), "the benchmark failures are not referenced");
    assert!(doc.contains("EPIC L"), "the EPIC L decision is not recorded");
    assert!(doc.contains("v1 is not tagged"), "the record must say nothing is tagged or released");
}

#[test]
fn the_migration_note_has_its_three_sections() {
    let m = read("docs/MIGRATION.md");
    for h in
        ["## What a v1.x reader must keep working", "## What may change within v1.x", "## What requires v2"]
    {
        assert!(m.contains(h), "MIGRATION.md has no `{h}`");
    }
}

#[test]
fn the_extension_rules_row_carries_the_dated_line_that_a_capability_may_own_block_kinds() {
    let row = freeze().lines().find(|l| l.starts_with("| extension rules")).expect("the row").to_string();
    assert!(row.contains("a capability may own block kinds"), "{row}");
    assert!(row.contains("ADR 0018") && row.contains("2026-"), "{row}");
}
