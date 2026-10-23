//! Blocks of a `.pvec` file: the hot type-batched immediate geometry blocks (issue P4.2) and the
//! variable-length advanced geometry arenas (issue P4.3).

pub mod advanced;
pub mod aligned;
pub mod arena;
pub mod hot;

pub use aligned::AlignedBytes;
