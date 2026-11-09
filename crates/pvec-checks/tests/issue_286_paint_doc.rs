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

#[test]
fn p2_the_registry_docs_and_matrix_know_capability_25() {
    let generated = render_markdown(&Registry::v1());
    let row = generated.lines().find(|l| l.starts_with("| 25 | `fill_paint` |")).expect("a row for 25");
    assert!(row.contains("| component | 1.0.0 |") && row.contains("ADR 0021"), "{row}");
    let d = read("docs/COMPONENT_STREAMS.md");
    for needle in ["`fill_paint`", "| 25 |", "| COMP-23 |", "| COMP-24 |", "| COMP-27 |"] {
        assert!(d.contains(needle), "docs/COMPONENT_STREAMS.md does not say {needle:?}");
    }
    let l = read("docs/SECURITY_LIMITS.md");
    for needle in ["25 `fill_paint`", "4,096 gradients", "1,024 tilings", "8,192 points"] {
        assert!(l.contains(needle), "docs/SECURITY_LIMITS.md does not say {needle:?}");
    }
    assert!(read("tests/compat/matrix.txt").contains("case old-reader-fill-paint-file-open-degraded"));
}

#[test]
fn p2_the_thirteenth_fuzz_target_is_named_in_the_docs_and_the_script() {
    let doc = read("docs/FUZZING.md");
    assert!(doc.contains("The thirteen targets") && doc.contains("`fuzz_fill_paint` is the thirteenth"));
    assert!(read("tools/fuzz.sh").contains("fuzz_fill_paint"));
    assert!(repo_root().join("fuzz/fuzz_targets/fuzz_fill_paint.rs").is_file());
}

#[test]
fn p3_the_registry_docs_and_matrix_know_capability_26() {
    let generated = render_markdown(&Registry::v1());
    let row = generated.lines().find(|l| l.starts_with("| 26 | `clip` |")).expect("a row for 26");
    assert!(row.contains("| component | 1.0.0 |") && row.contains("ADR 0021"), "{row}");
    let d = read("docs/COMPONENT_STREAMS.md");
    for needle in ["`clip`", "| 26 |", "| COMP-25 |", "| COMP-26 |"] {
        assert!(d.contains(needle), "docs/COMPONENT_STREAMS.md does not say {needle:?}");
    }
    let l = read("docs/SECURITY_LIMITS.md");
    for needle in ["26 `clip`", "65,536 clips", "8 deep"] {
        assert!(l.contains(needle), "docs/SECURITY_LIMITS.md does not say {needle:?}");
    }
    assert!(read("tests/compat/matrix.txt").contains("case old-reader-clip-file-open-degraded"));
}

/// The text of section 5.4, up to the next heading.
fn clip_section() -> String {
    let d = read("docs/PAINT.md");
    let from = d.find("### 5.4 Clips").expect("section 5.4");
    let rest = &d[from + 4..];
    rest[..rest.find("\n### ").unwrap_or(rest.len())].to_string()
}

#[test]
fn the_stencil_scheme_in_5_4_is_one_bit_per_level_with_a_write_mask() {
    let s = clip_section();
    for needle in ["one bit per level", "write mask", "compare mask", "nests at most 8 deep"] {
        assert!(s.contains(needle), "section 5.4 does not say {needle:?}");
    }
}

#[test]
fn the_stencil_scheme_does_not_mix_an_increment_with_an_invert_on_the_same_bits() {
    let s = clip_section();
    assert!(!s.contains("an increment where"), "the old increment-and-invert scheme is still in 5.4");
    assert!(!s.contains("stencil test of equal to the depth"), "the old depth test is still in 5.4");
}

#[test]
fn the_adr_names_the_same_stencil_scheme_as_5_4() {
    let a = read("docs/adr/0021-paint-model.md");
    assert!(a.contains("one stencil bit per clip level"), "ADR 0021 does not name the scheme");
    assert!(a.contains("write mask"), "ADR 0021 does not name the write mask");
}
