//! The `pvec` command line tool (issue P7.7): the checks of phases 2 to 6 from a shell.
//!
//! Every subcommand returns an [`Outcome`]: an exit code, a JSON report with a `report_schema_version`
//! (schema: `docs/cli-report.schema.json`) and a human-readable text. `docs/CLI.md` documents them all.

pub mod commands;
pub mod json;

use json::Json;

/// The version of the JSON report. It goes up when a field is removed or changes meaning.
pub const REPORT_SCHEMA_VERSION: u64 = 1;

/// The exit-code contract.
pub mod exit {
    /// The file conforms.
    pub const CONFORMS: u8 = 0;
    /// Non-conformance was found: a comparison disagreed.
    pub const NONCONFORMING: u8 = 1;
    /// The file was refused or is invalid.
    pub const REFUSED: u8 = 2;
    /// The file requires a capability this build does not support.
    pub const UNSUPPORTED_CAPABILITY: u8 = 3;
    /// The command line was wrong.
    pub const USAGE: u8 = 64;
    /// A file could not be read.
    pub const IO: u8 = 65;
    /// The tool itself failed.
    pub const INTERNAL: u8 = 70;
}

/// What a subcommand produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// The exit code.
    pub exit: u8,
    /// The JSON report.
    pub report: Json,
    /// The same, for a person.
    pub human: String,
    /// The named error code when the file was refused (`DIR-06`, `LIMIT-01`, `INTEGRITY-03`, ...).
    pub error_code: Option<String>,
}

/// Runs the tool on `args` (without the program name), reading the files they name.
pub fn run(_args: &[String]) -> (Outcome, bool) {
    todo!("P7.7")
}
