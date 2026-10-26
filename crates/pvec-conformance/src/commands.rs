//! The subcommands, each over the bytes of a file.

use crate::Outcome;

/// `inspect`: header, manifest and directory summary.
pub fn inspect(_bytes: &[u8], _path: &str) -> Outcome {
    todo!("P7.7")
}

/// `validate`: format validity, security limits and checksums.
pub fn validate(_bytes: &[u8], _path: &str) -> Outcome {
    todo!("P7.7")
}

/// `dump-manifest`.
pub fn dump_manifest(_bytes: &[u8], _path: &str) -> Outcome {
    todo!("P7.7")
}

/// `capabilities`: what the file requires against what this build supports.
pub fn capabilities(_bytes: &[u8], _path: &str) -> Outcome {
    todo!("P7.7")
}

/// `compare-step`: the file against an independent evaluator's report on a STEP export.
pub fn compare_step(_bytes: &[u8], _path: &str, _step_path: &str, _step_len: u64, _report: &str) -> Outcome {
    todo!("P7.7")
}

/// `tessellate`: display fidelity only.
pub fn tessellate(_bytes: &[u8], _path: &str, _error_px: f64, _units_per_px: f64) -> Outcome {
    todo!("P7.7")
}

/// `bench-open`: how long opening takes.
pub fn bench_open(_bytes: &[u8], _path: &str, _iterations: u32) -> Outcome {
    todo!("P7.7")
}
