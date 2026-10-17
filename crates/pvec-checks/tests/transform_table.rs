//! The promotion table of docs/ISO_PROFILE.md (issue P3.8): every row names the exact target
//! capability from the registry, by name and by id.

use pvec_capabilities::*;
use pvec_checks::*;
use std::fs;

fn table() -> Vec<Vec<String>> {
    let text = fs::read_to_string(repo_root().join("docs/ISO_PROFILE.md")).unwrap();
    let start = text
        .find("### Promotion table")
        .expect("ISO_PROFILE.md has a '### Promotion table' subsection in section 13");
    let body = &text[start..];
    let end = body[5..].find("\n#").map_or(body.len(), |e| e + 5);
    body[..end]
        .lines()
        .filter(|l| l.starts_with('|') && !l.starts_with("|---") && !l.contains("| input |"))
        .map(|l| l.trim_matches('|').split('|').map(|c| c.trim().to_string()).collect())
        .collect()
}

#[test]
fn the_table_has_the_rows_the_issue_requires_and_the_exact_results() {
    let rows = table();
    assert!(rows.len() >= 10, "expected at least ten rows, found {}", rows.len());
    assert!(
        rows.iter().all(|r| r.len() == 4),
        "every row has input, transform, exact result and target capability"
    );
    let find = |input: &str, tr: &str| {
        rows.iter()
            .find(|r| (r[0] == input || r[0].starts_with(&format!("{input} "))) && r[1].contains(tr))
            .unwrap_or_else(|| panic!("no row for {input} under {tr}"))
    };
    assert!(find("circle", "non-uniform scale")[2].contains("ellipse"));
    assert!(find("ellipse", "shear")[2].contains("ellipse"));
    assert!(find("arc", "any non-degenerate affine")[2].contains("trim parameters"));
    assert!(find("B-spline", "any non-degenerate affine")[2].contains("knots and weights preserved"));
    assert!(
        rows.iter().any(|r| r[2].starts_with("refused")),
        "the too-eccentric case is a documented refusal"
    );
}

#[test]
fn every_row_names_a_registered_target_capability_by_name_and_id() {
    let r = Registry::v1();
    for row in table() {
        let target = &row[3];
        if row[2].starts_with("refused") {
            assert!(target.starts_with("none"), "a refusal has no target: {row:?}");
            continue;
        }
        // `name` (id)
        let name = target.split('`').nth(1).unwrap_or_else(|| panic!("no capability name in {row:?}"));
        let id: u32 = target
            .split(['(', ')'])
            .nth(1)
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or_else(|| panic!("no capability id in {row:?}"));
        let c = r.by_name(name).unwrap_or_else(|| panic!("{name} is not in the registry ({row:?})"));
        assert_eq!(c.id.get(), id, "{name}");
    }
}

#[test]
fn the_table_does_not_promise_a_polyline_fallback() {
    let doc = fs::read_to_string(repo_root().join("docs/ISO_PROFILE.md")).unwrap();
    let start = doc.find("### Promotion table").unwrap();
    let section = doc[start..].to_lowercase();
    assert!(section.contains("never"), "the section says what is never done");
    assert!(
        !table().iter().any(|r| r[2].contains("polyline") && !r[0].contains("polyline")),
        "no row turns a curve into a polyline"
    );
}
