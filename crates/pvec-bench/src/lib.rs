//! The benchmark corpus of phase 8 (issue P8.1). The entry points are declared here and not
//! implemented yet.

use std::collections::BTreeMap;

/// The seven corpus classes, by the name the lock file and `CORPUS.md` use.
pub const CLASSES: [&str; 7] = [
    "synthetic-known-truth",
    "pathological",
    "architectural",
    "mep",
    "industrial",
    "huge-coordinate",
    "spline-heavy",
];

/// The artifact bytes of one corpus entry, rebuilt from the generator or the vendored bundle.
pub fn build(name: &str) -> Result<Vec<u8>, String> {
    Err(format!("{name}: not implemented"))
}

/// Entity counts by capability name, read back from the artifact.
pub fn counts(_artifact: &[u8]) -> Result<BTreeMap<String, u64>, String> {
    Err("not implemented".into())
}

/// The artifact's declared source bounds `[min_x, min_y, max_x, max_y]`.
pub fn bounds(_artifact: &[u8]) -> Result<[f64; 4], String> {
    Err("not implemented".into())
}

/// The 200 recorded viewports of one entry.
pub fn viewports(_name: &str, _bounds: [f64; 4]) -> Vec<[f64; 4]> {
    Vec::new()
}
