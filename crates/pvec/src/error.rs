use std::fmt;
use std::sync::Arc;

/// An input or output failure of the streaming writer, kept as the `std::io::Error` it was so that
/// `PvecError::source` can hand it back. The error is shared, which is what lets `PvecError` stay `Clone`,
/// and two failures are equal when their kind and message are.
#[derive(Clone, Debug)]
pub struct IoFailure(Arc<std::io::Error>);

impl IoFailure {
    /// The error as the file system or the sink raised it.
    pub fn error(&self) -> &std::io::Error {
        &self.0
    }

    /// The kind of the error.
    pub fn kind(&self) -> std::io::ErrorKind {
        self.0.kind()
    }
}

impl PartialEq for IoFailure {
    fn eq(&self, other: &IoFailure) -> bool {
        self.0.kind() == other.0.kind() && self.0.to_string() == other.0.to_string()
    }
}

impl From<std::io::Error> for IoFailure {
    fn from(e: std::io::Error) -> IoFailure {
        IoFailure(Arc::new(e))
    }
}

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
    /// The curve model refused: a parameter outside the domain, an unbounded curve, a derivative the
    /// curve is not smooth enough for.
    Geometry {
        /// The rule of `docs/NUMERIC_POLICY.md` or `docs/ISO_PROFILE.md` that refused.
        rule: String,
    },
    /// A tessellation context that cannot be used: a budget that is not positive, a pixel ratio that
    /// is not positive, no segments allowed.
    BadContext {
        /// What was wrong with it.
        reason: String,
    },
    /// The streaming writer could not read or write a file: a spill file under the caller's root, or the
    /// sink `finish_to` writes the artifact to. Nothing of the build is left behind.
    Io {
        /// Where it failed: `"spill"` for the files under the caller's root, `"output"` for the sink.
        stage: &'static str,
        /// The failure, which `std::error::Error::source` returns.
        error: IoFailure,
    },
    /// The error budget cannot be met within `max_segments`.
    ErrorBudgetUnmet {
        /// The deviation the best tessellation within the segment limit reaches, in the unit of the
        /// budget that was missed: device pixels for the screen budget, map units for the world one.
        /// The screen budget is the one reported when both are missed.
        achieved: f64,
        /// The budget that was asked for, in the same unit.
        requested: f64,
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
            PvecError::Geometry { rule } => write!(f, "curve refused: {rule}"),
            PvecError::Io { stage, error } => write!(f, "{stage} failed: {}", error.error()),
            PvecError::BadContext { reason } => write!(f, "tessellation context refused: {reason}"),
            PvecError::ErrorBudgetUnmet { achieved, requested } => {
                write!(f, "error budget {requested} not met within the segment limit, reached {achieved}")
            }
        }
    }
}

impl std::error::Error for PvecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PvecError::Io { error, .. } => Some(error.error()),
            _ => None,
        }
    }
}

/// The facade's refusal for one of the curve model's, kept as its rule so no geometry error type is
/// reachable (conversion layer).
pub(crate) fn geometry(e: pvec_geometry::GeometryError) -> PvecError {
    PvecError::Geometry { rule: e.rule().to_string() }
}
