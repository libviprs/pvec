//! Issue #192 (P8.6, pvec#65): the geometry thresholds the first P8.4 run missed, as the committed run has to
//! report them. These were written before the tessellator changed, against the committed `geometry.csv`, and they
//! go green when the rows are re-measured on mars (x64) and mercury (arm64) after the changes. They loosen no
//! threshold: the verdict code and the limits are the ones `issue_63_geometry.rs` pins.
#![allow(missing_docs)]

use pvec_bench::geometry_bench::{Status, parse_rows, verdicts};
use std::fs;
use std::path::PathBuf;

fn csv() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/bench/geometry.csv"))
        .expect("docs/bench/geometry.csv")
}

fn status(id: &str, platform: &str) -> Status {
    let rows = parse_rows(&csv()).unwrap_or_else(|e| panic!("{e}"));
    verdicts(&rows)
        .into_iter()
        .find(|v| v.id == id && v.platform == platform)
        .unwrap_or_else(|| panic!("no verdict {id} on {platform}"))
        .status
}

#[test]
fn issue_192_tessellate_allocates_once_per_curve_on_both_platforms() {
    for platform in ["linux-x86_64", "linux-aarch64"] {
        assert_eq!(status("tessellate-one-allocation", platform), Status::Pass, "{platform}");
    }
}

#[test]
fn issue_192_halving_a_conic_grows_it_by_at_most_one_and_a_half_on_both_platforms() {
    for platform in ["linux-x86_64", "linux-aarch64"] {
        assert_eq!(status("conic-halving-1.5x", platform), Status::Pass, "{platform}");
    }
}

#[test]
fn issue_192_no_tessellation_exceeds_its_budget_on_both_platforms() {
    for platform in ["linux-x86_64", "linux-aarch64"] {
        assert_eq!(status("error-budget-never-exceeded", platform), Status::Pass, "{platform}");
    }
}

#[test]
fn issue_192_the_cubic_point_at_is_not_slower_than_the_prototype_on_x64() {
    assert_eq!(status("bspline3-point-at-150ns", "linux-x86_64"), Status::Pass);
    assert_eq!(status("bspline3-point-at-150ns", "linux-aarch64"), Status::Pass);
}
