//! The safe reader (P4.5) as a [`Reader`] the corpus can be run against.
//!
//! The adapter is the whole of it: bytes in, the reader's own open and full validation, a verdict out.
//! What it provides is what the reader implements, so the harness holds it to exactly those cases and
//! counts the rest as skipped.

use super::catalog::Requires;
use super::harness::{Reader, Refusal, Verdict};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use std::sync::Arc;

/// `pvec_container::reader::ContainerReader` over memory.
#[derive(Clone, Debug)]
pub struct SafeReader {
    /// How the reader is configured.
    pub config: ReaderConfig,
}

impl SafeReader {
    /// The reader with its default configuration.
    pub fn new() -> SafeReader {
        SafeReader { config: ReaderConfig::default() }
    }
}

impl Default for SafeReader {
    fn default() -> SafeReader {
        SafeReader::new()
    }
}

impl Reader for SafeReader {
    fn name(&self) -> String {
        "safe".to_string()
    }

    fn provides(&self) -> Vec<Requires> {
        vec![Requires::Parse, Requires::Limits, Requires::Integrity, Requires::Negotiation]
    }

    fn read(&self, bytes: &[u8]) -> Verdict {
        let source = MemorySource::new(Arc::<[u8]>::from(bytes));
        let read = |source| -> Result<(), pvec_container::reader::ReadError> {
            let reader = ContainerReader::open(source, self.config.clone())?;
            reader.validate()?;
            reader.verify_artifact_digest()?;
            Ok(())
        };
        match read(source) {
            Ok(()) => Verdict::Accepted,
            Err(e) => Verdict::Refused(Refusal { rule: e.rule(), detail: e.to_string() }),
        }
    }
}
