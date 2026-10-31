//! Issue #232: a refused tile or raster stream names its rule, `LIMIT-NN` as well as `RASTER-NN`. The thirteen
//! hostile tiles of issue 219 go through the module. ABI version 2 stays, and what answered -10 and the rule
//! number before still does. `docs/WASM_ABI.md` is the contract.
#![allow(missing_docs)]

use pvec_testkit::raster_hostile::{file, hostile};
use pvec_wasm::{AbiError, Runtime, WASM_ABI_VERSION};

/// The rule a hostile tile names, as the module gives it: its number and its family (0 `RASTER`, 1 `LIMIT`).
fn number(rule: &str) -> (u32, u32) {
    let (family, n) = rule.split_once('-').unwrap();
    (n.parse().unwrap(), u32::from(family == "LIMIT"))
}

#[test]
fn issue_232_the_abi_stays_2() {
    assert_eq!(WASM_ABI_VERSION, 2);
    assert_eq!(AbiError::RasterSkipped.code(), -10);
    assert_eq!(AbiError::Read.code(), -4);
}

#[test]
fn issue_232_a_tile_over_the_ceiling_is_skipped_and_names_limit_15() {
    let mut seen = 0;
    for t in hostile().into_iter().filter(|t| t.rule == Some("LIMIT-15")) {
        let mut rt = Runtime::new();
        let f = rt.open(file(&t)).unwrap();
        assert_eq!(rt.raster_tile(f, 0, 0, 0, 0), Err(AbiError::RasterSkipped), "{}", t.name);
        assert_eq!(rt.raster_stat(f, 1), Ok(1), "{}", t.name);
        assert_eq!(rt.raster_skipped(f, 0, 0), Ok(0), "{}: the raster id", t.name);
        assert_eq!(rt.raster_skipped(f, 0, 1), Ok(15), "{}: the number", t.name);
        assert_eq!(rt.raster_skipped(f, 0, 2), Ok(1), "{}: the family is LIMIT", t.name);
        // asking again is the same answer
        assert_eq!(rt.raster_tile(f, 0, 0, 0, 0), Err(AbiError::RasterSkipped), "{}", t.name);
        seen += 1;
    }
    assert_eq!(seen, 2, "oversized-tile and tile-at-limit-15-plus-one");
}

#[test]
fn issue_232_a_tile_refused_by_a_raster_rule_names_it_in_the_raster_family() {
    for t in hostile().into_iter().filter(|t| matches!(t.rule, Some(r) if r.starts_with("RASTER-"))) {
        // the stream of the file is refused as a whole in one of these (below); the others are refused as tiles
        let mut rt = Runtime::new();
        let f = rt.open(file(&t)).unwrap();
        if rt.raster_stat(f, 4) != Ok(0) {
            continue;
        }
        let (n, family) = number(t.rule.unwrap());
        assert_eq!(rt.raster_tile(f, 0, 0, 0, 0), Err(AbiError::RasterSkipped), "{}", t.name);
        assert_eq!(rt.raster_skipped(f, 0, 1), Ok(n), "{}", t.name);
        assert_eq!(rt.raster_skipped(f, 0, 2), Ok(family), "{}", t.name);
    }
}

#[test]
fn issue_232_a_refused_raster_stream_names_its_rule() {
    let t = hostile().into_iter().find(|t| t.name == "jpeg-under-an-rgba-placement").unwrap();
    let mut rt = Runtime::new();
    let f = rt.open(file(&t)).unwrap();
    // as before: the file has an image it cannot show and no raster
    assert_eq!(rt.raster_stat(f, 2), Ok(1));
    assert_eq!(rt.raster_stat(f, 0), Ok(0));
    // new: stat 4 is the number of the rule that refused the stream and stat 5 its family
    assert_eq!(rt.raster_stat(f, 4), Ok(4));
    assert_eq!(rt.raster_stat(f, 5), Ok(0));
    assert_eq!(rt.raster_placement(f, 0).err(), Some(AbiError::Read));
}

#[test]
fn issue_232_a_file_with_no_refusal_answers_zero_and_a_bad_selector_stays_bad() {
    let t = hostile().into_iter().find(|t| t.rule == Some("LIMIT-15")).unwrap();
    let mut rt = Runtime::new();
    let f = rt.open(file(&t)).unwrap();
    assert_eq!(rt.raster_stat(f, 4), Ok(0));
    assert_eq!(rt.raster_stat(f, 5), Ok(0));
    assert_eq!(rt.raster_stat(f, 6), Err(AbiError::BadArgument));
    assert_eq!(rt.raster_skipped(f, 0, 3), Err(AbiError::BadArgument));
}
