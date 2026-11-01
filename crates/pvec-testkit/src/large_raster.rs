//! A large raster `.pvec` from a seed (issue #251), for the consumers that put a heap and a GPU budget
//! under load. Not written yet.

use crate::tiler::Shape;

/// One `.pvec` with a gray raster of `shape` (the tiler's pyramid) and `lines` seeded line segments and
/// `curves` seeded curves drawn over it.
pub fn large_raster(_seed: u64, _shape: Shape, _lines: u32, _curves: u32) -> Vec<u8> {
    unimplemented!("issue 251")
}
