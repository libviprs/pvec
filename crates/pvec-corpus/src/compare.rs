//! Comparison of a read value against the authored one, under the rules of `docs/NUMERIC_POLICY.md`.

use crate::scalar::Scalar;
use pvec_numeric::Policy;

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
    pub fn parse(text: &str) -> Option<Mode> {
        match text {
            "EXACT" => Some(Mode::Exact),
            "POLICY" => Some(Mode::Policy),
            _ => None,
        }
    }
}

/// Compares `actual` with the authored `expected`. `scale` is the size of the feature (NP-04); for
/// `Exact` it is ignored.
pub fn compare(mode: Mode, expected: &Scalar, actual: f64, scale: f64) -> bool {
    if !actual.is_finite() {
        return false;
    }
    // NP-03: negative zero is positive zero. Adding +0.0 turns -0.0 into +0.0 and changes nothing else.
    let (want, got) = (expected.value() + 0.0, actual + 0.0);
    match mode {
        Mode::Exact => want.to_bits() == got.to_bits(),
        Mode::Policy => Policy::V1.near(want, got, scale),
    }
}
