//! The container prefix of a `.pvec` file (issue P4.1): the fixed header and the artifact manifest.
//!
//! Everything here parses from a byte slice that the caller fetched with one bounded range read, and
//! every offset and length is checked against the artifact length before it is used. The encoding is
//! the one chosen by ADR 0003 (a custom block container) and written down in `docs/CONTAINER.md`.

pub mod error;
pub mod header;
pub mod manifest;
pub mod spec;

pub use error::ContainerError;
pub use header::{ChecksumAlgorithm, Header};
pub use manifest::{Bounds, ByteRange, DirectoryPointer, LengthUnit, Manifest, Metadata};
