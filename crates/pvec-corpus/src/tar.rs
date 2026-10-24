//! A reader for the deterministic ustar bundle: regular files only, every size and name checked.

use crate::error::Result;

/// One file of the archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// The path inside the archive, with `/` separators.
    pub path: String,
    /// The file's bytes.
    pub data: &'a [u8],
}

/// Upper bound on entries, so a hostile header run cannot size an allocation.
pub const MAX_ENTRIES: usize = 4096;

/// Parses a tar held in memory.
pub fn parse(_bytes: &[u8]) -> Result<Vec<Entry<'_>>> {
    todo!("tar::parse")
}
