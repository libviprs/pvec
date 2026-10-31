//! The raster fixture and the layout comparison of the measurement in issue pvec#220.
//!
//! The fixture is a scanned sheet: 8192 by 6144 pixels at a 512 tile edge, five levels, bilevel PNG tiles from the
//! test-only tiler (a stored deflate stream, so a tile is 32 KB and not the few hundred bytes a real scan
//! compresses to; the index and Tier 0 numbers are therefore on the heavy side). `scan` places it over the box of
//! the drawing so every viewport of the drawing sees it, and the file is `shape::artifact_with_rasters`'s.

use crate::layout_bench::{Row, Status, Verdict};
use crate::progressive_bench::{DIRECTORY_MAX_S, TIER0_SLOW_MAX_S};
use pvec_components::{Band, PixelFormat, Placement, TileCodec};
use pvec_container::blocks::raster::{TileIn, hilbert_key};
use pvec_stream::{Aabb, RasterGrid, TileSlot, ViewportQuery, plan_raster};
use pvec_testkit::raster_file::RasterIn;
use pvec_testkit::tiler::{Shape, tiles};

/// The scan the fixture is cut from.
pub const SCAN: Shape = Shape { width: 8192, height: 6144, tile_edge: 512, channels: 1, bit_depth: 1 };

/// The tiles of [`SCAN`], cut once and shared by every raster of a file.
pub fn scan_tiles() -> Vec<TileIn> {
    tiles(SCAN, |_, _, _| true)
}

/// A raster of [`SCAN`] laid over `bounds` (`min x, min y, max x, max y`), upright (`d < 0`), the coarsest level
/// inline.
pub fn scan(bounds: [f64; 4], tiles: &[TileIn]) -> RasterIn {
    let (sx, sy) =
        ((bounds[2] - bounds[0]) / f64::from(SCAN.width), (bounds[3] - bounds[1]) / f64::from(SCAN.height));
    RasterIn {
        placement: Placement {
            pixel_to_source: [sx, 0.0, 0.0, -sy, bounds[0], bounds[3]],
            width: SCAN.width,
            height: SCAN.height,
            visible_box: [0, 0, SCAN.width, SCAN.height],
            tile_edge: SCAN.tile_edge as u16,
            levels: SCAN.levels() as u8,
            tile_codec: TileCodec::Png,
            pixel_format: PixelFormat::Gray,
            bit_depth: 1,
            opacity: 255,
            band: Band::Underlay,
            layer_index: None,
            source_digest: None,
            attribution: String::new(),
        },
        tiles: tiles.to_vec(),
        inline_levels: 1,
        chunk_bytes: 4 << 20,
    }
}

/// An A0 sheet scanned at 300 dpi, the size ADR 0018 argues from.
pub const A0: (u32, u32) = (14_043, 9_933);
/// The bytes a tile takes in the layout comparison. Only the spacing matters: the gap threshold is 64 KiB.
pub const TILE_BYTES: u32 = 12_800;

/// The ordinal of every tile of every level under `order`: `hilbert` is the writer's order, otherwise row-major.
fn ordinals(grid: &RasterGrid, hilbert: bool) -> Vec<Vec<u32>> {
    (0..grid.levels)
        .map(|level| {
            let shrink = 1u64 << (grid.levels - 1 - level);
            let cols = u64::from(grid.width).div_ceil(shrink).div_ceil(u64::from(grid.tile_edge)) as u32;
            let rows = u64::from(grid.height).div_ceil(shrink).div_ceil(u64::from(grid.tile_edge)) as u32;
            let side = u64::from(cols.max(rows)).next_power_of_two();
            let mut order: Vec<(u64, u32)> = (0..rows)
                .flat_map(|r| (0..cols).map(move |c| (c, r)))
                .map(|(c, r)| {
                    (
                        if hilbert {
                            hilbert_key(side, c, r)
                        } else {
                            u64::from(r) * u64::from(cols) + u64::from(c)
                        },
                        r * cols + c,
                    )
                })
                .collect();
            order.sort_unstable();
            let mut ordinal = vec![0u32; (cols * rows) as usize];
            for (rank, (_, cell)) in order.into_iter().enumerate() {
                ordinal[cell as usize] = rank as u32;
            }
            ordinal
        })
        .collect()
}

/// Range requests for viewports of `screen` device pixels sliding over the sheet at full resolution, one device
/// pixel per image pixel, with the tiles of a level stored in Hilbert order or in row-major order: the ranges per
/// viewport, sorted ascending (the mean, the 95th percentile and the largest are read off it). Everything else is
/// the plan's own: the 64 KiB gap and the 16 MiB ceiling.
pub fn ranges_per_viewport(sheet: (u32, u32), screen: (u32, u32), hilbert: bool) -> Vec<u32> {
    let levels =
        (0..32u32).find(|n| (512u64 << n) >= u64::from(sheet.0.max(sheet.1))).expect("a pyramid") + 1;
    let grid = RasterGrid { width: sheet.0, height: sheet.1, tile_edge: 512, levels, channels: 1 };
    let order = ordinals(&grid, hilbert);
    let cols_of = |level: u32| {
        let shrink = 1u64 << (levels - 1 - level);
        u64::from(sheet.0).div_ceil(shrink).div_ceil(512) as u32
    };
    // identity map with y up: image pixel (px, py) is source (px, height - py)
    let map = [1.0, 0.0, 0.0, -1.0, 0.0, f64::from(sheet.1)];
    let slot = |level: u32, col: u32, row: u32| TileSlot::Stored {
        block: pvec_container::BlockId(0),
        offset: 1_000_000
            + u64::from(order[level as usize][(row * cols_of(level) + col) as usize]) * u64::from(TILE_BYTES),
        len: TILE_BYTES,
    };
    let (step_x, step_y) = (256u32, 256u32);
    let mut out = Vec::new();
    let mut y = 0;
    while y + screen.1 <= sheet.1 {
        let mut x = 0;
        while x + screen.0 <= sheet.0 {
            let (x0, y0) = (f64::from(x), f64::from(sheet.1 - (y + screen.1)));
            let bounds = Aabb::new([x0, y0], [x0 + f64::from(screen.0), y0 + f64::from(screen.1)])
                .expect("a viewport");
            let plan = plan_raster(
                &grid,
                &map,
                &ViewportQuery::new(bounds, 1.0),
                u64::MAX,
                64 << 10,
                16 << 20,
                slot,
            )
            .expect("a plan");
            assert_eq!(plan.level, levels - 1, "one device pixel per image pixel is full resolution");
            out.push(plan.ranges.len() as u32);
            x += step_x;
        }
        y += step_y;
    }
    out.sort_unstable();
    out
}

/// The corpus entry with ten scanned sheets over it: the heaviest case, the million-entity drawing under ten
/// rasters in every viewport.
pub const RASTER_ENTRY: &str = "architectural-1e6+r10";
/// The ids of the two P8.5 verdicts re-run with the raster fixture (ADR 0018, stage 4).
pub const RASTER_IDS: [&str; 2] = ["directory-300ms-raster", "tier0-4s-5mbit-raster"];

/// The P8.5 verdicts `directory-300ms` and `tier0-4s-5mbit` on [`RASTER_ENTRY`], with the thresholds P8.5 pinned
/// (`DIRECTORY_MAX_S`, `TIER0_SLOW_MAX_S`): a failed verdict is a finding with an owner, not a raised limit.
pub fn raster_verdicts(rows: &[Row]) -> Vec<Verdict> {
    let mut platforms: Vec<&str> = rows.iter().map(|r| r.platform.as_str()).collect();
    platforms.sort_unstable();
    platforms.dedup();
    let mut out = Vec::new();
    for platform in platforms {
        for (id, arm, metric, max) in [
            (RASTER_IDS[0], "25mbit-60ms", "directory_s", DIRECTORY_MAX_S),
            (RASTER_IDS[1], "5mbit-200ms", "tier0_s", TIER0_SLOW_MAX_S),
        ] {
            let found = rows.iter().find(|r| {
                r.platform == platform
                    && r.table == "stage"
                    && r.entry == RASTER_ENTRY
                    && r.arm == arm
                    && r.param == "p95"
                    && r.metric == metric
            });
            let (status, detail) = match found.and_then(|r| r.value) {
                Some(v) => (
                    if v <= max { Status::Pass } else { Status::Fail },
                    format!("{RASTER_ENTRY} on {arm}: {v:.4} s (limit {max} s)"),
                ),
                None => {
                    (Status::NotMeasured, format!("not measured: {RASTER_ENTRY} has no {metric} on {arm}"))
                }
            };
            out.push(Verdict { id, platform: platform.to_string(), status, detail });
        }
    }
    out
}
