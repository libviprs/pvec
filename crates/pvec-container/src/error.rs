//! Why a header or a manifest was refused.

use core::fmt;
use pvec_numeric::NumericError;

/// A refusal from `Header::parse` or `Manifest::parse`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ContainerError {
    /// The first eight bytes are not the PVec magic.
    BadMagic,
    /// The container major version is not one this reader implements.
    UnsupportedVersion {
        /// The major version in the file.
        found: u16,
        /// The only major version this reader implements.
        supported: u16,
    },
    /// The byte-order marker is not the little-endian one.
    UnsupportedByteOrder {
        /// The marker as read little-endian.
        found: u32,
    },
    /// The prefix ends before the structure does.
    Truncated {
        /// Bytes needed.
        needed: u64,
        /// Bytes supplied.
        got: u64,
    },
    /// The header length field is below the fixed header or above the 4 KiB bound.
    BadHeaderLength {
        /// The value in the file.
        found: u32,
    },
    /// An offset and length pair does not lie inside the artifact (or overlaps the header).
    PointerOutOfRange {
        /// Which pair.
        field: &'static str,
        /// The offset in the file.
        offset: u64,
        /// The length in the file.
        length: u64,
        /// The artifact length it was checked against.
        artifact_len: u64,
    },
    /// A directory or the block checksum table overlaps the manifest's own bytes.
    PointerIntoManifest {
        /// Which pair.
        field: &'static str,
        /// The offset in the file.
        offset: u64,
        /// The length in the file.
        length: u64,
    },
    /// The manifest is longer than the bounded prefix a reader is willing to fetch.
    ManifestTooLarge {
        /// The length in the file.
        length: u64,
    },
    /// A checksum algorithm id this reader does not know.
    UnknownChecksumAlgorithm {
        /// The id.
        id: u8,
    },
    /// A reserved field is not zero.
    NonZeroReserved {
        /// Which field.
        field: &'static str,
    },
    /// Flag bits this version does not define are set.
    InvalidFlags {
        /// The flags as read.
        bits: u16,
    },
    /// A length unit code that is not in the table.
    UnknownLengthUnit {
        /// The code.
        code: u16,
    },
    /// The stored metres-per-unit factor is not the unit's exact factor.
    UnitFactorMismatch,
    /// A source bound failed the numeric policy (NaN, infinite, subnormal or out of range).
    Bound {
        /// Which bound.
        field: &'static str,
        /// The rule that refused it.
        rule: NumericError,
    },
    /// `min > max` on an axis.
    InvertedBounds {
        /// `"x"` or `"y"`.
        axis: &'static str,
    },
    /// A capability id of zero.
    ZeroCapabilityId,
    /// The same capability id twice.
    DuplicateCapability {
        /// The id.
        id: u32,
    },
    /// A requirement byte that is neither required nor optional.
    InvalidRequirement {
        /// The capability.
        id: u32,
        /// The byte.
        value: u8,
    },
    /// The header's directory-pointer table range is not where the manifest puts it.
    DirectoryTableMismatch,
    /// The producer id is empty.
    MissingProducer,
    /// A string field is longer than the bound.
    StringTooLong {
        /// Which field.
        field: &'static str,
        /// The length in the file.
        length: u32,
    },
    /// A string field is not UTF-8.
    InvalidUtf8 {
        /// Which field.
        field: &'static str,
    },
    /// Bytes after the manifest's last field, in a file whose minor version adds none.
    TrailingBytes {
        /// How many.
        extra: u64,
    },
}

impl fmt::Display for ContainerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ContainerError {}
