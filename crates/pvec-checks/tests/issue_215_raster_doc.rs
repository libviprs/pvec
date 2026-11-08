//! Issue #215 (R1.1): the raster specification wording. `docs/RASTER.md` exists and carries the layout, the
//! conversions, the test vector and the rule table; the documents around it say the same thing about kinds 9 and 10.

use pvec_checks::repo_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|_| panic!("{rel} exists"))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn raster_md_carries_the_layouts_the_conversions_and_the_test_vector() {
    let d = flat(&read("docs/RASTER.md"));
    for needle in [
        "little endian",
        "[a b c d tx ty]",
        "top-left corner",
        "d < 0",
        "visible_box",
        "tile_edge",
        "Hilbert",
        "subtype = level * 1024 + chunk",
        "codec 0",
        "GeoTransform",
        "SVG order",
        "IMAGE",
        "pixel-centre",
        "CTM",
        "Test vector",
        "expected corners",
    ] {
        assert!(d.contains(needle), "docs/RASTER.md does not say {needle:?}");
    }
    for n in 1..=7 {
        assert!(d.contains(&format!("| RASTER-{n:02} |")), "docs/RASTER.md has no RASTER-{n:02} row");
    }
    assert!(!d.contains("| RASTER-08 |"));
}

#[test]
fn container_md_reserves_kinds_9_and_10_for_capability_21_and_hoists_over_required_only() {
    let d = flat(&read("docs/CONTAINER.md"));
    assert!(
        d.contains("| 9 | raster_index |") && d.contains("| 10 | raster_tiles |"),
        "kinds 9 and 10 are not in the table"
    );
    assert!(d.contains("capability-owned by 21"), "the kinds do not name capability 21");
    assert!(
        d.contains("the highest minimum reader of the required capabilities"),
        "4.3 does not say the min_reader hoist runs over required capabilities only"
    );
    assert!(d.contains("adds information about the drawing"), "5.4 item 1 still says entities");
    assert!(d.contains("first used by capability 21"), "5.4 item 2 is not marked as used for the first time");
}

#[test]
fn interchange_md_points_its_raster_scope_lines_at_the_raster_chapter() {
    let d = read("docs/INTERCHANGE.md");
    assert!(d.matches("RASTER.md").count() >= 3, "chapters 4, 5 and 6 do not all point at RASTER.md");
    assert!(d.contains("ADR 0018"));
}

#[test]
fn the_readme_links_raster_md_and_extensions_md_says_raster_is_not_a_geometry_extension() {
    assert!(read("docs/README.md").contains("(RASTER.md)"));
    assert!(flat(&read("docs/EXTENSIONS.md")).contains("raster is not a geometry extension"));
}

#[test]
fn migration_and_the_freeze_record_carry_the_new_wording() {
    assert!(flat(&read("docs/MIGRATION.md")).contains("1.1.0"), "MIGRATION.md does not list 1.1.0");
    let f = read("docs/FREEZE-v1.md");
    let row = f.lines().find(|l| l.starts_with("| extension rules")).expect("the extension rules row");
    assert!(row.contains("a capability may own block kinds") && row.contains("2026-"), "row: {row}");
    assert!(f.contains("capability format version 1.3.0"));
}
