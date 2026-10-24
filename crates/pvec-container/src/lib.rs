//! The container prefix of a `.pvec` file (issue P4.1): the fixed header and the artifact manifest.
//!
//! Everything here parses from a byte slice that the caller fetched with one bounded range read, and
//! every offset and length is checked against the artifact length before it is used. The encoding is
//! the one chosen by ADR 0003 (a custom block container) and written down in `docs/CONTAINER.md`.
//!
//! [`blocks`] holds the blocks the directories point at; the first is the hot type-batched immediate
//! geometry block (issue P4.2). [`directory`] is the block directory and [`writer`] the deterministic
//! writer that puts it all together (issue P4.4).

pub mod blocks;
pub mod directory;
pub mod error;
pub mod header;
pub mod manifest;
pub mod spec;
pub mod writer;

pub use error::ContainerError;
pub use header::{ChecksumAlgorithm, Header};
pub use manifest::{Bounds, ByteRange, DirectoryPointer, LengthUnit, Manifest, Metadata};
