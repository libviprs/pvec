//! The raster index block (kind 9) and the tile blocks it points into (kind 10), ADR 0018 and issue #218.
//!
//! PVec stores the encoded bytes of a raster pyramid and never looks at a pixel. One index block per raster
//! says, for every tile of every level, which chunk it is in, where, how long it is and its XXH3-64; the tiles
//! themselves are packed into tile blocks, one per (raster, level, chunk), and read by range. The coarsest levels
//! can be stored inline at the end of the index block, so the first paint of a raster is one range over the
//! adjacent index blocks.
//!
//! This module is the format and nothing else: it parses and builds bytes. It does not know a directory, a
//! source or a limit table. The writer (`writer.rs`) and the reader (`reader.rs`) put it in a file, and the
//! ceilings that are not about the bytes themselves (`LIMIT-13`, `LIMIT-15`) are theirs. What it does enforce is
//! `RASTER-02`, before every allocation it protects: a count in the header is checked against the length of the
//! block before anything is sized by it, so the work and the memory are bounded by the block's own size.
//!
//! # Layout
//!
//! All integers are little endian. The block is, in order and with no padding:
//!
//! | part | bytes | content |
//! |---|---|---|
//! | header | 16 | `levels`, `tiles`, `stored`, `inline_len`, each a `u32` |
//! | level table | 16 per level | `cols`, `rows`, `chunks`, `first`, each a `u32`, level 0 (the coarsest) first |
//! | tile table | 28 per tile | `xxh3 u64`, `col`, `row`, `offset`, `length`, `chunk`, each a `u32` |
//! | inline part | `inline_len` | the bytes of the tiles whose chunk is [`INLINE`], in table order |
//!
//! `tiles` is the number of entries of the tile table, which is `cols * rows` summed over the levels, and
//! `stored` is how many of them have a non-zero length. The tile table holds the levels one after the other, and
//! inside a level the tiles are in Hilbert order ([`hilbert_key`]), strictly ascending, so a viewport is a few runs
//! of the table and the order is checked by a parser rather than trusted. `first` is the ordinal of the level's
//! first entry.
//!
//! A tile with length 0 is not stored and draws nothing (a sparse pyramid); its other fields are 0. A stored tile
//! is either inline (`chunk` is [`INLINE`], `offset` is a position in the index block itself) or in chunk `chunk`
//! of its level (`offset` is a position in that kind 10 block). The bytes are packed with no gap, in table order,
//! so a chunk is exactly its tiles, and a level is either all inline (`chunks` 0) or all in chunks.

use crate::integrity::block_checksum;
use core::fmt;

/// The block kind of a raster index.
pub const KIND_INDEX: u16 = 9;
/// The block kind of a raster tile chunk.
pub const KIND_TILES: u16 = 10;
/// The registry id of the capability that owns both kinds.
pub const CAPABILITY: u16 = 21;
/// The `chunk` of a tile whose bytes are in the index block.
pub const INLINE: u32 = u32::MAX;
/// Chunks per level that `subtype = level * 1024 + chunk` can name.
pub const CHUNKS_PER_LEVEL: u32 = 1024;
/// Levels that fit the same `u16` subtype.
pub const MAX_LEVELS: u32 = 64;
/// Columns or rows of a level. The Hilbert key of a level is below the square of this.
pub const MAX_SIDE: u32 = 1 << 20;
/// Bytes of the header.
pub const HEADER_LEN: usize = 16;
/// Bytes of one level of the level table.
pub const LEVEL_LEN: usize = 16;
/// Bytes of one entry of the tile table.
pub const TILE_LEN: usize = 28;

/// The `subtype` of the directory entry of chunk `chunk` of level `level`: `level * 1024 + chunk`, which is
/// unique and ascends by level under LAYOUT-04. `None` when either does not fit.
pub fn subtype(level: u32, chunk: u32) -> Option<u16> {
    None
}

/// The position of tile `(col, row)` along the Hilbert curve over the square of side `side`, which is a power
/// of two at least as large as both. Distinct tiles of one level have distinct keys.
pub fn hilbert_key(side: u64, col: u32, row: u32) -> u64 {
    let _ = (side, col, row);
    0
}

/// The side of the Hilbert square of a level of `cols` by `rows` tiles.
fn side_of(cols: u32, rows: u32) -> u64 {
    u64::from(cols.max(rows)).next_power_of_two()
}

/// Why an index block or a tile set was refused. Every variant is `RASTER-02` except
/// [`IndexError::TooManyTiles`], which is `LIMIT-14`, and the ones that only the writer can meet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexError {
    /// The block is shorter than its header, or its length is not what the header and the tables add up to.
    Length {
        /// What the header and the tables add up to, or the header's size.
        expected: u64,
        /// The bytes there are.
        found: u64,
    },
    /// No level, or more levels than the subtype can name.
    Levels {
        /// The count in the block.
        found: u32,
    },
    /// More tile entries than the ceiling.
    TooManyTiles {
        /// The count in the block.
        declared: u64,
        /// The ceiling.
        ceiling: u64,
    },
    /// A level of no columns or rows, or of more than [`MAX_SIDE`] of either, or one whose `first` is not the
    /// number of entries before it, or with more chunks than a subtype can name.
    Level {
        /// The level.
        level: u32,
    },
    /// The levels hold a number of tiles other than the header says, or `stored` is not the number of non-zero
    /// lengths.
    Count,
    /// A level whose size is not the one the placement fixes.
    Grid {
        /// The level.
        level: u32,
    },
    /// A tile outside its level's grid, or tiles not in strictly ascending Hilbert order.
    Order {
        /// The level.
        level: u32,
        /// The ordinal in the tile table.
        ordinal: u32,
    },
    /// A tile whose chunk, offset or length breaks the packing rules of the layout.
    Packing {
        /// The ordinal in the tile table.
        ordinal: u32,
    },
    /// A tile chunk the tile blocks do not match: the block is missing, too short, or longer than its tiles.
    Chunk {
        /// The level.
        level: u32,
        /// The chunk.
        chunk: u32,
    },
    /// The same `(level, col, row)` twice, or a tile outside the grid, handed to the builder.
    Tile {
        /// The level.
        level: u32,
        /// The column.
        col: u32,
        /// The row.
        row: u32,
    },
    /// A tile longer than a chunk may be, or a level that needs more chunks than a subtype can name.
    TooLarge {
        /// The level.
        level: u32,
    },
}

impl IndexError {
    /// The rule of `docs/RASTER.md` and `docs/SECURITY_LIMITS.md` that refuses it.
    pub fn rule(&self) -> &'static str {
        match self {
            IndexError::TooManyTiles { .. } => "LIMIT-14",
            _ => "RASTER-02",
        }
    }
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            IndexError::Length { expected, found } => {
                write!(f, "{rule}: the index block is {found} bytes, its header says {expected}")
            }
            IndexError::Levels { found } => write!(f, "{rule}: {found} levels"),
            IndexError::TooManyTiles { declared, ceiling } => {
                write!(f, "{rule}: {declared} tiles against a ceiling of {ceiling}")
            }
            IndexError::Level { level } => write!(f, "{rule}: level {level} is malformed"),
            IndexError::Count => write!(f, "{rule}: the tile counts do not add up"),
            IndexError::Grid { level } => {
                write!(f, "{rule}: level {level} is not the size the placement fixes")
            }
            IndexError::Order { level, ordinal } => {
                write!(f, "{rule}: tile {ordinal} of level {level} is outside the grid or out of order")
            }
            IndexError::Packing { ordinal } => {
                write!(f, "{rule}: tile {ordinal} breaks the packing rules")
            }
            IndexError::Chunk { level, chunk } => {
                write!(f, "{rule}: chunk {chunk} of level {level} does not match its tiles")
            }
            IndexError::Tile { level, col, row } => {
                write!(f, "{rule}: tile ({col}, {row}) of level {level} is repeated or outside the grid")
            }
            IndexError::TooLarge { level } => write!(f, "{rule}: level {level} does not fit its chunks"),
        }
    }
}

impl std::error::Error for IndexError {}

/// One level of the level table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Level {
    /// Tiles across.
    pub cols: u32,
    /// Tiles down.
    pub rows: u32,
    /// Kind 10 blocks the level is in, or 0 when it is inline (or has no stored tile).
    pub chunks: u32,
    /// The ordinal in the tile table of the level's first entry.
    pub first: u32,
}

/// One entry of the tile table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    /// XXH3-64 of the tile's bytes; 0 for a tile that is not stored.
    pub xxh3: u64,
    /// Column in the level.
    pub col: u32,
    /// Row in the level.
    pub row: u32,
    /// Byte offset in the chunk, or in the index block for an inline tile.
    pub offset: u32,
    /// Stored bytes; 0 means not stored.
    pub length: u32,
    /// The chunk, or [`INLINE`].
    pub chunk: u32,
}

impl Tile {
    /// Whether the tile has bytes.
    pub fn is_stored(&self) -> bool {
        self.length != 0
    }

    /// Whether the bytes are in the index block.
    pub fn is_inline(&self) -> bool {
        self.chunk == INLINE
    }
}

/// A parsed, checked index block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterIndex {
    levels: Vec<Level>,
    tiles: Vec<Tile>,
    stored: u32,
    inline: Vec<u8>,
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

impl RasterIndex {
    /// Parses and checks an index block. `max_tiles` is the ceiling on table entries (`LIMIT-14`); it is applied
    /// to the header's count before anything is sized by it.
    ///
    /// Everything the layout promises is checked: the lengths add up, each level's tiles are in the grid and in
    /// strictly ascending Hilbert order (which also means no tile twice), a length of 0 comes with zeros, and
    /// the stored bytes of each chunk and of the inline part are packed with no gap and no overlap, in table
    /// order. A block that parses is the one [`RasterIndex::encode`] writes, byte for byte.
    pub fn parse(bytes: &[u8], max_tiles: u64) -> Result<RasterIndex, IndexError> {
        let _ = (bytes, max_tiles);
        Err(IndexError::Count)
    }

    /// The block, byte for byte what [`RasterIndex::parse`] accepts.
    pub fn encode(&self) -> Vec<u8> {
        Vec::new()
    }

    /// The level table, coarsest level first.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// The whole tile table.
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// The tile table of one level, in Hilbert order.
    pub fn level_tiles(&self, level: u32) -> Option<&[Tile]> {
        let _ = level;
        None
    }

    /// How many tiles have bytes. It is the `count` of the index block's directory entry.
    pub fn stored(&self) -> u32 {
        self.stored
    }

    /// The entry of tile `(col, row)` of `level`: `None` when the level or the tile is outside the pyramid.
    /// A tile that is inside and not stored is an entry with length 0.
    pub fn tile(&self, level: u32, col: u32, row: u32) -> Option<&Tile> {
        let _ = (level, col, row);
        None
    }

    /// The bytes of an inline tile, or `None` for a tile that is stored in a chunk or not at all.
    pub fn inline_bytes(&self, tile: &Tile) -> Option<&[u8]> {
        let _ = tile;
        None
    }

    fn inline_start(&self) -> usize {
        HEADER_LEN + LEVEL_LEN * self.levels.len() + TILE_LEN * self.tiles.len()
    }

    /// Checks the levels against the pyramid the placement fixes: `levels` levels over an image of `width` by
    /// `height` pixels in tiles of `tile_edge`, level `levels - 1` at full size and each one above it halved
    /// with a ceiling. `RASTER-02`: the grid of the index is the grid of the placement, not one the file chose.
    pub fn check_grid(&self, width: u32, height: u32, tile_edge: u32) -> Result<(), IndexError> {
        let _ = (width, height, tile_edge);
        Ok(())
    }

    /// Checks the kind 10 blocks against the index: `length_of(level, chunk)` is the stored length of that
    /// chunk's block, or `None` when the file has none. A chunk is exactly its tiles, so a block shorter than
    /// the last tile ends is refused (a read would run past it) and so is one longer (bytes nobody addresses).
    pub fn check_chunks(&self, length_of: impl Fn(u32, u32) -> Option<u32>) -> Result<(), IndexError> {
        let _ = length_of;
        Ok(())
    }
}

/// The columns and rows of level `level` of a pyramid of `levels` levels over `width` by `height` pixels.
pub fn grid_of(width: u32, height: u32, tile_edge: u32, levels: u32, level: u32) -> (u32, u32) {
    let _ = (width, height, tile_edge, levels, level);
    (0, 0)
}

/// A tile as the producer hands it over: its place in the pyramid and its encoded bytes, which are stored as
/// they are. A tile with no bytes is a tile that is not stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileIn {
    /// The level, 0 being the coarsest.
    pub level: u32,
    /// The column in the level.
    pub col: u32,
    /// The row in the level.
    pub row: u32,
    /// The encoded tile (PNG, JPEG or WebP), or nothing.
    pub bytes: Vec<u8>,
}

/// One kind 10 block: the tiles of a chunk of a level, packed in table order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChunkOut {
    /// The level.
    pub level: u32,
    /// The chunk in the level.
    pub chunk: u32,
    /// The tiles in the chunk, which is the `count` of its directory entry.
    pub count: u32,
    /// The first and last column of the tiles in it.
    pub cols: [u32; 2],
    /// The first and last row of the tiles in it.
    pub rows: [u32; 2],
    /// The block.
    pub bytes: Vec<u8>,
}

/// What [`build`] makes of the tiles of one raster: the index and the chunks it points into, in the order
/// the directory will have them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Built {
    /// The index block.
    pub index: RasterIndex,
    /// The kind 10 blocks, by level and then chunk.
    pub chunks: Vec<ChunkOut>,
}

/// Builds the index and the chunks of one raster. The writer owns the order of everything: the tiles can come
/// in any order and the result does not depend on it.
///
/// `grid` is `(cols, rows)` of each level, coarsest first. The first `inline_levels` levels are stored in the
/// index block. Every other level is cut into chunks of at most `chunk_bytes` bytes (a tile is never split,
/// and a level that would pass the limit becomes several chunks). `max_tiles` is the ceiling on the size of the
/// tile table, which is `cols * rows` over the levels: a grid is refused before the table is made.
pub fn build(
    grid: &[(u32, u32)],
    tiles: Vec<TileIn>,
    inline_levels: u32,
    chunk_bytes: u32,
    max_tiles: u64,
) -> Result<Built, IndexError> {
    let _ = (grid, tiles, inline_levels, chunk_bytes, max_tiles);
    Err(IndexError::Count)
}
