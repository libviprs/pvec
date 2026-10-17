//! Typed refusals for the encoder and the decoder. Every one carries a rule identifier: `NP-nn` for the
//! numeric policy, `ENC-nn` for a rule of `docs/GEOMETRY_ENCODING.md`.

use core::fmt;
use pvec_entity::EntityId;
use pvec_geometry::{CurveKind, GeometryError};

/// Why a block was refused. No variant carries partial output: a refused block yields no entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The block is shorter than what its own header and counts require.
    Truncated,
    /// The first four bytes are not `PVG1`.
    BadMagic,
    /// The format version is not one this reader understands.
    UnsupportedVersion(u16),
    /// A reserved field is not zero.
    ReservedNotZero,
    /// Padding bytes are not zero, so the block is not in canonical form.
    NonZeroPadding,
    /// The length the header declares differs from the bytes present, or the structure does not use
    /// all of them.
    LengthMismatch {
        /// What the header declares.
        declared: usize,
        /// What was found.
        actual: usize,
    },
    /// More entities than the NP-18 limit.
    TooMany,
    /// The per-kind counts do not add up to the entity count in the header.
    CountMismatch {
        /// The header's entity count.
        header: u32,
        /// The sum of the directory counts.
        sum: u64,
    },
    /// The kind directory is not strictly ascending, or has an empty entry.
    BadKindDirectory,
    /// A kind id this reader does not know.
    UnknownKind(u16),
    /// An entity id at or above the NP-18 limit.
    BadId(u32),
    /// The same entity id twice in one block.
    DuplicateId(EntityId),
    /// A flag bit that the format does not define.
    BadFlags,
    /// An offset table that does not start at zero, goes backwards, or points outside its arena.
    BadOffsets,
    /// A composite segment tag that the format does not define.
    BadTag(u32),
    /// A variable-length record whose content disagrees with its own header or its length.
    BadRecord,
    /// The bytes were well formed but describe a curve the geometry constructors refuse.
    Geometry {
        /// The entity.
        id: EntityId,
        /// The rule that refused it.
        error: GeometryError,
    },
}

impl DecodeError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            DecodeError::Truncated => "ENC-01",
            DecodeError::BadMagic => "ENC-02",
            DecodeError::UnsupportedVersion(_) => "ENC-03",
            DecodeError::ReservedNotZero => "ENC-04",
            DecodeError::LengthMismatch { .. } => "ENC-05",
            DecodeError::TooMany => "NP-18",
            DecodeError::CountMismatch { .. } => "ENC-06",
            DecodeError::BadKindDirectory => "ENC-07",
            DecodeError::UnknownKind(_) => "ENC-08",
            DecodeError::BadId(_) => "ENC-09",
            DecodeError::DuplicateId(_) => "ENC-10",
            DecodeError::BadFlags => "ENC-11",
            DecodeError::BadOffsets => "ENC-12",
            DecodeError::BadTag(_) => "ENC-13",
            DecodeError::BadRecord => "ENC-14",
            DecodeError::NonZeroPadding => "ENC-15",
            DecodeError::Geometry { error, .. } => error.rule(),
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            DecodeError::Truncated => write!(f, "the block is shorter than its structure requires ({rule})"),
            DecodeError::BadMagic => write!(f, "not a PVec geometry block: bad magic ({rule})"),
            DecodeError::UnsupportedVersion(v) => {
                write!(f, "unsupported geometry block version {v} ({rule})")
            }
            DecodeError::ReservedNotZero => write!(f, "a reserved field is not zero ({rule})"),
            DecodeError::NonZeroPadding => write!(f, "padding is not zero ({rule})"),
            DecodeError::LengthMismatch { declared, actual } => {
                write!(f, "declared length {declared} but {actual} found ({rule})")
            }
            DecodeError::TooMany => write!(f, "more entities than the file limit ({rule})"),
            DecodeError::CountMismatch { header, sum } => {
                write!(f, "header says {header} entities but the directory adds up to {sum} ({rule})")
            }
            DecodeError::BadKindDirectory => write!(f, "malformed kind directory ({rule})"),
            DecodeError::UnknownKind(k) => write!(f, "unknown section kind {k} ({rule})"),
            DecodeError::BadId(i) => write!(f, "entity id {i} is above the limit ({rule})"),
            DecodeError::DuplicateId(i) => write!(f, "entity id {i} appears twice ({rule})"),
            DecodeError::BadFlags => write!(f, "undefined flag bits set ({rule})"),
            DecodeError::BadOffsets => write!(f, "malformed offset table ({rule})"),
            DecodeError::BadTag(t) => write!(f, "unknown segment tag {t} ({rule})"),
            DecodeError::BadRecord => write!(f, "malformed record ({rule})"),
            DecodeError::Geometry { id, error } => write!(f, "entity {id}: {error}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Why a set of entities could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// Two entities share an id.
    DuplicateId(EntityId),
    /// More entities than the NP-18 limit.
    TooMany,
    /// The curve kind has no codec yet. It is refused; it is never encoded as something else.
    Unsupported(CurveKind),
    /// The encoded block would not fit the 32-bit offsets of the format.
    TooLarge,
}

impl EncodeError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            EncodeError::DuplicateId(_) => "ENC-10",
            EncodeError::TooMany => "NP-18",
            EncodeError::Unsupported(_) => "ENC-16",
            EncodeError::TooLarge => "ENC-17",
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            EncodeError::DuplicateId(i) => write!(f, "entity id {i} appears twice ({rule})"),
            EncodeError::TooMany => write!(f, "more entities than the file limit ({rule})"),
            EncodeError::Unsupported(k) => write!(f, "no codec for {k:?} (refused, not degraded) ({rule})"),
            EncodeError::TooLarge => write!(f, "the block does not fit 32-bit offsets ({rule})"),
        }
    }
}

impl std::error::Error for EncodeError {}
