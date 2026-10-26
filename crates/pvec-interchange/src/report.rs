//! Per-export fidelity reports (issue P6.8). Stub: the tests come first.

use crate::FidelityRecord;

/// The report schema version.
pub const SCHEMA_VERSION: &str = "0.0";

/// Why a report was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportError {
    /// The records cover a different number of entities than the artifact has.
    EntityCount {
        /// The artifact's entity count.
        expected: u32,
        /// The distinct entities in the records.
        found: u32,
    },
    /// The target name or version is blank.
    BlankTarget,
}

/// A fidelity report.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {}

impl Report {
    /// Builds a report.
    pub fn new<'a>(
        _target: &str,
        _target_version: &str,
        _source_digest: [u8; 32],
        _artifact_entities: u32,
        _records: impl IntoIterator<Item = &'a FidelityRecord>,
    ) -> Result<Report, ReportError> {
        unimplemented!("P6.8")
    }

    /// The JSON document.
    pub fn to_json(&self) -> String {
        unimplemented!("P6.8")
    }

    /// The Markdown rendering.
    pub fn to_markdown(&self) -> String {
        unimplemented!("P6.8")
    }
}
