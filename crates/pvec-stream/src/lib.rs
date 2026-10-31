//! Viewport streaming over a PVec container (epic P5).
//!
//! What is here today is the spatial directory of issue P5.1: the index that says which cells of a
//! drawing a viewport touches, built over source-space bounds only, so one file serves any display zoom
//! and any downstream projection. The layout and what a reader must check are in `docs/STREAMING.md`
//! chapter 1. The long-entity and straddler strategy of issue P5.3 (chapter 3) keeps drawing-spanning
//! entities out of the cells, in per-level lists. This is an API crate: it opens no file or socket, and
//! it reads through `pvec_container::ContainerReader`.

mod directory;
mod generation;
mod ownership;
mod raster_plan;
mod scheduler;
mod straddler;
mod tier0_leaf;
mod tiers;
mod viewport;

pub use directory::*;
pub use generation::*;
pub use ownership::*;
pub use raster_plan::*;
pub use scheduler::*;
pub use straddler::*;
pub use tier0_leaf::*;
pub use tiers::*;
pub use viewport::*;
