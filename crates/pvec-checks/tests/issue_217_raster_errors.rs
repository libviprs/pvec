//! Issue 217: the error table of `RasterError` in `docs/RASTER.md` is rendered from `RASTER_ERRORS`, the way the
//! `DIR-NN` and `COMP-NN` tables are kept in step, and `pvec-components` stays free of an image or geometry crate.

use pvec_checks::repo_root;
use pvec_components::RASTER_ERRORS;
use std::fs;

const BEGIN: &str = "<!-- raster-errors:begin -->";
const END: &str = "<!-- raster-errors:end -->";

fn rendered() -> String {
    let mut s = String::from("| rule | error | meaning |\n|---|---|---|\n");
    for (rule, name, meaning) in RASTER_ERRORS {
        s.push_str(&format!("| {rule} | `{name}` | {meaning} |\n"));
    }
    s
}

#[test]
fn the_error_table_in_raster_md_is_exactly_the_rendered_one() {
    let path = repo_root().join("docs/RASTER.md");
    let doc = fs::read_to_string(&path).expect("docs/RASTER.md exists");
    let (head, rest) = doc.split_once(BEGIN).expect("RASTER.md has the begin marker of the error table");
    let (old, tail) = rest.split_once(END).expect("RASTER.md has the end marker of the error table");
    let want = format!("\n{}", rendered());
    if std::env::var_os("PVEC_BLESS_RASTER").is_some() {
        fs::write(&path, format!("{head}{BEGIN}{want}{END}{tail}")).unwrap();
        return;
    }
    assert!(
        old == want,
        "the error table of docs/RASTER.md differs from RASTER_ERRORS; regenerate it with PVEC_BLESS_RASTER=1 \
         cargo test -p pvec-checks --test issue_217_raster_errors"
    );
}

#[test]
fn every_raster_rule_the_document_defines_has_its_own_row_or_is_a_later_stage() {
    let doc = fs::read_to_string(repo_root().join("docs/RASTER.md")).unwrap();
    for rule in ["RASTER-01", "RASTER-04", "RASTER-05", "RASTER-07"] {
        assert!(RASTER_ERRORS.iter().any(|(r, _, _)| *r == rule), "{rule} has no error");
        assert!(doc.contains(&format!("| {rule} |")), "{rule} is not in the document");
    }
}

#[test]
fn pvec_components_depends_on_no_image_and_no_geometry_crate() {
    let manifest = fs::read_to_string(repo_root().join("crates/pvec-components/Cargo.toml")).unwrap();
    let deps = manifest.split("[dependencies]").nth(1).unwrap().split("\n[").next().unwrap();
    let names: Vec<&str> =
        deps.lines().filter_map(|l| l.split('=').next()).map(str::trim).filter(|n| !n.is_empty()).collect();
    for n in &names {
        assert!(["pvec-capabilities", "pvec-entity", "pvec-numeric"].contains(n), "{n} is not allowed");
    }
    assert!(names.contains(&"pvec-numeric"), "NP-19 comes from pvec-numeric");
    let allowed = pvec_checks::api::API_CRATES.iter().find(|(n, _)| *n == "pvec-components").unwrap().1;
    assert!(allowed.contains(&"pvec-numeric"));
}
