//! One error type for the harness. Each variant names the fixture and the rule it broke.

use core::fmt;

/// Why the harness refused something.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConformanceError {
    /// The manifest gave a fixture no class, or one outside EPIC O's four.
    Unclassified {
        /// The fixture id.
        fixture: String,
        /// The text it carried.
        found: String,
    },
    /// A fixture id the registry does not hold.
    UnknownFixture(String),
    /// Something tried to give a fixture a class other than the manifest's.
    ClassIsReadOnly {
        /// The fixture id.
        fixture: String,
        /// The class EPIC O's manifest gives it.
        manifest: String,
        /// The class that was claimed.
        claimed: String,
    },
    /// An assertion the fixture's class cannot support.
    AssertionNotSupported {
        /// The fixture id.
        fixture: String,
        /// Its class.
        class: String,
        /// What was asserted.
        assertion: String,
    },
    /// A verdict was recorded for an assertion nobody registered.
    NotRegistered {
        /// The fixture id.
        fixture: String,
        /// What was asserted.
        assertion: String,
    },
    /// A geometric comparison was refused or could not be set up.
    Geometric(String),
    /// A report line breaks the report format.
    Report {
        /// The offending line.
        line: String,
        /// What is wrong with it.
        why: String,
    },
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConformanceError::Unclassified { fixture, found } => write!(
                f,
                "fixture `{fixture}` is unclassified: `{found}` is not one of EPIC O's four evidence classes"
            ),
            ConformanceError::UnknownFixture(id) => write!(f, "no fixture `{id}` is registered"),
            ConformanceError::ClassIsReadOnly { fixture, manifest, claimed } => write!(
                f,
                "fixture `{fixture}`: the class is read from EPIC O's manifest ({manifest}) and cannot be {claimed} here"
            ),
            ConformanceError::AssertionNotSupported { fixture, class, assertion } => {
                write!(f, "fixture `{fixture}` is {class}, which cannot support a claim of {assertion}")
            }
            ConformanceError::NotRegistered { fixture, assertion } => {
                write!(f, "fixture `{fixture}`: {assertion} was never registered")
            }
            ConformanceError::Geometric(why) => write!(f, "geometric comparison refused: {why}"),
            ConformanceError::Report { line, why } => write!(f, "report line `{line}`: {why}"),
        }
    }
}

impl std::error::Error for ConformanceError {}

/// Shorthand used throughout the crate.
pub type Result<T> = std::result::Result<T, ConformanceError>;
