//! Typed refusals. Every error names the rule that was violated, so a rejection can be traced to
//! `docs/NUMERIC_POLICY.md` or `docs/ISO_PROFILE.md`.

use core::fmt;
use pvec_numeric::NumericError;

/// Everything a construction or an evaluation can refuse.
///
/// [`GeometryError::rule`] gives the identifier of the violated rule: `NP-nn` for the numeric policy,
/// `PROFILE-n` for a section of the semantic profile, and `GEOM-nn` for the few rules of this crate's
/// own API (`GEOM-01`: a bounding box has `min <= max`; `GEOM-02`: a bulge is not zero).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GeometryError {
    /// A numeric policy rule refused the input; the inner error names it.
    Numeric(NumericError),
    /// The parameter is outside the domain of the curve (NP-13).
    OutsideDomain,
    /// The operation needs a bounded curve and a line is not one (profile section 4).
    Unbounded,
    /// The basis of a trimmed curve is not one of the allowed kinds (profile section 8).
    BadBasis,
    /// A composite segment is not one of the allowed kinds (profile section 9).
    BadSegment,
    /// The number of weights differs from the number of control points (NP-15).
    WeightCount,
    /// A bounding box whose lower corner is above or to the right of its upper corner (GEOM-01).
    InvertedBounds,
    /// A stored direction is not unit length (NP-10).
    NotUnitDirection,
    /// A bulge of zero describes a straight segment, which is a trimmed line and not an arc (GEOM-02).
    ZeroBulge,
}

impl GeometryError {
    /// The rule identifier, as written in the policy or profile document.
    pub fn rule(&self) -> &'static str {
        match self {
            GeometryError::Numeric(e) => e.rule(),
            GeometryError::OutsideDomain => "NP-13",
            GeometryError::Unbounded => "PROFILE-4",
            GeometryError::BadBasis => "PROFILE-8",
            GeometryError::BadSegment => "PROFILE-9",
            GeometryError::WeightCount => "NP-15",
            GeometryError::InvertedBounds => "GEOM-01",
            GeometryError::NotUnitDirection => "NP-10",
            GeometryError::ZeroBulge => "GEOM-02",
        }
    }

    /// The numeric policy error behind this refusal, if it came from the numeric policy.
    pub fn as_numeric(&self) -> Option<NumericError> {
        match self {
            GeometryError::Numeric(e) => Some(*e),
            _ => None,
        }
    }
}

impl From<NumericError> for GeometryError {
    fn from(e: NumericError) -> Self {
        GeometryError::Numeric(e)
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::Numeric(e) => write!(f, "{e}"),
            GeometryError::OutsideDomain => write!(f, "OutsideDomain ({})", self.rule()),
            GeometryError::Unbounded => write!(f, "Unbounded: a line has no bounds ({})", self.rule()),
            GeometryError::BadBasis => write!(f, "BadBasis: not an allowed trim basis ({})", self.rule()),
            GeometryError::BadSegment => write!(f, "BadSegment: not an allowed segment ({})", self.rule()),
            GeometryError::WeightCount => {
                write!(f, "WeightCount: one weight per control point ({})", self.rule())
            }
            GeometryError::InvertedBounds => {
                write!(f, "InvertedBounds: min must not exceed max ({})", self.rule())
            }
            GeometryError::NotUnitDirection => {
                write!(f, "NotUnitDirection: a stored direction must be unit length ({})", self.rule())
            }
            GeometryError::ZeroBulge => {
                write!(f, "ZeroBulge: a zero bulge is a line, not an arc ({})", self.rule())
            }
        }
    }
}

impl std::error::Error for GeometryError {}

/// The result type used throughout the crate.
pub(crate) type Result<T> = core::result::Result<T, GeometryError>;
