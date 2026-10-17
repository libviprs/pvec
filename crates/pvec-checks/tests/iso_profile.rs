//! Acceptance checks for `docs/ISO_PROFILE.md` (issue P1.1).

use pvec_checks::*;
use std::fs;

fn profile() -> String {
    fs::read_to_string(repo_root().join("docs/ISO_PROFILE.md")).expect("docs/ISO_PROFILE.md exists")
}

/// The v1 capability list from the epic, as profile section titles.
const CAPABILITIES: [&str; 13] = [
    "Cartesian point",
    "Direction and vector",
    "Axis placement 2D",
    "Line",
    "Polyline",
    "Circle",
    "Ellipse",
    "Trimmed curve",
    "Composite curve",
    "Bezier",
    "B-spline with knots",
    "Rational B-spline",
    "2D affine transform",
];

#[test]
fn one_entry_per_v1_capability() {
    let entries = parse_profile(&profile());
    for cap in CAPABILITIES {
        assert!(entries.iter().any(|e| e.title.contains(cap)), "profile has no section for {cap}");
    }
    assert_eq!(entries.len(), CAPABILITIES.len(), "unexpected extra numbered sections");
}

#[test]
fn all_seven_fields_are_present_in_order_and_populated() {
    for entry in parse_profile(&profile()) {
        let names: Vec<&str> = entry.fields.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, REQUIRED_FIELDS, "fields of '{}'", entry.title);
        for (k, v) in &entry.fields {
            assert!(!v.is_empty(), "'{}' field '{k}' is empty", entry.title);
            assert!(!v.to_uppercase().contains("TBD"), "'{}' field '{k}' contains TBD", entry.title);
        }
    }
}

#[test]
fn every_entry_names_a_construct_and_a_part21_form() {
    for entry in parse_profile(&profile()) {
        let construct = &entry.fields[0].1;
        assert!(construct.contains('`'), "'{}' names no construct", entry.title);
        let form = &entry.fields[5].1;
        // A Part 21 form is an upper-case entity keyword followed by an opening parenthesis.
        let has_form = form.split('`').any(|code| {
            let code = code.trim_start_matches('(').trim();
            let kw: String = code
                .chars()
                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            kw.len() >= 4 && code[kw.len()..].starts_with('(')
        });
        assert!(has_form, "'{}' has no concrete Part 21 form: {form}", entry.title);
    }
}

#[test]
fn every_entry_says_what_a_decoder_rejects() {
    for entry in parse_profile(&profile()) {
        assert!(
            entry.body.contains("A decoder rejects"),
            "'{}' does not say what a decoder rejects",
            entry.title
        );
    }
}

#[test]
fn unsupported_list_covers_the_required_classes_with_reasons() {
    let text = profile();
    let section =
        text.split("## Unsupported Part 42 constructs, by class").nth(1).expect("unsupported section exists");
    for class in ["Surfaces", "Topology", "3D-only", "Swept geometry"] {
        let row = section
            .lines()
            .find(|l| l.starts_with(&format!("| {class}")))
            .unwrap_or_else(|| panic!("no row for {class}"));
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        assert!(cells.len() >= 4 && cells[3].len() > 10, "{class} has no stated reason");
    }
}

#[test]
fn the_approved_claim_is_present_verbatim() {
    assert!(profile().contains(APPROVED_CLAIM));
}

#[test]
fn no_file_claims_iso_conformance() {
    let root = repo_root();
    let mut offenders = Vec::new();
    for rel in text_files(&root) {
        // The checker's own sources have to name the forbidden words to detect them.
        if rel.starts_with("crates/pvec-checks") {
            continue;
        }
        let text = fs::read_to_string(root.join(&rel)).unwrap_or_default();
        for hit in forbidden_claims(&text) {
            offenders.push(format!("{}: {hit}", rel.display()));
        }
    }
    assert!(offenders.is_empty(), "forbidden ISO conformance wording:\n{}", offenders.join("\n"));
}

#[test]
fn no_em_dashes_in_docs() {
    let root = repo_root();
    for rel in text_files(&root) {
        if !rel.starts_with("docs") || rel.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let text = fs::read_to_string(root.join(&rel)).unwrap_or_default();
        assert!(!text.contains('\u{2014}'), "{} contains an em dash", rel.display());
    }
}
