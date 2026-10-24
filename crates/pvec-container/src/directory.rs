//! The block directory (issue P4.4): one fixed 64 byte entry per block, ADR 0003 and ADR 0006.

use crate::manifest::Bounds;
use crate::spec::FieldSpec;
use core::fmt;
use pvec_numeric::check_value;

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
        let mut b = [0u8; ENTRY_LEN];
        let mut put = |at: usize, bytes: &[u8]| b[at..at + bytes.len()].copy_from_slice(bytes);
        put(e::OFFSET, &self.offset.to_le_bytes());
        put(e::STORED_LENGTH, &self.stored_length.to_le_bytes());
        put(e::UNCOMPRESSED_LENGTH, &self.uncompressed_length.to_le_bytes());
        put(e::COUNT, &self.count.to_le_bytes());
        put(e::CELL, &self.cell.to_le_bytes());
        if let Some(bounds) = self.bounds {
            put(e::MIN_X, &bounds.min[0].to_le_bytes());
            put(e::MIN_Y, &bounds.min[1].to_le_bytes());
            put(e::MAX_X, &bounds.max[0].to_le_bytes());
            put(e::MAX_Y, &bounds.max[1].to_le_bytes());
        }
        put(e::KIND, &self.kind.to_le_bytes());
        put(e::SUBTYPE, &self.subtype.to_le_bytes());
        put(e::FLAGS, &[self.flags]);
        put(e::CODEC, &[self.codec]);
        b
    }
}

/// Byte offsets of the fields of an entry.
mod e {
    pub const OFFSET: usize = 0;
    pub const STORED_LENGTH: usize = 8;
    pub const UNCOMPRESSED_LENGTH: usize = 12;
    pub const COUNT: usize = 16;
    pub const CELL: usize = 20;
    pub const MIN_X: usize = 24;
    pub const MIN_Y: usize = 32;
    pub const MAX_X: usize = 40;
    pub const MAX_Y: usize = 48;
    pub const KIND: usize = 56;
    pub const SUBTYPE: usize = 58;
    pub const FLAGS: usize = 60;
    pub const CODEC: usize = 61;
    pub const RESERVED: usize = 62;
}

/// Why a directory was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DirectoryError {
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
        /// The flag byte.
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
    /// A block outside the artifact, or before the end of the header.
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
    /// Bounds that are not valid, or non-zero without the has-bounds flag.
    BadBounds {
        /// The entry.
        index: usize,
    },
}

impl DirectoryError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            DirectoryError::BadLength { .. } => "DIR-01",
            DirectoryError::NonZeroReserved { .. } => "DIR-02",
            DirectoryError::BadFlags { .. } => "DIR-03",
            DirectoryError::UnknownRequiredKind { .. } => "DIR-04",
            DirectoryError::UnknownCodec { .. } => "DIR-05",
            DirectoryError::OutOfRange { .. } => "DIR-06",
            DirectoryError::Misaligned { .. } => "DIR-07",
            DirectoryError::Overlap { .. } => "DIR-08",
            DirectoryError::LengthMismatch { .. } => "DIR-09",
            DirectoryError::BadBounds { .. } => "DIR-10",
        }
    }
}

impl fmt::Display for DirectoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            DirectoryError::BadLength { length } => {
                write!(
                    f,
                    "{rule}: a directory of {length} bytes is not a whole number of {ENTRY_LEN} byte entries"
                )
            }
            DirectoryError::NonZeroReserved { index } => {
                write!(f, "{rule}: entry {index} has a reserved field set")
            }
            DirectoryError::BadFlags { index, bits } => {
                write!(f, "{rule}: entry {index} sets undefined flag bits ({bits:#04x})")
            }
            DirectoryError::UnknownRequiredKind { index, id } => {
                write!(f, "{rule}: entry {index} is a must-understand block of unknown kind {id}")
            }
            DirectoryError::UnknownCodec { index, id } => {
                write!(f, "{rule}: entry {index} uses unknown codec {id}")
            }
            DirectoryError::OutOfRange { index, offset, length, artifact_len } => write!(
                f,
                "{rule}: entry {index} is {length} bytes at {offset}, outside an artifact of {artifact_len} bytes"
            ),
            DirectoryError::Misaligned { index, offset } => {
                write!(
                    f,
                    "{rule}: entry {index} starts at {offset}, which is not a multiple of {BLOCK_ALIGN}"
                )
            }
            DirectoryError::Overlap { index } => {
                write!(f, "{rule}: entry {index} starts before the previous block ends")
            }
            DirectoryError::LengthMismatch { index } => {
                write!(
                    f,
                    "{rule}: entry {index} stores a different number of bytes than it decodes to, with no codec"
                )
            }
            DirectoryError::BadBounds { index } => write!(f, "{rule}: entry {index} has invalid bounds"),
        }
    }
}

impl std::error::Error for DirectoryError {}

/// The refusals, rule by rule.
pub const DIRECTORY_RULES: &[(&str, &str)] = &[
    ("DIR-01", "the table is not a whole number of 64 byte entries"),
    ("DIR-02", "a reserved field is not zero"),
    ("DIR-03", "a flag bit other than must-understand and has-bounds is set"),
    (
        "DIR-04",
        "a must-understand block of a kind the reader does not know (an optional one is kept and skipped)",
    ),
    ("DIR-05", "a codec id the reader does not know"),
    (
        "DIR-06",
        "a block that starts before the end of the header, runs past the end of the artifact, or whose end overflows",
    ),
    ("DIR-07", "a block that does not start on a multiple of 8"),
    ("DIR-08", "a block that starts before the previous block ends"),
    ("DIR-09", "stored and uncompressed lengths that differ while the codec is 0"),
    ("DIR-10", "bounds that are not finite, in range and ordered, or non-zero without the has-bounds flag"),
];

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

fn f64_at(b: &[u8], at: usize) -> f64 {
    f64::from_bits(u64_at(b, at))
}

/// Parses and validates a directory table that was read from an artifact of `artifact_len` bytes
/// whose header is `header_len` bytes long.
///
/// Every offset and length is checked against the artifact before it is used and nothing is
/// allocated before the table's length is known to be a whole number of entries, so the work and the
/// memory are bounded by the table's own size. Blocks must be in file order and must not overlap.
pub fn parse_directory(
    table: &[u8],
    header_len: u64,
    artifact_len: u64,
) -> Result<Vec<BlockEntry>, DirectoryError> {
    if table.len() % ENTRY_LEN != 0 {
        return Err(DirectoryError::BadLength { length: table.len() as u64 });
    }
    let mut out = Vec::with_capacity(table.len() / ENTRY_LEN);
    let mut previous_end = header_len;
    for (index, b) in table.chunks_exact(ENTRY_LEN).enumerate() {
        if u16_at(b, e::RESERVED) != 0 {
            return Err(DirectoryError::NonZeroReserved { index });
        }
        let flags = b[e::FLAGS];
        if flags & !(FLAG_MUST_UNDERSTAND | FLAG_HAS_BOUNDS) != 0 {
            return Err(DirectoryError::BadFlags { index, bits: flags });
        }
        let kind = u16_at(b, e::KIND);
        if BlockKind::from_id(kind).is_none() && flags & FLAG_MUST_UNDERSTAND != 0 {
            return Err(DirectoryError::UnknownRequiredKind { index, id: kind });
        }
        let codec = b[e::CODEC];
        if codec != CODEC_NONE {
            return Err(DirectoryError::UnknownCodec { index, id: codec });
        }
        let (offset, stored_length) = (u64_at(b, e::OFFSET), u32_at(b, e::STORED_LENGTH));
        let end = offset.checked_add(u64::from(stored_length));
        match end {
            Some(end) if offset >= header_len && end <= artifact_len => {}
            _ => {
                return Err(DirectoryError::OutOfRange {
                    index,
                    offset,
                    length: u64::from(stored_length),
                    artifact_len,
                });
            }
        }
        if offset % BLOCK_ALIGN != 0 {
            return Err(DirectoryError::Misaligned { index, offset });
        }
        if offset < previous_end {
            return Err(DirectoryError::Overlap { index });
        }
        previous_end = offset + u64::from(stored_length);
        let uncompressed_length = u32_at(b, e::UNCOMPRESSED_LENGTH);
        if uncompressed_length != stored_length {
            return Err(DirectoryError::LengthMismatch { index });
        }
        let raw = [f64_at(b, e::MIN_X), f64_at(b, e::MIN_Y), f64_at(b, e::MAX_X), f64_at(b, e::MAX_Y)];
        let bounds = if flags & FLAG_HAS_BOUNDS != 0 {
            let mut v = [0.0f64; 4];
            for (slot, x) in v.iter_mut().zip(raw) {
                *slot = check_value(x).map_err(|_| DirectoryError::BadBounds { index })?;
            }
            if v[0] > v[2] || v[1] > v[3] {
                return Err(DirectoryError::BadBounds { index });
            }
            Some(Bounds { min: [v[0], v[1]], max: [v[2], v[3]] })
        } else if raw.iter().any(|x| x.to_bits() != 0) {
            return Err(DirectoryError::BadBounds { index });
        } else {
            None
        };
        out.push(BlockEntry {
            offset,
            stored_length,
            uncompressed_length,
            count: u32_at(b, e::COUNT),
            cell: u32_at(b, e::CELL),
            bounds,
            kind,
            subtype: u16_at(b, e::SUBTYPE),
            flags,
            codec,
        });
    }
    Ok(out)
}
