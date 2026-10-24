//! Per-block compression (issue P4.6). STUB for the red commit: nothing is implemented.
#![allow(dead_code)]

use crate::directory::BlockKind;

/// A codec id, as written in the `codec` byte of a directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Codec {
    /// Stored as it is.
    None,
    /// An LZ4 block.
    Lz4,
    /// Raw deflate.
    Deflate,
}

impl Codec {
    /// Every codec, in id order.
    pub const ALL: [Codec; 3] = [Codec::None, Codec::Lz4, Codec::Deflate];

    /// The id stored in the file.
    pub fn id(self) -> u8 {
        match self {
            Codec::None => 0,
            Codec::Lz4 => 1,
            Codec::Deflate => 2,
        }
    }

    /// The codec for an id.
    pub fn from_id(id: u8) -> Option<Codec> {
        Codec::ALL.into_iter().find(|c| c.id() == id)
    }

    /// A short name.
    pub fn name(self) -> &'static str {
        match self {
            Codec::None => "none",
            Codec::Lz4 => "lz4",
            Codec::Deflate => "deflate",
        }
    }
}

/// Why a block could not be decompressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CodecError {
    /// The decoder had more to write than the declared length.
    Overrun {
        /// The declared length.
        declared: u64,
    },
    /// The decoder finished short of the declared length.
    Short {
        /// The declared length.
        declared: u64,
        /// What it produced.
        got: u64,
    },
    /// The stream is not valid for the codec.
    Malformed,
}

impl CodecError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            CodecError::Overrun { .. } => "CODEC-01",
            CodecError::Short { .. } => "CODEC-02",
            CodecError::Malformed => "CODEC-03",
        }
    }
}

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {self:?}", self.rule())
    }
}

impl std::error::Error for CodecError {}

/// Compresses `bytes`.
pub fn compress(_codec: Codec, bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

/// Decompresses `stored` into exactly `declared` bytes.
pub fn decompress(_codec: Codec, _stored: &[u8], _declared: usize) -> Result<Vec<u8>, CodecError> {
    Err(CodecError::Malformed)
}

/// Which codec each kind of block is written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodecPolicy {
    /// Hot geometry blocks.
    pub hot: Codec,
    /// The advanced curve record block.
    pub records: Codec,
    /// The payload arenas.
    pub arena: Codec,
    /// The component stream set.
    pub components: Codec,
}

impl CodecPolicy {
    /// Nothing compressed.
    pub const NONE: CodecPolicy =
        CodecPolicy { hot: Codec::None, records: Codec::None, arena: Codec::None, components: Codec::None };

    /// One codec for every kind.
    pub fn uniform(codec: Codec) -> CodecPolicy {
        CodecPolicy { hot: codec, records: codec, arena: codec, components: codec }
    }

    /// The codec for a kind of block.
    pub fn for_kind(&self, kind: BlockKind) -> Codec {
        match kind {
            BlockKind::Hot => self.hot,
            BlockKind::Records => self.records,
            BlockKind::Arena => self.arena,
            BlockKind::Components => self.components,
        }
    }
}

impl Default for CodecPolicy {
    fn default() -> CodecPolicy {
        CodecPolicy::NONE
    }
}
