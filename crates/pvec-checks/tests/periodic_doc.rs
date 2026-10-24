//! Issue 93: the decision about periodic B-splines is written down where a reader looks for it.

use pvec_checks::*;
use std::fs;

fn read(rel: &str) -> String {
    fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|_| panic!("{rel} exists"))
}

fn amendment() -> String {
    let t = read("docs/adr/0011-geometry-block-encoding.md");
    let at = t.find("## Amendment for the periodic B-spline (issue 93)").expect("ADR 0011 has the amendment");
    t[at..].to_string()
}

#[test]
fn the_amendment_weighs_the_three_options_and_names_a_reversal_threshold() {
    let a = amendment();
    for needle in ["exact unwrap", "wrapped form", "keep refusing", "Reversal threshold", "NG-B3", "ARN-16"] {
        assert!(a.contains(needle), "amendment does not mention {needle}");
    }
    // the evidence is numbers, and says which machine produced them
    assert!(a.contains("native x64") && a.contains("arm64"), "results are labelled by architecture");
}

#[test]
fn the_profile_says_how_a_periodic_spline_reaches_pvec() {
    let p = read("docs/ISO_PROFILE.md");
    assert!(p.contains("BSpline::from_periodic"), "section 11 names the entry point");
    assert!(!p.contains("There is no wrapped periodic representation in v1"), "the stale sentence is gone");
}

#[test]
fn the_container_still_refuses_the_periodic_bit() {
    let c = read("docs/CONTAINER.md");
    assert!(c.contains("ARN-16") && c.contains("the periodic flag is set"));
    assert!(c.contains("unwrapped before it is stored"), "CONTAINER.md says why the bit stays refused");
}

#[test]
fn the_geometry_api_listing_carries_the_entry_point() {
    let api = read("crates/pvec-geometry/api.txt");
    assert!(api.contains("BSpline::from_periodic("), "api.txt lists from_periodic");
    assert!(api.contains("Unwrapped"), "api.txt lists the result type");
}
