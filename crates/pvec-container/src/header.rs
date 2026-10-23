//! The fixed header.

use crate::error::ContainerError;

/// A checksum algorithm id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    /// XXH3-64 (ADR 0006, per block).
    Xxh3_64,
    /// SHA-256 (ADR 0006, per artifact).
    Sha256,
}

/// The parsed header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Container major version.
    pub major: u16,
    /// Container minor version.
    pub minor: u16,
    /// Length of the header in the file.
    pub header_len: u16,
    /// Per-block checksum algorithm.
    pub block_checksum: ChecksumAlgorithm,
    /// Artifact-level checksum algorithm.
    pub artifact_checksum: ChecksumAlgorithm,
    /// Where the manifest starts.
    pub manifest_offset: u64,
    /// How long it is.
    pub manifest_length: u64,
    /// Where the directory-pointer table starts.
    pub directory_table_offset: u64,
    /// How long it is.
    pub directory_table_length: u64,
}

impl Header {
    /// The fixed header of this version, in bytes.
    pub const LEN: usize = 64;
    /// The largest header a reader will fetch.
    pub const MAX_LEN: usize = 4096;
    /// The one major version this reader implements.
    pub const MAJOR: u16 = 1;

    /// Parses and validates a header from the first bytes of an artifact of `artifact_len` bytes.
    pub fn parse(_prefix: &[u8], _artifact_len: u64) -> Result<Header, ContainerError> {
        todo!("P4.1")
    }

    /// The fixed 64-byte encoding.
    pub fn encode(&self) -> [u8; Header::LEN] {
        todo!("P4.1")
    }
}
