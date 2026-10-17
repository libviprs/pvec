//! Errors carry the rule that refused the input, so a rejection can be traced to the text.

use core::fmt;

/// A refusal under the numeric policy. Each variant names the rule that produced it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericError {
    /// NP-01: NaN or infinite.
    NotFinite,
    /// NP-01: subnormal (denormal) value.
    Subnormal,
    /// NP-02: magnitude above the coordinate bound.
    OutOfRange,
    /// NP-06: segment shorter than the degenerate length.
    DegenerateLength,
    /// NP-07: radius or semi-axis degenerate, or axis ratio above the conditioning bound.
    DegenerateRadius,
    /// NP-08: every control point coincides within tolerance.
    DegenerateHull,
    /// NP-10: direction of zero length.
    ZeroDirection,
    /// NP-12: trim domain collapsed to nothing.
    CollapsedTrim,
    /// NP-11: periodic trim spans more than one period, or is the ambiguous negative full period.
    PeriodicSpan,
    /// NP-12: trim outside the basis domain.
    TrimOutsideDomain,
    /// NP-12: sense agreement contradicts the order of the trim parameters.
    InconsistentSense,
    /// NP-13: parameter domain has no width.
    EmptyDomain,
    /// NP-14: knot vector has the wrong length, is not non-decreasing, or has a bad multiplicity.
    BadKnots,
    /// NP-14: two distinct knots are closer than the parameter tolerance.
    AmbiguousKnots,
    /// NP-15: weight non-positive or below the conditioning floor.
    BadWeight,
    /// NP-16: a two-sided derivative was requested where only one-sided limits exist.
    TwoSidedAtBreak,
    /// NP-17: composite segments do not join.
    Gap,
    /// NP-18: a count or dimension above its maximum, or below its minimum.
    Limit,
    /// NP-19: affine transform singular, or an entry out of range.
    SingularTransform,
    /// NP-20: closed flag contradicts the geometry.
    ClosedMismatch,
}

impl NumericError {
    /// The rule identifier, as written in `docs/NUMERIC_POLICY.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            NumericError::NotFinite | NumericError::Subnormal => "NP-01",
            NumericError::OutOfRange => "NP-02",
            NumericError::DegenerateLength => "NP-06",
            NumericError::DegenerateRadius => "NP-07",
            NumericError::DegenerateHull => "NP-08",
            NumericError::ZeroDirection => "NP-10",
            NumericError::PeriodicSpan => "NP-11",
            NumericError::CollapsedTrim
            | NumericError::TrimOutsideDomain
            | NumericError::InconsistentSense => "NP-12",
            NumericError::EmptyDomain => "NP-13",
            NumericError::BadKnots | NumericError::AmbiguousKnots => "NP-14",
            NumericError::BadWeight => "NP-15",
            NumericError::TwoSidedAtBreak => "NP-16",
            NumericError::Gap => "NP-17",
            NumericError::Limit => "NP-18",
            NumericError::SingularTransform => "NP-19",
            NumericError::ClosedMismatch => "NP-20",
        }
    }
}

impl fmt::Display for NumericError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} ({})", self, self.rule())
    }
}

impl std::error::Error for NumericError {}
