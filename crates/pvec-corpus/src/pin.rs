//! The in-repo pin: which corpus revision this repository tests against, and the digest of its bundle.

use crate::error::Result;

/// The content of `tests/corpus/r0-open.pin.json`, a byte-for-byte copy of the file the fixture
/// repository publishes at the pinned tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The corpus revision, for example `r0-open`.
    pub corpus_revision: String,
    /// The git tag in `libviprs/pvec-gold-fixtures` the bundle was taken from.
    pub tag: String,
    /// SHA-256 of the deterministic tar.
    pub bundle_sha256: String,
    /// SHA-256 of `manifest.json` inside it.
    pub manifest_sha256: String,
    /// The numeric policy the corpus was authored against.
    pub numeric_policy_version: String,
}

impl Pin {
    /// Reads the pin file.
    pub fn parse(_bytes: &[u8]) -> Result<Pin> {
        todo!("Pin::parse")
    }
}
