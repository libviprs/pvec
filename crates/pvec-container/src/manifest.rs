//! The artifact manifest.

use crate::{ContainerError, Header};
use pvec_capabilities::{Declaration, Declared, Version};

/// The unit of length of source space, with its ISO declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LengthUnit {
    /// Metre.
    Metre,
    /// Millimetre.
    Millimetre,
    /// Centimetre.
    Centimetre,
    /// Kilometre.
    Kilometre,
    /// Inch.
    Inch,
    /// Foot.
    Foot,
}

impl LengthUnit {
    /// Every unit, in code order.
    pub const ALL: [LengthUnit; 6] = [
        LengthUnit::Metre,
        LengthUnit::Millimetre,
        LengthUnit::Centimetre,
        LengthUnit::Kilometre,
        LengthUnit::Inch,
        LengthUnit::Foot,
    ];

    /// The code stored in the file.
    pub fn code(self) -> u16 {
        todo!("P4.1")
    }

    /// The unit for a code.
    pub fn from_code(_code: u16) -> Option<LengthUnit> {
        todo!("P4.1")
    }

    /// Metres in one unit.
    pub fn metres_per_unit(self) -> f64 {
        todo!("P4.1")
    }

    /// The ISO 10303-41 length unit this one is declared as.
    pub fn iso_declaration(self) -> &'static str {
        todo!("P4.1")
    }
}

/// Axis-aligned source bounds, in source space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// `[min_x, min_y]`.
    pub min: [f64; 2],
    /// `[max_x, max_y]`.
    pub max: [f64; 2],
}

/// An offset and length in the artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ByteRange {
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u64,
}

/// A pointer to a directory, by kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirectoryPointer {
    /// What the directory is (`KIND_BLOCK`, `KIND_COMPONENT`, `KIND_SPATIAL`, or a later kind).
    pub kind: u32,
    /// Offset.
    pub offset: u64,
    /// Length.
    pub length: u64,
}

impl DirectoryPointer {
    /// Block directory (P4.2).
    pub const KIND_BLOCK: u32 = 1;
    /// Component directory (P4.3).
    pub const KIND_COMPONENT: u32 = 2;
    /// Spatial directory (P5.1).
    pub const KIND_SPATIAL: u32 = 3;
}

/// Caller-supplied artifact metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    /// Producer id.
    pub producer_id: String,
    /// Producer version.
    pub producer_version: String,
    /// Fixture or corpus revision, where there is one.
    pub corpus_revision: Option<String>,
}

/// The parsed manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// Unit of length.
    pub unit: LengthUnit,
    /// Source bounds.
    pub bounds: Bounds,
    /// The lowest reader version the file may be opened with.
    pub min_reader: Version,
    /// Declared capabilities.
    pub capabilities: Vec<Declared>,
    /// Directory pointers.
    pub directories: Vec<DirectoryPointer>,
    /// Where P4.6's per-block checksum table lives, if written.
    pub block_checksums: Option<ByteRange>,
    /// P4.6's artifact SHA-256, if written.
    pub artifact_digest: Option<[u8; 32]>,
    /// Producer metadata.
    pub metadata: Metadata,
}

impl Manifest {
    /// The fixed part, in bytes.
    pub const FIXED_LEN: usize = 128;
    /// The largest manifest a reader will fetch.
    pub const MAX_LEN: u64 = 1 << 20;

    /// Parses and validates the manifest bytes the header points at.
    pub fn parse(_bytes: &[u8], _header: &Header, _artifact_len: u64) -> Result<Manifest, ContainerError> {
        todo!("P4.1")
    }

    /// The canonical encoding.
    pub fn encode(&self) -> Vec<u8> {
        todo!("P4.1")
    }

    /// The capability declaration P4.7 negotiates against.
    pub fn declaration(&self) -> Declaration {
        todo!("P4.1")
    }
}
