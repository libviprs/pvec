use std::fmt;

/// Everything the facade can refuse. The variants carry the rule that was broken, as text, so no
/// container or codec type is reachable through them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum PvecError {
    /// The bytes are not a PVec file this reader accepts. `rule` is the reader rule that refused.
    Read {
        /// The reader rule, as `docs/READER.md` names it.
        rule: String,
    },
    /// The writer refused an input. `rule` is the writer rule that refused.
    Write {
        /// The writer rule, as `docs/CONTAINER.md` names it.
        rule: String,
    },
    /// The manifest's length unit is not one the format knows.
    UnknownUnit {
        /// The metres-per-unit value that matched no unit.
        metres_per_unit: f64,
    },
    /// A picking input or index entry was refused. `rule` names the rule, `PICK-nn`.
    Query {
        /// The rule that refused.
        rule: String,
    },
}

impl fmt::Display for PvecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PvecError::Read { rule } => write!(f, "read refused: {rule}"),
            PvecError::Write { rule } => write!(f, "write refused: {rule}"),
            PvecError::Query { rule } => write!(f, "query refused: {rule}"),
            PvecError::UnknownUnit { metres_per_unit } => {
                write!(f, "no length unit is {metres_per_unit} metres")
            }
        }
    }
}

impl std::error::Error for PvecError {}
