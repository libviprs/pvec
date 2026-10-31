//! The viewport plan for one raster (issue pvec#220, `docs/STREAMING.md` chapter 9).

use crate::viewport::ViewportQuery;
use pvec_container::BlockId;

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
}

/// The level whose pixels are not coarser than the device's sharper direction wants.
pub fn choose_level(_pixel_to_source: &[f64; 6], _scale: f64, _levels: u32) -> u32 {
    unimplemented!("pvec#220")
}

/// Plans the tiles of one raster for `query`.
pub fn plan_raster(
    _grid: &RasterGrid,
    _pixel_to_source: &[f64; 6],
    _channels: u32,
    _query: &ViewportQuery,
    _max_plan_decoded_bytes: u64,
    _gap_threshold: u32,
    _max_request_bytes: u32,
    _slot: impl Fn(u32, u32, u32) -> TileSlot,
) -> Result<RasterPlan, RasterPlanError> {
    unimplemented!("pvec#220")
}
