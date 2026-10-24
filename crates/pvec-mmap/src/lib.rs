//! Memory-mapped source (issue #108). Stub for the red tests: nothing is mapped yet.

use pvec_container::source::{BlockSource, Bytes, SourceError};
use std::path::Path;

/// A read-only memory map of a local file.
#[derive(Debug)]
pub struct MmapSource {
    len: u64,
}

impl MmapSource {
    /// Maps `path` read-only.
    pub fn open(_path: impl AsRef<Path>) -> Result<MmapSource, SourceError> {
        Err(SourceError::Io("not implemented".into()))
    }
}

impl BlockSource for MmapSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_range(&self, _offset: u64, _len: u32) -> Result<Bytes, SourceError> {
        Err(SourceError::Io("not implemented".into()))
    }
}
