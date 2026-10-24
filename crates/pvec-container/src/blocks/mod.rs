//! Blocks of a `.pvec` file. Today that is the hot type-batched immediate geometry block (issue P4.2).

pub mod aligned;
pub mod hot;

pub use aligned::AlignedBytes;
