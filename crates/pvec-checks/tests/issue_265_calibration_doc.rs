//! Issue #265 (#263 slice 1): the calibration specification wording. ADR 0019 is accepted and records the choices
//! the panel left open, `docs/CALIBRATION.md` carries the layout and the rule table, and the documents around it say
//! the same thing about capability 22 and kind 11.

use pvec_checks::repo_root;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|_| panic!("{rel} exists"))
}

fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn adr_0019_is_accepted_has_the_sections_and_records_the_implementer_choices() {
    let t = read("docs/adr/0019-calibration.md");
    assert!(t.starts_with("# ADR 0019"));
    let status = t.lines().find(|l| l.starts_with("Status: ")).expect("a status line");
    assert!(status.starts_with("Status: Accepted, 2026-"), "{status}");
    for h in ["Context", "Options", "Decision", "Consequences", "Reversal threshold"] {
        assert!(t.contains(&format!("\n## {h}")), "ADR 0019 has no '{h}' section");
    }
    let d = flat(&t);
    for needle in [
        "### The choices the panel left to the implementer",
        "**Field order.**",
        "**The residual check in `validate`.**",
        "detmath::hypot",
        "**The WASM export names.**",
        "`calibration_stat(file, which)`",
        "**LIMIT-17 is a `Limits` field**",
        "`max_calibration_bytes`",
        "ADR 0018 open question 2",
        "Effect::Uncalibrated",
        "1.2.0",
    ] {
        assert!(d.contains(needle), "ADR 0019 does not say {needle:?}");
    }
}

#[test]
fn calibration_md_carries_the_layout_and_every_rule() {
    let d = flat(&read("docs/CALIBRATION.md"));
    for needle in [
        "little endian",
        "`PVCL`",
        "[a b c d tx ty]",
        "`metres_per_source_unit`",
        "1 `declared_unit`, 2 `plan_scale`, 3 `measured`, 4 `georeference`, 5 `manual`",
        "1 projected, 2 geographic, 3 engineering or local",
        "1 declared by the source, 2 fitted by the writer from the points, 3 manual",
        "0 none, 1 WKT2, 2 PROJJSON",
        "`[A-Za-z0-9:_.+-]`",
        "49,894 bytes",
        "`effective_scale()`",
        "always the last block",
        "<!-- calibration-errors:begin -->",
    ] {
        assert!(d.contains(needle), "docs/CALIBRATION.md does not say {needle:?}");
    }
    for n in 1..=12 {
        assert!(d.contains(&format!("| CAL-{n:02} |")), "docs/CALIBRATION.md has no CAL-{n:02} row");
    }
    assert!(d.contains("| LIMIT-17 |") && !d.contains("| CAL-13 |"));
}

#[test]
fn container_md_reserves_kind_11_for_capability_22() {
    let d = flat(&read("docs/CONTAINER.md"));
    assert!(d.contains("| 11 | calibration |"), "kind 11 is not in the kind table");
    assert!(d.contains("capability-owned by 22"), "kind 11 does not name capability 22");
    assert!(d.contains("kind 11 belongs to capability 22"), "NEGOTIATE-06 does not name kind 11");
    assert!(d.contains("whose class is never required"), "NEGOTIATE-04 is not worded by class");
}

#[test]
fn adr_0010_and_0018_record_the_class_and_the_answer() {
    let t = read("docs/adr/0010-capability-registry.md");
    let from = t.find("\n## Amendment 2026-11-07").expect("ADR 0010 has the 2026-11-07 amendment");
    let a = t[from + 1..].split("\n## ").next().unwrap();
    for needle in ["`Calibration`", "optional", "no entity", "block kinds", "ADR 0019", "1.2.0"] {
        assert!(a.contains(needle), "ADR 0010's amendment does not say {needle:?}");
    }
    let t = flat(&read("docs/adr/0018-raster-in-pvec.md"));
    assert!(t.contains("Answered 2026-11-07 (ADR 0019)"), "ADR 0018 open question 2 is not answered");
}

#[test]
fn readme_migration_and_freeze_carry_the_new_wording() {
    assert!(read("docs/README.md").contains("(CALIBRATION.md)"));
    let m = flat(&read("docs/MIGRATION.md"));
    assert!(
        m.contains("1.2.0 is the second") && m.contains("capability 22"),
        "MIGRATION.md has no 1.2.0 line"
    );
    let f = read("docs/FREEZE-v1.md");
    assert!(f.contains("capability format version 1.2.0"));
    let row = f.lines().find(|l| l.starts_with("| extension rules")).expect("the extension rules row");
    assert!(row.contains("kind 11 belongs to capability 22") && row.contains("ADR 0019"), "row: {row}");
}
