//! The raster viewport plan (issue pvec#220, ADR 0018 "Spatial index and range fetch").
//!
//! The matrices are written out by hand, so each expected level is a number worked out on paper, not a
//! copy of the code under test. The RASTER.md test vector replaces `SHEARED` once that chapter lands.

use pvec_container::BlockId;
use pvec_stream::{
    Aabb, PlannedTile, RasterGrid, RasterPlan, RasterPlanError, TileSlot, ViewportQuery, choose_level,
    plan_raster,
};

const GAP: u32 = 64 << 10;
const MAX_REQUEST: u32 = 16 << 20;
/// The tile bytes begin here; everything below it stands for geometry blocks.
const TILES_AT: u64 = 1_000_000;
const TILE_LEN: u32 = 20_000;
const CHANNELS: u32 = 4;
const LIMIT_16: u64 = 256 << 20;

/// Rows `[a b tx]`, `[c d ty]`: x = 2 px + 2 py, y = -py (an upright image has d < 0). The Jacobian
/// `[[2, 2], [0, -1]]` has `J^T J = [[4, 4], [4, 5]]`, trace 9 and determinant 4, so the squared singular
/// values are (9 +- sqrt(65)) / 2 and the larger singular value is sqrt(8.5311) = 2.9208.
const SHEARED: [f64; 6] = [2.0, 2.0, 0.0, 0.0, -1.0, 0.0];

fn grid_20k() -> RasterGrid {
    RasterGrid { width: 20_000, height: 20_000, tile_edge: 512, levels: 7, channels: CHANNELS }
}

fn cols(grid: &RasterGrid, level: u32) -> u32 {
    let shrink = 1u64 << (grid.levels - 1 - level);
    (u64::from(grid.width).div_ceil(shrink)).div_ceil(u64::from(grid.tile_edge)) as u32
}

/// Every tile stored, row-major in one chunk after the geometry.
fn dense(grid: RasterGrid) -> impl Fn(u32, u32, u32) -> TileSlot {
    move |level, col, row| {
        let ordinal = u64::from(row) * u64::from(cols(&grid, level)) + u64::from(col);
        TileSlot::Stored {
            block: BlockId(40 + level),
            offset: TILES_AT + ordinal * u64::from(TILE_LEN),
            len: TILE_LEN,
        }
    }
}

fn view(min: [f64; 2], max: [f64; 2], scale: f64) -> ViewportQuery {
    ViewportQuery::new(Aabb::new(min, max).unwrap(), scale)
}

fn plan(
    grid: &RasterGrid,
    map: &[f64; 6],
    q: &ViewportQuery,
    max_decoded: u64,
    slot: impl Fn(u32, u32, u32) -> TileSlot,
) -> Result<RasterPlan, RasterPlanError> {
    plan_raster(grid, map, q, max_decoded, GAP, MAX_REQUEST, slot)
}

#[test]
fn level_follows_the_larger_singular_value() {
    // device pixels per full-resolution pixel along the sharper direction: 2.9208 * scale.
    // scale 1/16: 0.18255, -log2 = 2.45, floor 2, so two halvings below full resolution: level 5 - 2 = 3.
    assert_eq!(choose_level(&SHEARED, 1.0 / 16.0, 6), 3);
    // scale 1/64: 0.04564, -log2 = 4.45, floor 4: level 1.
    assert_eq!(choose_level(&SHEARED, 1.0 / 64.0, 6), 1);
}

#[test]
fn level_is_unchanged_by_rotating_the_placement() {
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    // rotate the sheared placement by 30 degrees in source space: R * J keeps the singular values
    let [a, b, _, cc, d, _] = SHEARED;
    let rotated = [c * a - s * cc, c * b - s * d, 0.0, s * a + c * cc, s * b + c * d, 0.0];
    for scale in [1.0 / 4.0, 1.0 / 16.0, 1.0 / 64.0, 1.0 / 300.0] {
        assert_eq!(choose_level(&rotated, scale, 9), choose_level(&SHEARED, scale, 9), "scale {scale}");
    }
}

#[test]
fn overzoom_clamps_at_full_resolution() {
    assert_eq!(choose_level(&SHEARED, 50.0, 6), 5);
    assert_eq!(
        choose_level(&SHEARED, 1.0 / 2.9208, 6),
        5,
        "exactly one device pixel per pixel is full resolution"
    );
    let grid = grid_20k();
    let p = plan(
        &grid,
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        &view([100.0, 100.0], [140.0, 140.0], 40.0),
        LIMIT_16,
        dense(grid),
    )
    .unwrap();
    assert_eq!(p.level, grid.levels - 1);
    assert_eq!(p.tiles.len(), 1);
}

#[test]
fn a_far_zoom_out_clamps_at_the_one_tile_level() {
    assert_eq!(choose_level(&SHEARED, 1e-9, 6), 0);
}

#[test]
fn a_rotation_overfetches_by_at_most_one_point_eight() {
    let grid = grid_20k();
    // pixel space is source space turned by 30 degrees, one source unit per pixel
    let (s, c) = (30f64.to_radians().sin(), 30f64.to_radians().cos());
    let map = [c, -s, 2_000.0, s, c, 1_000.0];
    let q = view([6_000.0, 4_000.0], [10_000.0, 8_000.0], 1.0);
    let p = plan(&grid, &map, &q, u64::MAX, dense(grid)).unwrap();
    assert_eq!(p.level, 6);

    // the exact set: tiles whose square meets the viewport rectangle mapped back into pixel space
    let det = map[0] * map[4] - map[1] * map[3];
    let inv = |x: f64, y: f64| {
        let (dx, dy) = (x - map[2], y - map[5]);
        [(map[4] * dx - map[1] * dy) / det, (-map[3] * dx + map[0] * dy) / det]
    };
    let poly = [inv(6_000.0, 4_000.0), inv(10_000.0, 4_000.0), inv(10_000.0, 8_000.0), inv(6_000.0, 8_000.0)];
    let mut exact = Vec::new();
    for row in 0..cols(&grid, 6) {
        for col in 0..cols(&grid, 6) {
            let sq = [
                [f64::from(col) * 512.0, f64::from(row) * 512.0],
                [f64::from(col + 1) * 512.0, f64::from(row) * 512.0],
                [f64::from(col + 1) * 512.0, f64::from(row + 1) * 512.0],
                [f64::from(col) * 512.0, f64::from(row + 1) * 512.0],
            ];
            if convex_overlap(&poly, &sq) {
                exact.push((col, row));
            }
        }
    }
    let got: Vec<(u32, u32)> = p.tiles.iter().map(|t| (t.col, t.row)).collect();
    assert!(exact.iter().all(|t| got.contains(t)), "the plan must hold every tile the viewport touches");
    let ratio = got.len() as f64 / exact.len() as f64;
    println!("rotation over-fetch: {} planned / {} exact = {ratio:.3}", got.len(), exact.len());
    assert!(ratio <= 1.8, "over-fetch {ratio}");
}

/// Separating axis test for two convex quads.
fn convex_overlap(a: &[[f64; 2]; 4], b: &[[f64; 2]; 4]) -> bool {
    for poly in [a, b] {
        for i in 0..4 {
            let (p, q) = (poly[i], poly[(i + 1) % 4]);
            let axis = [q[1] - p[1], p[0] - q[0]];
            let project = |pts: &[[f64; 2]; 4]| {
                let v: Vec<f64> = pts.iter().map(|x| x[0] * axis[0] + x[1] * axis[1]).collect();
                (v.iter().cloned().fold(f64::MAX, f64::min), v.iter().cloned().fold(f64::MIN, f64::max))
            };
            let (a0, a1) = project(a);
            let (b0, b1) = project(b);
            if a1 < b0 || b1 < a0 {
                return false;
            }
        }
    }
    true
}

#[test]
fn ranges_stay_inside_the_tile_bytes_and_merge_close_tiles() {
    let grid = grid_20k();
    let q = view([1_000.0, 1_000.0], [2_800.0, 2_800.0], 1.0);
    let p = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, dense(grid)).unwrap();
    assert!(!p.tiles.is_empty() && !p.ranges.is_empty());
    let lo = p.tiles.iter().filter_map(stored).map(|(o, _)| o).min().unwrap();
    let hi = p.tiles.iter().filter_map(stored).map(|(o, l)| o + u64::from(l)).max().unwrap();
    for &(offset, len) in &p.ranges {
        assert!(offset >= TILES_AT, "a range reached into the geometry blocks");
        assert!(offset >= lo && offset + u64::from(len) <= hi, "a range left the hull of its tiles");
    }
    // every stored tile is covered by exactly one range
    for t in p.tiles.iter().filter_map(stored) {
        let n =
            p.ranges.iter().filter(|&&(o, l)| o <= t.0 && t.0 + u64::from(t.1) <= o + u64::from(l)).count();
        assert_eq!(n, 1);
    }
    // a row of tiles is TILE_LEN apart in the file, so the 64 KiB gap joins neighbours: fewer ranges than tiles
    assert!(p.ranges.len() < p.tiles.len());
}

#[test]
fn tiles_far_apart_are_separate_ranges_and_the_request_ceiling_splits_runs() {
    let grid = grid_20k();
    // 1 MiB between consecutive tiles: no gap bridges them
    let sparse = |_l: u32, col: u32, row: u32| TileSlot::Stored {
        block: BlockId(41),
        offset: TILES_AT + (u64::from(row) * 100 + u64::from(col)) * (1 << 20),
        len: TILE_LEN,
    };
    let q = view([1_000.0, 1_000.0], [2_000.0, 1_500.0], 1.0);
    let p = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, sparse).unwrap();
    assert_eq!(p.ranges.len(), p.tiles.len());
    // a ceiling of two tiles splits an adjacent run
    let q = view([0.0, 0.0], [2_000.0, 500.0], 1.0);
    let split =
        plan_raster(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, GAP, 2 * TILE_LEN, dense(grid))
            .unwrap();
    assert!(split.ranges.iter().all(|&(_, l)| l <= 2 * TILE_LEN));
}

fn stored(t: &PlannedTile) -> Option<(u64, u32)> {
    match t.slot {
        TileSlot::Stored { offset, len, .. } => Some((offset, len)),
        _ => None,
    }
}

/// One MiB between consecutive tiles, so no gap bridges two of them and each range is one tile.
fn spaced(level: u32, col: u32, row: u32) -> TileSlot {
    let ordinal = u64::from(row) * 100 + u64::from(col);
    TileSlot::Stored { block: BlockId(40 + level), offset: TILES_AT + ordinal * (1 << 20), len: TILE_LEN }
}

#[test]
fn a_tile_that_is_not_stored_yields_no_range_and_no_tile() {
    let grid = grid_20k();
    let map = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let q = view([0.0, 0.0], [1_000.0, 500.0], 1.0);
    let hole = |l: u32, col: u32, row: u32| {
        if (col, row) == (1, 0) { TileSlot::NotStored } else { spaced(l, col, row) }
    };
    let whole = plan(&grid, &map, &q, LIMIT_16, spaced).unwrap();
    let holed = plan(&grid, &map, &q, LIMIT_16, hole).unwrap();
    assert_eq!(holed.tiles.len() + 1, whole.tiles.len());
    assert!(holed.tiles.iter().all(|t| (t.col, t.row) != (1, 0)));
    assert_eq!(holed.ranges.len() + 1, whole.ranges.len());
    let missing = TILES_AT + (1 << 20);
    assert!(holed.ranges.iter().all(|&(o, _)| o != missing));
    let nothing = plan(&grid, &map, &q, LIMIT_16, |_, _, _| TileSlot::NotStored).unwrap();
    assert!(nothing.tiles.is_empty() && nothing.ranges.is_empty() && nothing.decoded_bytes == 0);
}

#[test]
fn inline_tiles_are_reported_present_and_never_requested() {
    let grid = grid_20k();
    let q = view([0.0, 0.0], [1_000.0, 500.0], 1.0);
    let inline_first = |l: u32, col: u32, row: u32| {
        if (col, row) == (0, 0) { TileSlot::Inline } else { spaced(l, col, row) }
    };
    let p = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, inline_first).unwrap();
    assert!(p.tiles.iter().any(|t| (t.col, t.row) == (0, 0) && t.slot == TileSlot::Inline));
    assert!(p.ranges.iter().all(|&(o, _)| o != TILES_AT), "the inline tile was requested");
    assert_eq!(p.ranges.len() + 1, p.tiles.len());
}

#[test]
fn a_tiny_scale_is_planned_coarser_and_stays_under_limit_16() {
    // a 20,000 px image placed so one source unit is 16 pixels, seen at scale 1/16 of a 40,000-unit view:
    // the sharper direction asks for level 6 (full), whose window holds every tile.
    let grid = grid_20k();
    let map = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let q = view([0.0, 0.0], [20_000.0, 20_000.0], 1.0);
    let tile_bytes = u64::from(grid.tile_edge).pow(2) * u64::from(CHANNELS);
    let fine = plan(&grid, &map, &q, u64::MAX, dense(grid)).unwrap();
    assert_eq!(fine.level, 6);
    assert_eq!(fine.tiles.len(), 40 * 40);
    assert_eq!(fine.decoded_bytes, 1600 * tile_bytes);

    let capped = plan(&grid, &map, &q, LIMIT_16, dense(grid)).unwrap();
    assert!(capped.level < fine.level, "the limit must push the plan to a coarser level");
    assert!(capped.decoded_bytes <= LIMIT_16);
    assert_eq!(capped.decoded_bytes, capped.tiles.len() as u64 * tile_bytes);
    // 256 tiles of 512 px is what 256 MiB admits; level 5 is 20x20 = 400 tiles, level 4 is 10x10 = 100
    assert_eq!(capped.level, 4);

    // the same plan under a raised limit picks the finer one again
    let raised = plan(&grid, &map, &q, 400 * tile_bytes * 4 + 1, dense(grid)).unwrap();
    assert!(raised.level > capped.level);
}

#[test]
fn nothing_fits_when_one_tile_is_over_the_limit() {
    let grid = grid_20k();
    let q = view([0.0, 0.0], [100.0, 100.0], 1.0);
    let r = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, 1024, dense(grid));
    assert_eq!(r, Err(RasterPlanError::OverBudget));
}

#[test]
fn a_singular_or_non_finite_map_is_refused() {
    let grid = grid_20k();
    let q = view([0.0, 0.0], [100.0, 100.0], 1.0);
    for map in [[1.0, 2.0, 0.0, 2.0, 4.0, 0.0], [f64::NAN, 0.0, 0.0, 0.0, 1.0, 0.0], [0.0; 6]] {
        assert_eq!(plan(&grid, &map, &q, LIMIT_16, dense(grid)), Err(RasterPlanError::BadMap));
    }
}

#[test]
fn a_viewport_off_the_raster_plans_nothing() {
    let grid = grid_20k();
    let q = view([-5_000.0, -5_000.0], [-1_000.0, -1_000.0], 1.0);
    let p = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, dense(grid)).unwrap();
    assert!(p.tiles.is_empty() && p.ranges.is_empty());
}

#[test]
fn a_huge_viewport_saturates_instead_of_overflowing() {
    let grid = grid_20k();
    let q = view([-1e9, -1e9], [1e9, 1e9], 1e-9);
    let p = plan(&grid, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0], &q, LIMIT_16, dense(grid)).unwrap();
    assert!(p.decoded_bytes <= LIMIT_16);
    assert!(p.tiles.iter().all(|t| t.col < cols(&grid, p.level) && t.row < cols(&grid, p.level)));
}
