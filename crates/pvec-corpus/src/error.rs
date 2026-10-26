//! One error type for the whole loader. Every variant names what it refused, so a failed run says
//! which file, fixture, capability or revision was at fault.

use std::fmt;

/// Why the corpus, or part of it, was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CorpusError {
    /// A JSON syntax problem, or JSON this loader refuses on purpose (a bare float, a duplicate key).
    Json {
        /// Byte offset of the problem.
        at: usize,
        /// What was wrong.
        why: String,
    },
    /// A malformed or unsupported tar.
    Tar(String),
    /// A file or field the loader needs is not there.
    Missing(String),
    /// A field is there but wrong.
    Invalid {
        /// The field or file.
        what: String,
        /// What was wrong with it.
        why: String,
    },
    /// Bytes do not hash to the pinned digest.
    HashMismatch {
        /// What was hashed (the bundle, the manifest, a file path).
        what: String,
        /// The digest the pin or `SHA256SUMS` records.
        pinned: String,
        /// The digest of the bytes in hand.
        actual: String,
    },
    /// The bundle records a different corpus revision from the one this repository pins.
    RevisionMismatch {
        /// The revision the pin names.
        pinned: String,
        /// The revision the bundle's manifest records.
        recorded: String,
    },
    /// A scalar's decimal text does not parse to exactly its recorded IEEE-754 bits.
    DecimalMismatch {
        /// Where in the record.
        at: String,
        /// The decimal text.
        decimal: String,
        /// The recorded bit pattern.
        hex: String,
    },
    /// A scalar's decimal text carries more significant digits than the precision it declares.
    PrecisionExceeded {
        /// Where in the record.
        at: String,
        /// The decimal text.
        decimal: String,
        /// Significant digits as written.
        digits: u32,
        /// The declared precision.
        declared: u32,
    },
    /// A scalar declares fewer significant digits than its bit pattern needs to round-trip.
    PrecisionTooLow {
        /// Where in the record.
        at: String,
        /// The recorded bit pattern.
        hex: String,
        /// Digits the shortest round-trip decimal has.
        needed: u32,
        /// The declared precision.
        declared: u32,
    },
    /// A fixture carries no evidence class, or one outside the four of EPIC O.
    Unclassified {
        /// The fixture id.
        fixture: String,
        /// The class text it carried, if any.
        found: Option<String>,
    },
    /// A fixture claims a class its recorded evidence cannot support.
    ClassWithoutEvidence {
        /// The fixture id.
        fixture: String,
        /// The class it claims.
        class: String,
        /// Why the evidence is not enough.
        why: String,
    },
    /// A fixture exercises a capability the registry does not know.
    UnknownCapability {
        /// The fixture id.
        fixture: String,
        /// The capability name as the corpus spells it.
        capability: String,
    },
    /// A fixture exercises a capability that `docs/ISO_PROFILE.md` has no section for.
    CapabilityNotInProfile {
        /// The fixture id.
        fixture: String,
        /// The capability name as the corpus spells it.
        capability: String,
        /// The registry name it resolves to.
        registry_name: String,
    },
    /// A comparison names a mode other than `EXACT` or `POLICY`.
    UnknownMode {
        /// The fixture id.
        fixture: String,
        /// The mode text.
        mode: String,
    },
    /// A fixture cannot carry dual-oracle evidence because one STEP output is missing.
    DualOracleUnavailable {
        /// The fixture id.
        fixture: String,
        /// The class it belongs under instead, `REAL_WORLD_DIFFERENTIAL` or `QUARANTINED`.
        route: String,
    },
    /// The two STEP paths did not go through the same evaluator.
    EvaluatorMismatch {
        /// The fixture id.
        fixture: String,
        /// The evaluator identity on the primary path.
        primary: String,
        /// The evaluator identity on the secondary path.
        secondary: String,
    },
}

impl fmt::Display for CorpusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CorpusError::Json { at, why } => write!(f, "json refused at byte {at}: {why}"),
            CorpusError::Tar(why) => write!(f, "tar refused: {why}"),
            CorpusError::Missing(what) => write!(f, "missing {what}"),
            CorpusError::Invalid { what, why } => write!(f, "{what} is invalid: {why}"),
            CorpusError::HashMismatch { what, pinned, actual } => {
                write!(f, "{what} does not match its pin: pinned sha256 {pinned}, actual sha256 {actual}")
            }
            CorpusError::RevisionMismatch { pinned, recorded } => write!(
                f,
                "corpus revision mismatch: this repository pins `{pinned}`, the bundle records `{recorded}`"
            ),
            CorpusError::DecimalMismatch { at, decimal, hex } => write!(
                f,
                "{at}: decimal `{decimal}` does not parse to the recorded bits {hex}; \
                 a shortened decimal has replaced the f64"
            ),
            CorpusError::PrecisionExceeded { at, decimal, digits, declared } => write!(
                f,
                "{at}: decimal `{decimal}` carries {digits} significant digits but declares a precision of {declared}"
            ),
            CorpusError::PrecisionTooLow { at, hex, needed, declared } => write!(
                f,
                "{at}: {hex} needs {needed} significant digits to round-trip but declares a precision of {declared}"
            ),
            CorpusError::Unclassified { fixture, found } => match found {
                Some(c) => write!(
                    f,
                    "fixture `{fixture}` is unclassified: `{c}` is not one of the four evidence classes"
                ),
                None => write!(f, "fixture `{fixture}` is unclassified: it declares no evidence class"),
            },
            CorpusError::ClassWithoutEvidence { fixture, class, why } => {
                write!(f, "fixture `{fixture}` claims {class} without the evidence for it: {why}")
            }
            CorpusError::UnknownCapability { fixture, capability } => write!(
                f,
                "fixture `{fixture}` exercises capability `{capability}`, which is not in the capability registry"
            ),
            CorpusError::CapabilityNotInProfile { fixture, capability, registry_name } => write!(
                f,
                "fixture `{fixture}` exercises capability `{capability}` (registry `{registry_name}`), \
                 which docs/ISO_PROFILE.md has no section for"
            ),
            CorpusError::UnknownMode { fixture, mode } => {
                write!(f, "fixture `{fixture}` has a comparison in unknown mode `{mode}`")
            }
            CorpusError::DualOracleUnavailable { fixture, route } => write!(
                f,
                "fixture `{fixture}` has one STEP output, so it carries no dual-oracle evidence; \
                 it belongs under {route}"
            ),
            CorpusError::EvaluatorMismatch { fixture, primary, secondary } => write!(
                f,
                "fixture `{fixture}`: the STEP paths went through different evaluators \
                 (primary {primary}, secondary {secondary})"
            ),
        }
    }
}

impl std::error::Error for CorpusError {}

/// Shorthand used throughout the crate.
pub type Result<T> = std::result::Result<T, CorpusError>;
