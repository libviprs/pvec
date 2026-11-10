//! Issue A2.4 (pvec#304): `tools/pin-check.sh` passes when every consumer pins one pvec rev and fails, naming the
//! revs, when they split. The fixtures under `tests/pin-check/` copy the shape of the libviprs, planviewer and server
//! manifests.
#![allow(missing_docs)]

use pvec_checks::repo_root;
use std::process::{Command, Output};

fn run(dirs: &[&str]) -> Output {
    let root = repo_root();
    let mut cmd = Command::new("bash");
    cmd.arg(root.join("tools/pin-check.sh"));
    for d in dirs {
        cmd.arg(root.join("tests/pin-check").join(d));
    }
    cmd.output().expect("bash runs")
}

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

#[test]
fn equal_revs_pass() {
    let o = run(&["equal/libviprs", "equal/planviewer", "equal/server"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("OK: one pvec rev"));
}

#[test]
fn a_split_rev_fails_and_names_both() {
    let o = run(&["split/libviprs", "split/planviewer", "split/server"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    let t = text(&o);
    assert!(t.contains("1a9d972ef844d4258a641d3891473ce2f1c39ce4") && t.contains("b9f441618153d256777121a6a1518b15e7f42099"), "{t}");
    assert!(t.contains("tests/pdf-compare/gen/Cargo.toml"), "{t}");
}

#[test]
fn a_dependency_without_a_rev_fails() {
    let o = run(&["no-rev/app"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("has no rev"));
}

#[test]
fn a_pin_json_missing_crates_fails() {
    let o = run(&["json-short/app"]);
    assert_eq!(o.status.code(), Some(1), "{}", text(&o));
    assert!(text(&o).contains("pvec-entity") && text(&o).contains("pvec-geometry"));
}

#[test]
fn a_checkout_without_pvec_is_skipped_but_none_at_all_is_an_error() {
    let o = run(&["equal/libviprs", "no-pvec/app"]);
    assert!(o.status.success(), "{}", text(&o));
    assert!(text(&o).contains("no pvec dependency, skipped"));
    assert_eq!(run(&["no-pvec/app"]).status.code(), Some(2));
}
