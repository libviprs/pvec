//! Issue R2.3 (pvec#222): `INTERCHANGE.md` has the raster chapter, the scope lines of chapters 1, 4, 5 and 6
//! point at it, every export has its row, and the taxonomy names the PMTiles raster passthrough as `Exact`
//! for bytes and says why pixels have no `BoundedApproximation`.

use pvec_checks::repo_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of one `## ` chapter, from its heading to the next chapter.
fn chapter<'a>(doc: &'a str, heading: &str) -> &'a str {
    let at = doc.find(heading).unwrap_or_else(|| panic!("no chapter {heading}"));
    let rest = &doc[at + heading.len()..];
    let end = rest.find("\n## ").map_or(rest.len(), |e| e);
    &rest[..end]
}

#[test]
fn the_interchange_document_has_the_raster_chapter() {
    let d = read("docs/INTERCHANGE.md");
    let ch = flat(chapter(&d, "## 8. Rasters"));
    for needle in [
        "What each export does with a raster",
        "### PMTiles passthrough, out",
        "### PMTiles passthrough, in",
        "### Fidelity",
        "### Reports",
        "vnd.pvec",
        "ceil(log2(max(width, height) / tile_edge))",
        "tile type 2",
        "`Raw` archive is refused",
        "Sparse stays sparse",
        "RASTER-03",
        "viprs pmtiles extract",
        "libviprs/libviprs#1328",
    ] {
        assert!(ch.contains(needle), "chapter 8 does not say `{needle}`");
    }
}

#[test]
fn every_export_has_a_row_that_says_carries_drops_or_refuses() {
    let d = read("docs/INTERCHANGE.md");
    let ch = chapter(&d, "## 8. Rasters");
    let table: Vec<&str> = ch.lines().filter(|l| l.starts_with("| ")).collect();
    for export in [
        "PMTiles raster passthrough",
        "PMTiles raster import",
        "STEP AP242 Part 21",
        "GML",
        "GeoPackage feature tables",
        "MVT tiles",
        "PMTiles vector packaging",
    ] {
        let row = table
            .iter()
            .find(|l| l.starts_with(&format!("| {export}")))
            .unwrap_or_else(|| panic!("no row for {export}"));
        assert!(
            row.contains("**carries**") || row.contains("**drops**") || row.contains("**refuses**"),
            "the row for {export} does not say what happens to a raster: {row}"
        );
    }
    assert!(table.iter().any(|l| l.contains("**refuses**")), "a refusal row exists");
}

#[test]
fn the_scope_lines_of_chapters_1_4_5_and_6_point_at_the_raster_chapter() {
    let d = read("docs/INTERCHANGE.md");
    for (heading, must) in [
        ("### 1.8 Not covered", "chapter 8"),
        ("### Not done\n\nReading GeoPackage", "chapter 8"),
        ("### Out of scope here\n\nPMTiles packaging", "chapter 8"),
        ("### Not done\n\nServing, CDN", "chapter 8"),
    ] {
        let at = d.find(heading).unwrap_or_else(|| panic!("no section {heading:?}"));
        let end = d[at + heading.len()..].find("\n#").map_or(d.len(), |e| at + heading.len() + e);
        assert!(flat(&d[at..end]).contains(must), "{heading:?} does not point at {must}");
    }
    let intro = flat(&d[..d.find("## 1.").unwrap()]);
    assert!(intro.contains("chapter 8"), "the introduction lists chapter 8");
}

#[test]
fn the_taxonomy_names_the_passthrough_exact_for_bytes_and_has_no_pixel_bound() {
    let t = flat(&read("docs/FIDELITY_TAXONOMY.md"));
    let at = t.find("## Raster tiles").expect("a raster section");
    let s = &t[at..];
    assert!(s.contains("`Exact` for **bytes**"));
    assert!(s.contains("No `BoundedApproximation` exists for pixels"));
    assert!(s.contains("bound in drawing units"));
}

#[test]
fn chapter_7_documents_the_raster_report_object() {
    let d = read("docs/INTERCHANGE.md");
    let ch = flat(chapter(&d, "## 7. Fidelity reports"));
    for key in ["rasters", "count", "exported", "unsupported", "raster", "reason"] {
        assert!(ch.contains(&format!("`{key}`")), "chapter 7 does not document `{key}`");
    }
    assert!(ch.contains("Version 1.1 adds the `rasters` object"));
}
