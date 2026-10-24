//! The lossless number of O1.4: a decimal string with the IEEE-754 bit pattern it stands for.

use crate::error::Result;
use crate::json::Value;

/// One real number as the corpus records it. `f64_hex` is authoritative; the decimal is checked to
/// parse, under round-to-nearest, to exactly those bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scalar {
    decimal: String,
    bits: u64,
}

impl Scalar {
    /// Reads a `{decimal, f64_hex}` object. `at` names the place in the file, for the error.
    pub fn from_json(_at: &str, _v: &Value) -> Result<Scalar> {
        todo!("Scalar::from_json")
    }
    /// The IEEE-754 bit pattern.
    pub fn bits(&self) -> u64 {
        self.bits
    }
    /// The value those bits stand for.
    pub fn value(&self) -> f64 {
        f64::from_bits(self.bits)
    }
    /// The decimal text as recorded.
    pub fn decimal(&self) -> &str {
        &self.decimal
    }
}

/// Parses `0x` followed by exactly 16 lowercase hex digits.
pub fn parse_hex(_text: &str) -> Option<u64> {
    todo!("scalar::parse_hex")
}

/// The canonical text of a bit pattern: `0x` and 16 lowercase hex digits.
pub fn to_hex(_bits: u64) -> String {
    todo!("scalar::to_hex")
}

/// What a value becomes when it is routed through a decimal of `significant` digits and parsed back.
/// This is the lossy path the loader exists to catch; tests use it to prove the check bites.
pub fn through_decimal(_x: f64, _significant: usize) -> f64 {
    todo!("scalar::through_decimal")
}
