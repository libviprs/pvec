//! The bundle: the vendored tar, checked against the pin before anything in it is trusted.

use crate::error::Result;
use crate::json::Value;
use crate::pin::Pin;
use std::collections::BTreeMap;

/// A verified bundle held in memory.
#[derive(Clone, Debug)]
pub struct Bundle {
    pin: Pin,
    files: BTreeMap<String, Vec<u8>>,
    manifest: Value,
}

impl Bundle {
    /// Opens a tar against a pin. Refuses a bundle whose recorded revision differs (naming both),
    /// whose digest differs, whose manifest digest differs, or any of whose files disagrees with its
    /// `SHA256SUMS`.
    pub fn open(_tar: &[u8], _pin: &Pin) -> Result<Bundle> {
        todo!("Bundle::open")
    }
    /// The bundle vendored under `tests/corpus/`, opened against the pin next to it. Reads the two
    /// files from the repository checkout; nothing is fetched.
    pub fn vendored() -> Result<Bundle> {
        todo!("Bundle::vendored")
    }
    /// The pin it was opened against.
    pub fn pin(&self) -> &Pin {
        &self.pin
    }
    /// The parsed `manifest.json`.
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    /// A file by its path below the corpus root, for example `fixtures/circle-001/source.dwg`.
    pub fn file(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }
    /// Every path in the bundle, sorted.
    pub fn paths(&self) -> Vec<&str> {
        self.files.keys().map(String::as_str).collect()
    }
}

/// The repository's `tests/corpus` directory.
pub fn corpus_dir() -> std::path::PathBuf {
    todo!("bundle::corpus_dir")
}
