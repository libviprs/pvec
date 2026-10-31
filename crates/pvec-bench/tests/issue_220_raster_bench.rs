//! The raster measurement (issue pvec#220): the P8.5 verdicts `directory-300ms` and `tier0-4s-5mbit` re-run with
//! ten scanned sheets over the million-entity drawing, with the thresholds P8.5 pinned. The last test reads the
//! recorded numbers and is red until the fixture has run on MARS and the rows are in `docs/bench/raster.csv`.
#![allow(missing_docs)]

use pvec_bench::layout_bench::{Row, Status, parse_rows};
use pvec_bench::progressive_bench::{DIRECTORY_MAX_S, TIER0_SLOW_MAX_S};
use pvec_bench::raster_bench::{
    RASTER_ENTRY, RASTER_IDS, SCAN, ranges_per_viewport, raster_verdicts, scan, scan_tiles,
};
use std::fs;
use std::path::PathBuf;

const X: &str = "linux-x86_64";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn stage(entry: &str, arm: &str, metric: &str, value: f64) -> Row {
    Row {
        table: "stage".into(),
        entry: entry.into(),
        platform: X.into(),
        arm: arm.into(),
        param: "p95".into(),
        metric: metric.into(),
        value: Some(value),
        note: String::new(),
    }
}

fn status(rows: &[Row], id: &str) -> Option<Status> {
    raster_verdicts(rows).into_iter().find(|v| v.id == id && v.platform == X).map(|v| v.status)
}

#[test]
fn the_raster_verdicts_use_the_thresholds_p8_5_pinned() {
    let at = |d: f64, t: f64| {
        vec![
            stage(RASTER_ENTRY, "25mbit-60ms", "directory_s", d),
            stage(RASTER_ENTRY, "5mbit-200ms", "tier0_s", t),
        ]
    };
    assert_eq!(
        status(&at(DIRECTORY_MAX_S, TIER0_SLOW_MAX_S), RASTER_IDS[0]),
        Some(Status::Pass),
        "exactly at the limit passes"
    );
    assert_eq!(status(&at(DIRECTORY_MAX_S, TIER0_SLOW_MAX_S), RASTER_IDS[1]), Some(Status::Pass));
    assert_eq!(status(&at(DIRECTORY_MAX_S + 0.001, 1.0), RASTER_IDS[0]), Some(Status::Fail));
    assert_eq!(status(&at(0.1, TIER0_SLOW_MAX_S + 0.001), RASTER_IDS[1]), Some(Status::Fail));
    assert_eq!((DIRECTORY_MAX_S, TIER0_SLOW_MAX_S), (0.3, 4.0));
    let other = vec![stage("architectural-1e6", "25mbit-60ms", "directory_s", 0.1)];
    assert_eq!(
        status(&other, RASTER_IDS[0]),
        Some(Status::NotMeasured),
        "the entry without rasters is not a pass"
    );
    assert_eq!(status(&[], RASTER_IDS[0]), None, "no rows, no platform");
}

#[test]
fn the_fixture_is_a_valid_scan_over_the_drawing() {
    let tiles = scan_tiles();
    assert_eq!(tiles.len(), 192 + 48 + 12 + 4 + 1, "five levels of a 16 by 12 tile sheet");
    let r = scan([0.0, 0.0, 800.0, 600.0], &tiles);
    let [a, b, c, d, tx, ty] = r.placement.pixel_to_source;
    assert!(a > 0.0 && d < 0.0 && b == 0.0 && c == 0.0, "upright: y flips");
    // the four corners of the sheet land on the corners of the drawing's box
    assert_eq!((tx, ty), (0.0, 600.0));
    // 800 / 8192 and 600 / 6144 are exact in binary, so the corners land exactly
    assert_eq!(a * f64::from(SCAN.width), 800.0);
    assert_eq!(ty + d * f64::from(SCAN.height), 0.0);
}

#[test]
fn a_sheet_costs_the_same_ranges_whatever_the_order_only_when_it_fits_one_run() {
    // a sheet one tile across has one tile at every level: both orders give one range per viewport
    for hilbert in [true, false] {
        let v = ranges_per_viewport((512, 512), (256, 256), hilbert);
        assert!(v.iter().all(|&n| n == 1), "{v:?}");
    }
    // a wide sheet: row-major makes a viewport of several tile rows several runs, never fewer than one
    let v = ranges_per_viewport((4096, 4096), (1024, 1024), false);
    assert!(v.iter().all(|&n| n >= 1) && v.iter().any(|&n| n > 1));
}

#[test]
fn the_recorded_numbers_pass_both_verdicts_with_ten_rasters_over_the_million() {
    let csv = fs::read_to_string(repo().join("docs/bench/raster.csv"))
        .expect("docs/bench/raster.csv: the MARS run of tools/progressive-bench.sh with the raster entries is not in yet");
    let rows = parse_rows(&csv).unwrap_or_else(|e| panic!("{e}"));
    let verdicts = raster_verdicts(&rows);
    assert!(verdicts.iter().any(|v| v.platform == X), "no linux-x86_64 rows");
    for v in verdicts.iter().filter(|v| v.platform == X) {
        assert_eq!(v.status, Status::Pass, "{}: {}", v.id, v.detail);
    }
}
