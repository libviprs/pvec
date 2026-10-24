//! The in-repo pin: which corpus revision this repository tests against, and the digest of its bundle.

use crate::error::{CorpusError, Result};
use crate::json::{self, Value};

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
    pub fn parse(bytes: &[u8]) -> Result<Pin> {
        let v = json::parse(bytes)?;
        let field = |key: &str| -> Result<String> {
            match v.get(key).and_then(Value::as_str) {
                Some(s) if !s.is_empty() => Ok(s.to_string()),
                _ => Err(CorpusError::Missing(format!("pin field `{key}`"))),
            }
        };
        let digest = |key: &str| -> Result<String> {
            let d = field(key)?;
            if d.len() == 64 && d.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                Ok(d)
            } else {
                Err(CorpusError::Invalid {
                    what: format!("pin field `{key}`"),
                    why: "not 64 lowercase hex digits".to_string(),
                })
            }
        };
        Ok(Pin {
            corpus_revision: field("corpus_revision")?,
            tag: field("tag")?,
            bundle_sha256: digest("bundle_sha256")?,
            manifest_sha256: digest("manifest_sha256")?,
            numeric_policy_version: field("numeric_policy_version")?,
        })
    }
}
