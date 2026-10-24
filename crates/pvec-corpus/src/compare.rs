//! Comparison of a read value against the authored one, under the rules of `docs/NUMERIC_POLICY.md`.

use crate::scalar::Scalar;

/// The two modes the corpus records per field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The DWG stores the authored f64 verbatim: the bits must be equal (NP-03: `-0.0` is `+0.0`).
    Exact,
    /// The DWG stores a derived value: `near` under PVec numeric policy v1 (NP-05).
    Policy,
}

impl Mode {
    /// Reads the manifest spelling.
    pub fn parse(_text: &str) -> Option<Mode> {
        todo!("Mode::parse")
    }
}

/// Compares `actual` with the authored `expected`. `scale` is the size of the feature (NP-04); for
/// `Exact` it is ignored.
pub fn compare(_mode: Mode, _expected: &Scalar, _actual: f64, _scale: f64) -> bool {
    todo!("compare::compare")
}
