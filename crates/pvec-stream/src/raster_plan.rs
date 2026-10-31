//! The viewport plan for one raster (issue pvec#220, `docs/STREAMING.md` chapter 9).

use crate::viewport::{PlanSource, ViewportQuery};
use pvec_components::{PixelFormat, Placement};
use pvec_container::blocks::raster::{KIND_TILES, RasterIndex, subtype};
use pvec_container::{BlockId, ReaderConfig};
use std::collections::HashMap;

/// The pixel grid a placement fixes: these four numbers vary together per raster (RASTER-01).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterGrid {
    /// Pixels of the full resolution level.
    pub width: u32,
    /// Pixels of the full resolution level.
    pub height: u32,
    /// Edge of a tile, a power of two from 256 to 1024.
    pub tile_edge: u32,
    /// Levels, the last one full resolution.
    pub levels: u32,
    /// Channels of a decoded pixel (1 to 4), which sizes a decoded tile for `LIMIT-16`.
    pub channels: u32,
}

/// What the raster index says about one tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileSlot {
    /// Length 0 in the index: the tile is not stored and draws nothing.
    NotStored,
    /// In the inline part of the index block, which the reader already has.
    Inline,
    /// In a `raster_tiles` block, at a file offset.
    Stored {
        /// The chunk block holding it.
        block: BlockId,
        /// File offset of the first byte of the tile.
        offset: u64,
        /// Stored length.
        len: u32,
    },
}

/// One tile of a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlannedTile {
    /// Column in the level.
    pub col: u32,
    /// Row in the level.
    pub row: u32,
    /// Where it is.
    pub slot: TileSlot,
}

/// The tiles of one raster a viewport wants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterPlan {
    /// The level chosen, after LIMIT-16 pushed it coarser if it had to.
    pub level: u32,
    /// The stored and inline tiles, row-major. Not-stored tiles are left out.
    pub tiles: Vec<PlannedTile>,
    /// The merged byte ranges `(offset, len)` that fetch the stored tiles that are not inline.
    pub ranges: Vec<(u64, u32)>,
    /// `tile_edge^2 * channels` summed over `tiles`.
    pub decoded_bytes: u64,
}

/// Why a raster could not be planned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterPlanError {
    /// The placement or the viewport gives no usable map (RASTER-05).
    BadMap,
    /// Not even one tile of the coarsest level fits `max_plan_decoded_bytes` (LIMIT-16).
    OverBudget,
    /// The raster is kept in the stream and not drawn (an unknown tile codec).
    Skipped,
}

/// The most cells of one level window the plan will look at. It is `LIMIT-14`'s default tile count: an
/// index cannot hold more tiles than that, so a window past it is not one a plan hands on.
const MAX_WINDOW_CELLS: u64 = 1 << 20;

/// The level whose pixels are not coarser than the device's sharper direction wants.
///
/// `scale` is device pixels per source unit. The device size of one full-resolution pixel is the
/// Jacobian `scale * [[a, b], [c, d]]` of the placement, and its larger singular value is the number of
/// device pixels the image's sharper direction spends on one pixel. A level that halves `k` times has
/// pixels `2^k` times as big, so the finest level not under-sampled is the largest `k` that keeps that
/// size at one device pixel or more: `k = floor(-log2(sigma))`, clamped to the pyramid. Overzoom
/// (`sigma >= 1`) is full resolution.
pub fn choose_level(pixel_to_source: &[f64; 6], scale: f64, levels: u32) -> u32 {
    let [a, b, c, d, _, _] = *pixel_to_source;
    let sum = a * a + b * b + c * c + d * d;
    let diff = a * a + b * b - c * c - d * d;
    let cross = a * c + b * d;
    let sigma = scale * ((sum + diff.hypot(2.0 * cross)) / 2.0).sqrt();
    let top = levels.saturating_sub(1);
    if sigma.is_nan() || sigma < f64::MIN_POSITIVE {
        return 0;
    }
    if sigma >= 1.0 {
        return top;
    }
    let halvings = (-sigma.log2()).floor();
    top - if halvings >= f64::from(top) { top } else { halvings as u32 }
}

/// The (col, row) window `[col0, col1] x [row0, row1]` of `level` that the viewport's box covers, or
/// `None` when it misses the level. `inverse` maps source space to full-resolution pixels.
fn window(grid: &RasterGrid, level: u32, inverse: &[f64; 6], query: &ViewportQuery) -> Option<[u64; 4]> {
    let shrink = (1u64 << (grid.levels - 1 - level)) as f64;
    let (lo, hi) = (query.bounds.min(), query.bounds.max());
    let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for (x, y) in [(lo[0], lo[1]), (hi[0], lo[1]), (hi[0], hi[1]), (lo[0], hi[1])] {
        let px = inverse[0] * x + inverse[1] * y + inverse[2];
        let py = inverse[3] * x + inverse[4] * y + inverse[5];
        min = [min[0].min(px), min[1].min(py)];
        max = [max[0].max(px), max[1].max(py)];
    }
    let level_w = u64::from(grid.width).div_ceil(shrink as u64);
    let level_h = u64::from(grid.height).div_ceil(shrink as u64);
    let edge = f64::from(grid.tile_edge);
    let (cols, rows) =
        (level_w.div_ceil(u64::from(grid.tile_edge)), level_h.div_ceil(u64::from(grid.tile_edge)));
    let cell = |v: f64| (v / shrink / edge).floor();
    let mut out = [0u64; 4];
    for (i, (v, count)) in
        [(min[0], cols), (min[1], rows), (max[0], cols), (max[1], rows)].into_iter().enumerate()
    {
        let c = cell(v);
        if c.is_nan() || (i < 2 && c >= count as f64) || (i >= 2 && c < 0.0) {
            return None;
        }
        // saturated in f64 first, so the cast is exact
        out[i] = c.clamp(0.0, (count - 1) as f64) as u64;
    }
    (out[0] <= out[2] && out[1] <= out[3]).then_some(out)
}

/// Plans the tiles of one raster for `query`.
///
/// The level is [`choose_level`]'s. The (col, row) window is the pixel-space box of the viewport's four
/// corners mapped through the inverse of `pixel_to_source`, so it over-fetches under rotation (about 1.8x
/// at worst, ADR 0018). `slot` answers `(level, col, row)` from the raster index. While the decoded size
/// of the tiles handed on (`tile_edge^2 * channels` each) is over `max_plan_decoded_bytes` (`LIMIT-16`),
/// the plan drops one level coarser; if even the coarsest level does not fit it is refused. Stored tiles
/// that are not inline are merged into ranges, tile with tile only: a gap of at most `gap_threshold`
/// bytes is bridged while a range stays within `max_request_bytes`. Tiles of one raster do not overlap in the
/// file (RASTER-02 refuses an index where they do), which the ranges rely on.
pub fn plan_raster(
    grid: &RasterGrid,
    pixel_to_source: &[f64; 6],
    query: &ViewportQuery,
    max_plan_decoded_bytes: u64,
    gap_threshold: u32,
    max_request_bytes: u32,
    slot: impl Fn(u32, u32, u32) -> TileSlot,
) -> Result<RasterPlan, RasterPlanError> {
    let [a, b, c, d, tx, ty] = *pixel_to_source;
    let det = a * d - b * c;
    let finite = pixel_to_source.iter().all(|v| v.is_finite()) && det.is_finite() && det != 0.0;
    if !(finite && query.scale.is_finite() && query.scale > 0.0)
        || grid.levels == 0
        || grid.levels > 32
        || grid.tile_edge == 0
        || grid.width == 0
        || grid.height == 0
    {
        return Err(RasterPlanError::BadMap);
    }
    let inverse = [d / det, -b / det, (b * ty - d * tx) / det, -c / det, a / det, (c * tx - a * ty) / det];
    if !inverse.iter().all(|v| v.is_finite()) {
        return Err(RasterPlanError::BadMap);
    }
    let unit = u64::from(grid.tile_edge).pow(2).saturating_mul(u64::from(grid.channels));
    let mut level = choose_level(pixel_to_source, query.scale, grid.levels);
    loop {
        let mut tiles = Vec::new();
        let fits = match window(grid, level, &inverse, query) {
            None => true,
            Some([c0, r0, c1, r1]) => {
                let cells = (c1 - c0 + 1).saturating_mul(r1 - r0 + 1);
                cells <= MAX_WINDOW_CELLS && {
                    for row in r0..=r1 {
                        for col in c0..=c1 {
                            let slot = slot(level, col as u32, row as u32);
                            if slot != TileSlot::NotStored {
                                tiles.push(PlannedTile { col: col as u32, row: row as u32, slot });
                            }
                        }
                    }
                    (tiles.len() as u64).saturating_mul(unit) <= max_plan_decoded_bytes
                }
            }
        };
        if fits {
            let decoded_bytes = tiles.len() as u64 * unit;
            let ranges = merge_ranges(&tiles, gap_threshold, max_request_bytes);
            return Ok(RasterPlan { level, tiles, ranges, decoded_bytes });
        }
        level = level.checked_sub(1).ok_or(RasterPlanError::OverBudget)?;
    }
}

/// The stored, non-inline tiles as byte ranges in file order, a gap of at most `gap` bytes bridged while a
/// range stays within `max` bytes. A tile longer than `max` is a range of its own.
fn merge_ranges(tiles: &[PlannedTile], gap: u32, max: u32) -> Vec<(u64, u32)> {
    let mut spans: Vec<(u64, u64)> = tiles
        .iter()
        .filter_map(|t| match t.slot {
            TileSlot::Stored { offset, len, .. } => Some((offset, offset.saturating_add(u64::from(len)))),
            _ => None,
        })
        .collect();
    spans.sort_unstable();
    spans.dedup();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (start, end) in spans {
        match out.last_mut() {
            Some((s, e))
                if start.saturating_sub(*e) <= u64::from(gap) && end.max(*e) - *s <= u64::from(max) =>
            {
                *e = (*e).max(end);
            }
            _ => out.push((start, end)),
        }
    }
    out.into_iter().map(|(s, e)| (s, u32::try_from(e - s).unwrap_or(u32::MAX))).collect()
}

/// Plans one raster of an open file from its placement and its parsed index block (`docs/STREAMING.md` 6.5).
///
/// The caller has fetched and parsed the index (`ContainerReader::raster_index`, or the `-7` protocol in the
/// browser), so this reads nothing. `reader` is only asked for its directory, to turn the index's
/// `(chunk, offset)` into a block and a file offset. A tile the index does not hold, or one whose chunk is not in
/// the directory or is shorter than the index says, is planned as not stored and draws nothing; `check_raster_blocks`
/// refuses such a file when it is opened. `LIMIT-16`, the gap and the request ceiling are the reader's own.
pub fn plan_placed<R: PlanSource>(
    reader: &R,
    raster: u32,
    placement: &Placement,
    index: &RasterIndex,
    query: &ViewportQuery,
    config: &ReaderConfig,
) -> Result<RasterPlan, RasterPlanError> {
    if placement.skip().is_some() {
        return Err(RasterPlanError::Skipped);
    }
    let chunks: HashMap<u16, (BlockId, u64, u32)> = reader
        .entries()
        .iter()
        .enumerate()
        .filter(|(_, e)| e.kind == KIND_TILES && e.cell == raster)
        .map(|(i, e)| (e.subtype, (BlockId(i as u32), e.offset, e.stored_length)))
        .collect();
    let grid = RasterGrid {
        width: placement.width,
        height: placement.height,
        tile_edge: u32::from(placement.tile_edge),
        levels: u32::from(placement.levels),
        channels: match placement.pixel_format {
            PixelFormat::Gray => 1,
            PixelFormat::GrayAlpha => 2,
            PixelFormat::Rgb => 3,
            // Rgba, and any format a later minor adds: the most a decoded pixel can take
            _ => 4,
        },
    };
    let slot = |level: u32, col: u32, row: u32| {
        let Some(tile) = index.tile(level, col, row).filter(|t| t.is_stored()) else {
            return TileSlot::NotStored;
        };
        if tile.is_inline() {
            return TileSlot::Inline;
        }
        let found = subtype(level, tile.chunk).and_then(|st| chunks.get(&st));
        match found {
            Some(&(block, offset, stored))
                if u64::from(tile.offset) + u64::from(tile.length) <= u64::from(stored) =>
            {
                TileSlot::Stored { block, offset: offset + u64::from(tile.offset), len: tile.length }
            }
            _ => TileSlot::NotStored,
        }
    };
    plan_raster(
        &grid,
        &placement.pixel_to_source,
        query,
        config.limits.max_plan_decoded_bytes,
        config.gap_threshold,
        config.max_request_bytes,
        slot,
    )
}
