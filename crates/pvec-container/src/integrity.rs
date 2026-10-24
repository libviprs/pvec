//! Block checksums and the artifact digest (issue P4.6). STUB for the red commit: nothing is implemented.
#![allow(dead_code)]

use core::fmt;

/// What the writer records for integrity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Integrity {
    /// Nothing: no checksum table and no digest.
    None,
    /// The per-block checksum table.
    Checksums,
    /// The checksum table and the artifact digest.
    #[default]
    Full,
}

/// The XXH3-64 of a block's stored bytes.
pub fn block_checksum(_bytes: &[u8]) -> u64 {
    0
}

/// The SHA-256 of `bytes`.
pub fn sha256(_bytes: &[u8]) -> [u8; 32] {
    [0; 32]
}

/// The artifact digest of a whole file: its SHA-256 with the 32 digest bytes at `digest_at` read as zero.
pub fn artifact_digest(_artifact: &[u8], _digest_at: usize) -> [u8; 32] {
    [0; 32]
}

/// Why integrity was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IntegrityError {
    /// The artifact digest is not the one in the manifest.
    ArtifactDigest {
        /// What the manifest holds.
        expected: [u8; 32],
        /// What the file hashes to.
        found: [u8; 32],
    },
    /// The block checksum table is not where or what it should be.
    BadTable {
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A block's stored bytes do not match their checksum.
    BlockChecksum {
        /// The block.
        block: u32,
        /// What the table holds.
        expected: u64,
        /// What the bytes hash to.
        found: u64,
    },
    /// The reader was told to require integrity and the file carries none.
    NotWritten,
}

impl IntegrityError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            IntegrityError::ArtifactDigest { .. } => "INTEGRITY-01",
            IntegrityError::BadTable { .. } => "INTEGRITY-02",
            IntegrityError::BlockChecksum { .. } => "INTEGRITY-03",
            IntegrityError::NotWritten => "INTEGRITY-04",
        }
    }
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {self:?}", self.rule())
    }
}

impl std::error::Error for IntegrityError {}

/// What verifying the artifact digest found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestStatus {
    /// The file hashes to the digest in its manifest.
    Verified,
    /// The manifest carries no digest, so there was nothing to check.
    NotWritten,
}
