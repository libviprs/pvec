//! Issue 286, P1: the docs and the matrix say what the registry says about capabilities 23 and 24, and
//! `docs/COMPONENT_STREAMS.md` lists the layouts and the new rules. The rule table is the codec's own list.

use pvec_capabilities::{FORMAT_VERSION, Registry, Version, render_markdown};
use pvec_checks::repo_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

#[test]
fn the_format_version_is_1_3_0_and_the_snapshot_says_so() {
    assert_eq!(FORMAT_VERSION, Version::new(1, 3, 0));
    assert!(read("docs/capabilities.toml").contains("format_version = \"1.3.0\""));
    assert!(read("docs/COMPATIBILITY.md").contains("`FORMAT_VERSION`, 1.3.0 today"));
}

#[test]
fn the_generated_capabilities_document_has_rows_23_and_24() {
    let generated = render_markdown(&Registry::v1());
    for (id, name) in [(23, "paint_order"), (24, "stroke_paint")] {
        let row = generated
            .lines()
            .find(|l| l.starts_with(&format!("| {id} | `{name}` |")))
            .unwrap_or_else(|| panic!("a row for {id}"));
        assert!(row.contains("| component | 1.0.0 |") && row.contains("| optional |"), "{row}");
        assert!(row.contains("| 1.0.0 |") && row.contains("ADR 0021"), "{row}");
    }
}

#[test]
fn component_streams_md_lists_the_two_layouts_and_the_new_rules() {
    let d = read("docs/COMPONENT_STREAMS.md");
    for needle in [
        "`paint_order`",
        "`stroke_paint`",
        "| 23 |",
        "| 24 |",
        "| COMP-21 |",
        "| COMP-22 |",
        "| COMP-28 |",
        "docs/PAINT.md",
    ] {
        assert!(d.contains(needle), "docs/COMPONENT_STREAMS.md does not say {needle:?}");
    }
}

#[test]
fn paint_md_lists_comp_28_the_rule_for_entities_that_do_not_ascend() {
    assert!(read("docs/PAINT.md").contains("COMP-28"));
}

#[test]
fn the_matrix_has_cases_for_an_old_reader_a_current_reader_and_a_newer_major_of_23() {
    let m = read("tests/compat/matrix.txt");
    for case in [
        "case old-reader-paint-file-open-degraded",
        "case current-reader-paint-file-open",
        "case reader-meets-a-newer-major-of-the-paint-order-stream",
    ] {
        assert!(m.contains(case), "tests/compat/matrix.txt has no {case:?}");
    }
}

#[test]
fn security_limits_lists_the_ceilings_of_the_two_streams() {
    let d = read("docs/SECURITY_LIMITS.md");
    for needle in ["paint_order", "1,048,576 runs", "stroke_paint", "65,536 colours"] {
        assert!(d.contains(needle), "docs/SECURITY_LIMITS.md does not say {needle:?}");
    }
}
