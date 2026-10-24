//! The lossless number of O1.4: a decimal string with the IEEE-754 bit pattern it stands for.

use crate::error::{CorpusError, Result};
use crate::json::Value;

/// One real number as the corpus records it. `f64_hex` is authoritative; the decimal is checked to
/// parse, under round-to-nearest, to exactly those bits. A scalar may also declare a `precision`
/// (significant decimal digits, O1.4); when it does, the decimal may not carry more digits than that
/// and the declaration may not be fewer than the bits need. No declaration is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scalar {
    decimal: String,
    bits: u64,
    precision: Option<u32>,
}

impl Scalar {
    /// Reads a `{decimal, f64_hex}` object. `at` names the place in the file, for the error.
    pub fn from_json(at: &str, v: &Value) -> Result<Scalar> {
        let invalid = |why: &str| CorpusError::Invalid { what: at.to_string(), why: why.to_string() };
        let members =
            v.as_obj().ok_or_else(|| invalid("a scalar is an object with `decimal` and `f64_hex`"))?;
        if members.iter().any(|(k, _)| !matches!(k.as_str(), "decimal" | "f64_hex" | "precision")) {
            return Err(invalid("a scalar has the fields `decimal`, `f64_hex` and optionally `precision`"));
        }
        let precision = match v.get("precision") {
            None => None,
            Some(Value::Int(p)) if (1..=17).contains(p) => Some(*p as u32),
            Some(_) => return Err(invalid("`precision` is an integer from 1 to 17")),
        };
        if members.len() != 2 + usize::from(precision.is_some()) {
            return Err(invalid("a scalar has the fields `decimal`, `f64_hex` and optionally `precision`"));
        }
        let decimal = v
            .get("decimal")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("`decimal` is missing or not a string"))?;
        let hex = v
            .get("f64_hex")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("`f64_hex` is missing or not a string"))?;
        if !is_decimal(decimal) {
            return Err(invalid("`decimal` is not a plain decimal number"));
        }
        let bits =
            parse_hex(hex).ok_or_else(|| invalid("`f64_hex` is not 0x and 16 lowercase hex digits"))?;
        if !f64::from_bits(bits).is_finite() {
            return Err(invalid("a recorded value must be finite (NP-01)"));
        }
        let parsed = decimal.parse::<f64>().map_err(|_| invalid("`decimal` does not parse"))?;
        if parsed.to_bits() != bits {
            return Err(CorpusError::DecimalMismatch {
                at: at.to_string(),
                decimal: decimal.to_string(),
                hex: hex.to_string(),
            });
        }
        if let Some(declared) = precision {
            let digits = significant_digits(decimal);
            if digits > declared {
                return Err(CorpusError::PrecisionExceeded {
                    at: at.to_string(),
                    decimal: decimal.to_string(),
                    digits,
                    declared,
                });
            }
            let needed = shortest_digits(f64::from_bits(bits));
            if declared < needed {
                return Err(CorpusError::PrecisionTooLow {
                    at: at.to_string(),
                    hex: hex.to_string(),
                    needed,
                    declared,
                });
            }
        }
        Ok(Scalar { decimal: decimal.to_string(), bits, precision })
    }
    /// The declared precision in significant digits, if the record declares one.
    pub fn precision(&self) -> Option<u32> {
        self.precision
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
pub fn parse_hex(text: &str) -> Option<u64> {
    let digits = text.strip_prefix("0x")?;
    if digits.len() != 16 || !digits.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    u64::from_str_radix(digits, 16).ok()
}

/// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, the schema's pattern.
fn is_decimal(text: &str) -> bool {
    let b = text.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'-'));
    let digits = |i: &mut usize| {
        let start = *i;
        while b.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - start
    };
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == b.len()
}

/// The canonical text of a bit pattern: `0x` and 16 lowercase hex digits.
pub fn to_hex(bits: u64) -> String {
    format!("0x{bits:016x}")
}

/// What a value becomes when it is routed through a decimal of `significant` digits and parsed back.
/// This is the lossy path the loader exists to catch; tests use it to prove the check bites.
pub fn through_decimal(x: f64, significant: usize) -> f64 {
    format!("{:.*e}", significant.saturating_sub(1), x).parse().unwrap_or(f64::NAN)
}

/// Significant digits of a decimal as written: leading zeros never count, and trailing zeros count
/// only after a decimal point (`1.50` has three, `1500` has two).
fn significant_digits(decimal: &str) -> u32 {
    let mantissa = decimal.split(['e', 'E']).next().unwrap_or("").trim_start_matches('-');
    let has_point = mantissa.contains('.');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let digits = digits.trim_start_matches('0');
    let digits = if has_point { digits } else { digits.trim_end_matches('0') };
    digits.len().max(1) as u32
}

/// Digits in the shortest decimal that parses back to `x`.
fn shortest_digits(x: f64) -> u32 {
    let text = format!("{x:e}");
    text.split('e').next().unwrap_or("").chars().filter(char::is_ascii_digit).count().max(1) as u32
}
