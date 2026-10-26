//! The versioned report schema, `pvec/conformance-report/1`, and the reader an older consumer uses.
//!
//! Numbers are `{decimal, f64_hex}` scalars like everywhere else in the corpus format. Within a major
//! version a field may be added to the document or to a row, and a consumer ignores what it does not
//! know; a new major version is refused.

use crate::error::Result;
use crate::gates::{Gate, GateVerdict, Runner};
use pvec_capabilities::EvidenceClass;

/// The schema identifier written at the top of every report.
pub const SCHEMA: &str = "pvec/conformance-report/1";

/// One row as a consumer of version 1 reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadRow {
    /// The gate.
    pub gate: Gate,
    /// The fixture id.
    pub fixture: String,
    /// The evidence class.
    pub class: EvidenceClass,
    /// The verdict.
    pub verdict: GateVerdict,
    /// The numbers, by name.
    pub numbers: Vec<(String, f64)>,
}

/// The report as JSON.
pub fn write(_runner: &Runner) -> String {
    todo!("issue 41")
}

/// Reads a version 1 report, ignoring fields it does not know.
pub fn read(_text: &str) -> Result<Vec<ReadRow>> {
    todo!("issue 41")
}
