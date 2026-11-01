//! Fixtures, a shuffler, a SHA-256 and a stand-in for the P4.5 reader, shared by the writer tests.
#![allow(dead_code)]

pub mod art;
pub mod fixtures;
pub mod raster;
#[path = "../../../pvec/src/spill_fs.rs"]
pub mod spill_fs;
pub mod sha256;
pub mod verify;

#[allow(unused_imports)]
pub use fixtures::*;
#[allow(unused_imports)]
pub use sha256::sha256_hex;
#[allow(unused_imports)]
pub use verify::verify;

use pvec_container::spill::{SpillError, Store};
use pvec_container::spill_writer::ByteSink;
use std::path::Path;

/// A store of spill files in a fresh directory under `root`.
pub fn store(root: &Path) -> Store {
    std::sync::Arc::new(spill_fs::FsStore::create(root).expect("the root exists"))
}

/// A sink that keeps what it is given.
#[derive(Default)]
pub struct VecSink(pub Vec<u8>);

impl ByteSink for VecSink {
    fn put(&mut self, bytes: &[u8]) -> Result<(), SpillError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

/// A sink that takes `room` bytes and then fails, as a full disk or a closed pipe does.
pub struct FullSink {
    pub room: usize,
    pub taken: usize,
}

impl ByteSink for FullSink {
    fn put(&mut self, bytes: &[u8]) -> Result<(), SpillError> {
        if self.taken + bytes.len() > self.room {
            return Err(SpillError::new(std::io::Error::new(std::io::ErrorKind::StorageFull, "full")));
        }
        self.taken += bytes.len();
        Ok(())
    }
}
