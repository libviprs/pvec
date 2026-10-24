//! The block directory (issue P4.4): one fixed 64 byte entry per block, ADR 0003 and ADR 0006.

use crate::manifest::Bounds;
use crate::spec::FieldSpec;
use core::fmt;

/// Bytes of one directory entry.
pub const ENTRY_LEN: usize = 64;
/// Flag bit: a reader that does not know the block kind refuses the file (ADR 0008).
pub const FLAG_MUST_UNDERSTAND: u8 = 1;
/// Flag bit: the entry's bounds are meaningful.
pub const FLAG_HAS_BOUNDS: u8 = 2;
/// Codec id of a block stored as it is.
pub const CODEC_NONE: u8 = 0;
/// Blocks start on a multiple of this many bytes from the first byte of the file.
pub const BLOCK_ALIGN: u64 = 8;

const fn field(
    name: &'static str,
    offset: usize,
    size: usize,
    encoding: &'static str,
    meaning: &'static str,
) -> FieldSpec {
    FieldSpec { name, offset, size, encoding, meaning }
}

/// The directory entry, row by row.
pub const BLOCK_ENTRY_FIELDS: &[FieldSpec] = &[
    field("offset", 0, 8, "u64", "absolute offset of the stored block, a multiple of 8"),
    field("stored_length", 8, 4, "u32", "bytes of the block as stored"),
    field(
        "uncompressed_length",
        12,
        4,
        "u32",
        "bytes of the block once decoded; equal to `stored_length` while the codec is 0",
    ),
    field("count", 16, 4, "u32", "entities, records, offset-table entries or streams in the block"),
    field("cell", 20, 4, "u32", "spatial cell id (P5.1); 0 for a block that has none"),
    field(
        "min_x",
        24,
        8,
        "f64",
        "bounds of the entities in the block; zero unless the has-bounds flag is set",
    ),
    field("min_y", 32, 8, "f64", "see `min_x`"),
    field("max_x", 40, 8, "f64", "see `min_x`"),
    field("max_y", 48, 8, "f64", "see `min_x`"),
    field("kind", 56, 2, "u16", "block kind from the table in section 4.2"),
    field("subtype", 58, 2, "u16", "geometry type of a hot block, payload kind of an arena, otherwise 0"),
    field("flags", 60, 1, "u8", "bit 0 must-understand, bit 1 has-bounds; the rest zero"),
    field("codec", 61, 1, "u8", "0 stored as it is; later ids belong to P4.6"),
    field("reserved", 62, 2, "u16", "zero"),
];

/// What a block holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    /// A hot type-batched geometry block (chapter 2).
    Hot,
    /// The advanced curve record block (chapter 3).
    Records,
    /// One advanced payload arena (chapter 3).
    Arena,
    /// The component stream set (`docs/COMPONENT_STREAMS.md`).
    Components,
}

impl BlockKind {
    /// Every kind, in id order.
    pub const ALL: [BlockKind; 4] =
        [BlockKind::Hot, BlockKind::Records, BlockKind::Arena, BlockKind::Components];

    /// The id stored in the file.
    pub fn id(self) -> u16 {
        match self {
            BlockKind::Hot => 1,
            BlockKind::Records => 2,
            BlockKind::Arena => 3,
            BlockKind::Components => 4,
        }
    }

    /// The kind for an id.
    pub fn from_id(id: u16) -> Option<BlockKind> {
        BlockKind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// A short name.
    pub fn name(self) -> &'static str {
        match self {
            BlockKind::Hot => "hot",
            BlockKind::Records => "records",
            BlockKind::Arena => "arena",
            BlockKind::Components => "components",
        }
    }
}

/// One directory entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockEntry {
    /// Absolute offset of the block.
    pub offset: u64,
    /// Bytes as stored.
    pub stored_length: u32,
    /// Bytes once decoded.
    pub uncompressed_length: u32,
    /// Entities, records, entries or streams.
    pub count: u32,
    /// Spatial cell id.
    pub cell: u32,
    /// Bounds of the entities, when the block has any.
    pub bounds: Option<Bounds>,
    /// The kind id as stored (it may be one this reader does not know).
    pub kind: u16,
    /// Geometry type, payload kind or 0.
    pub subtype: u16,
    /// Flag bits.
    pub flags: u8,
    /// Codec id.
    pub codec: u8,
}

impl BlockEntry {
    /// The known kind, if the id is one.
    pub fn block_kind(&self) -> Option<BlockKind> {
        BlockKind::from_id(self.kind)
    }

    /// The 64 byte encoding.
    pub fn encode(&self) -> [u8; ENTRY_LEN] {
        [0; ENTRY_LEN]
    }
}

/// Why a directory was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectoryError {
    /// The directory is not implemented yet.
    NotImplemented,
    /// The table is not a whole number of entries.
    BadLength {
        /// Its length.
        length: u64,
    },
    /// A reserved field is not zero.
    NonZeroReserved {
        /// The entry.
        index: usize,
    },
    /// A flag bit that is not defined.
    BadFlags {
        /// The entry.
        index: usize,
        /// The bits.
        bits: u8,
    },
    /// A must-understand block of a kind this reader does not know.
    UnknownRequiredKind {
        /// The entry.
        index: usize,
        /// The kind id.
        id: u16,
    },
    /// A codec id this reader does not know.
    UnknownCodec {
        /// The entry.
        index: usize,
        /// The codec id.
        id: u8,
    },
    /// A block outside the artifact.
    OutOfRange {
        /// The entry.
        index: usize,
        /// Its offset.
        offset: u64,
        /// Its stored length.
        length: u64,
        /// The artifact length.
        artifact_len: u64,
    },
    /// A block that does not start on a multiple of 8.
    Misaligned {
        /// The entry.
        index: usize,
        /// Its offset.
        offset: u64,
    },
    /// A block that starts before the previous one ends.
    Overlap {
        /// The entry.
        index: usize,
    },
    /// Stored and uncompressed lengths that differ with no codec.
    LengthMismatch {
        /// The entry.
        index: usize,
    },
    /// Bounds that are not valid, or set without the flag.
    BadBounds {
        /// The entry.
        index: usize,
    },
}

impl DirectoryError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        "DIR-00"
    }
}

impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "directory not implemented")
    }
}

impl std::error::Error for DirectoryError {}

/// The refusals, rule by rule.
pub const DIRECTORY_RULES: &[(&str, &str)] = &[];

/// Parses and validates a directory table.
pub fn parse_directory(
    _table: &[u8],
    _header_len: u64,
    _artifact_len: u64,
) -> Result<Vec<BlockEntry>, DirectoryError> {
    Err(DirectoryError::NotImplemented)
}
