//! Per-block compression (issue P4.6).
//!
//! Blocks compress independently or range streaming stops working: a reader that wants one block
//! decompresses one block. The codec id is in the block's directory entry, so different kinds of block may
//! use different codecs and an uncompressed block (codec 0) stays legal. The codecs are the candidates of
//! ADR 0006, both pure Rust: an LZ4 block and raw deflate. `docs/spikes/04-6-compression.md` has the
//! measurement and the decision on which the writer uses by default.
//!
//! Decompression is where a hostile file is cheapest to make expensive, so it is bounded twice. The
//! reader refuses a descriptor whose declared decoded length is above the expansion ratio before the
//! length is reserved (`limits.rs`), and [`decompress`] is given that declared length and stops there:
//! a stream that would write more is [`CodecError::Overrun`], never an allocation of what it wanted, and a
//! stream that ends short is [`CodecError::Short`], never a short block.

use crate::directory::BlockKind;
use lz4_flex::block::DecompressError;
use miniz_oxide::inflate::TINFLStatus;

/// The deflate level: the one miniz_oxide documents as the default balance. It is fixed, not a
/// parameter, so the same input is the same bytes (a changed level is a changed artifact).
const DEFLATE_LEVEL: u8 = 6;

/// A codec id, as written in the `codec` byte of a directory entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Codec {
    /// Stored as it is.
    None,
    /// An LZ4 block, with no size prefix: the decoded length is in the directory entry.
    Lz4,
    /// Raw deflate, with no zlib or gzip wrapper.
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

/// The refusals, rule by rule.
pub const CODEC_RULES: &[(&str, &str)] = &[
    (
        "CODEC-01",
        "a stream that has more to write than the length its descriptor declares (it is stopped at that length)",
    ),
    ("CODEC-02", "a stream that ends short of the length its descriptor declares"),
    ("CODEC-03", "a stream that is not valid for its codec"),
];

impl core::fmt::Display for CodecError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            CodecError::Overrun { declared } => {
                write!(
                    f,
                    "{rule}: the stream has more to write than the {declared} bytes its descriptor declares"
                )
            }
            CodecError::Short { declared, got } => {
                write!(
                    f,
                    "{rule}: the stream ends after {got} of the {declared} bytes its descriptor declares"
                )
            }
            CodecError::Malformed => write!(f, "{rule}: the stream is not valid for its codec"),
        }
    }
}

impl std::error::Error for CodecError {}

/// Compresses `bytes`. The same bytes give the same output for a given version of the codec crate
/// (`Cargo.lock` pins it) and a given pointer width: both codecs are integer code with no dependence on the
/// host's byte order or floating point, but `lz4_flex` picks a different match finder on 32-bit targets, so
/// a wasm32 writer's LZ4 output is not a 64-bit writer's.
pub fn compress(codec: Codec, bytes: &[u8]) -> Vec<u8> {
    match codec {
        Codec::None => bytes.to_vec(),
        Codec::Lz4 => lz4_flex::block::compress(bytes),
        Codec::Deflate => miniz_oxide::deflate::compress_to_vec(bytes, DEFLATE_LEVEL),
    }
}

/// Decompresses `stored` into exactly `declared` bytes.
///
/// `declared` is what the directory entry says and what the caller has already held to its limits and
/// reserved; this never allocates more than that for the output. A stream that has more to write is
/// [`CodecError::Overrun`], one that ends short is [`CodecError::Short`], and anything the codec cannot
/// parse is [`CodecError::Malformed`]. It does not panic on any input.
pub fn decompress(codec: Codec, stored: &[u8], declared: usize) -> Result<Vec<u8>, CodecError> {
    let declared64 = declared as u64;
    match codec {
        Codec::None => match stored.len().cmp(&declared) {
            core::cmp::Ordering::Equal => Ok(stored.to_vec()),
            core::cmp::Ordering::Greater => Err(CodecError::Overrun { declared: declared64 }),
            core::cmp::Ordering::Less => {
                Err(CodecError::Short { declared: declared64, got: stored.len() as u64 })
            }
        },
        Codec::Lz4 => {
            let mut out = vec![0u8; declared];
            match lz4_flex::block::decompress_into(stored, &mut out) {
                Ok(n) if n == declared => Ok(out),
                Ok(n) => Err(CodecError::Short { declared: declared64, got: n as u64 }),
                Err(DecompressError::OutputTooSmall { .. }) => {
                    Err(CodecError::Overrun { declared: declared64 })
                }
                Err(_) => Err(CodecError::Malformed),
            }
        }
        Codec::Deflate => match miniz_oxide::inflate::decompress_to_vec_with_limit(stored, declared) {
            Ok(out) if out.len() == declared => Ok(out),
            Ok(out) => Err(CodecError::Short { declared: declared64, got: out.len() as u64 }),
            Err(e) if e.status == TINFLStatus::HasMoreOutput => {
                Err(CodecError::Overrun { declared: declared64 })
            }
            Err(_) => Err(CodecError::Malformed),
        },
    }
}

/// Compresses `bytes` with `codec` and returns what a writer stores: the compressed stream and `codec` when
/// that is smaller than the input, and the input as it is, with [`Codec::None`], when it is not. An
/// uncompressed block is always legal, so a block that does not shrink is not made larger by trying.
pub fn pack(codec: Codec, bytes: Vec<u8>) -> (Codec, Vec<u8>) {
    if codec == Codec::None {
        return (Codec::None, bytes);
    }
    let packed = compress(codec, &bytes);
    if packed.len() < bytes.len() { (codec, packed) } else { (Codec::None, bytes) }
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

    /// What `docs/spikes/04-6-compression.md` recommends for anything served over HTTP: LZ4 for every
    /// kind. It is not the default, because the measurement was made on synthetic fixtures; the default
    /// stays [`CodecPolicy::NONE`] until real drawings say otherwise (P8.3).
    pub const RECOMMENDED: CodecPolicy =
        CodecPolicy { hot: Codec::Lz4, records: Codec::Lz4, arena: Codec::Lz4, components: Codec::Lz4 };

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
            // the spatial directory is the one block every query opens, so it is read in a single range
            // and costs no decode work: it is always stored as it is
            BlockKind::SpatialDirectory => Codec::None,
        }
    }
}

impl Default for CodecPolicy {
    fn default() -> CodecPolicy {
        CodecPolicy::NONE
    }
}
