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
//! `docs/RASTER.md` section 2 is the specification; this is the code that writes and reads it. All integers are
//! little endian. The block is, in order and with no padding:
//!
//! | part | bytes | content |
//! |---|---|---|
//! | header | 32 | `PVX1`, `version u16` (1), `levels u16`, `tile_total u32`, `tile_stored u32`, `chunk_total u32`, `inline_len u32`, 8 reserved zero bytes |
//! | level table | 16 per level | `cols`, `rows`, `chunks`, `first`, each a `u32`, level 0 (the coarsest) first |
//! | tile table | 24 per tile | `chunk u32`, `offset u32`, `length u32`, `col u16`, `row u16`, `xxh3 u64` |
//! | inline part | `inline_len` | the bytes of the tiles whose chunk is [`INLINE`], in table order |
//!
//! `tile_total` is the number of entries of the tile table, which is `cols * rows` summed over the levels, and
//! `tile_stored` is how many of them have a non-zero length. The tile table holds the levels one after the other,
//! and inside a level the tiles are in Hilbert order ([`hilbert_key`]), strictly ascending, so a viewport is a few
//! runs of the table and the order is checked by a parser rather than trusted. `first` is the ordinal of the
//! level's first entry.
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
/// Columns or rows of a level: a tile's place is two `u16`.
pub const MAX_SIDE: u32 = 65_535;
/// The first four bytes of an index block.
pub const MAGIC: [u8; 4] = *b"PVX1";
/// The version of the layout.
pub const VERSION: u16 = 1;
/// Bytes of the header.
pub const HEADER_LEN: usize = 32;
/// Bytes of one level of the level table.
pub const LEVEL_LEN: usize = 16;
/// Bytes of one entry of the tile table.
pub const TILE_LEN: usize = 24;

/// The `subtype` of the directory entry of chunk `chunk` of level `level`: `level * 1024 + chunk`, which is
/// unique and ascends by level under LAYOUT-04. `None` when either does not fit.
pub fn subtype(level: u32, chunk: u32) -> Option<u16> {
    if level >= MAX_LEVELS || chunk >= CHUNKS_PER_LEVEL {
        return None;
    }
    u16::try_from(level * CHUNKS_PER_LEVEL + chunk).ok()
}

/// The position of tile `(col, row)` along the Hilbert curve over the square of side `side`, which is a power
/// of two at least as large as both, with the first step along x: the key of `(1, 0)` is 1 on a square of 2.
/// Distinct tiles of one level have distinct keys, and the order of a level is the order of its keys with the
/// cells outside the grid left out.
pub fn hilbert_key(side: u64, col: u32, row: u32) -> u64 {
    // the usual xy2d walks y first; swapping the axes makes the first step along x
    let (mut x, mut y) = (u64::from(row), u64::from(col));
    let mut d = 0u64;
    let mut s = side / 2;
    while s > 0 {
        let rx = u64::from(x & s != 0);
        let ry = u64::from(y & s != 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = side - 1 - x;
                y = side - 1 - y;
            }
            core::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    d
}

/// The side of the Hilbert square of a level of `cols` by `rows` tiles.
fn side_of(cols: u32, rows: u32) -> u64 {
    u64::from(cols.max(rows)).next_power_of_two()
}

/// Why an index block or a tile set was refused. [`IndexError::rule`] is `RASTER-02` for the ones about the
/// bytes, `RASTER-06` for the ones about the directory and `LIMIT-14` for [`IndexError::TooManyTiles`].
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
    /// A header that is not this layout's: the wrong magic or version, or a reserved field that is not zero.
    Header,
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
    /// number of entries before it, or with more chunks than a subtype can name, or whose chunks are not the
    /// ones its tiles name.
    Level {
        /// The level.
        level: u32,
    },
    /// The levels hold a number of tiles or chunks other than the header says, or `tile_stored` is not the number
    /// of non-zero lengths, or the inline part is not exactly the inline tiles.
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
    /// A raster named by a kind 9 or 10 block, or asked for, that the placement stream does not place
    /// (`RASTER-06`).
    NoPlacement {
        /// The raster.
        raster: u32,
    },
    /// A raster named by a kind 10 block, or asked for, that has no kind 9 block (`RASTER-06`).
    NoIndex {
        /// The raster.
        raster: u32,
    },
    /// A kind 10 block whose subtype names a level the index does not have, or a chunk the level does not have,
    /// or which is stored with a codec (`RASTER-06`): a tile is read by range, which only a block stored as it
    /// is allows.
    Block {
        /// The raster.
        raster: u32,
        /// The subtype of the block.
        subtype: u16,
    },
}

impl IndexError {
    /// The rule of `docs/RASTER.md` and `docs/SECURITY_LIMITS.md` that refuses it.
    pub fn rule(&self) -> &'static str {
        match self {
            IndexError::TooManyTiles { .. } => "LIMIT-14",
            IndexError::NoIndex { .. } | IndexError::NoPlacement { .. } | IndexError::Block { .. } => {
                "RASTER-06"
            }
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
            IndexError::Header => write!(f, "{rule}: the header is not a version 1 index header"),
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
            IndexError::NoIndex { raster } => write!(f, "{rule}: raster {raster} has no index block"),
            IndexError::NoPlacement { raster } => {
                write!(f, "{rule}: raster {raster} has no placement in the stream")
            }
            IndexError::Block { raster, subtype } => {
                write!(f, "{rule}: the tile block {subtype} of raster {raster} is not a chunk the index has")
            }
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
        if bytes.len() < HEADER_LEN {
            return Err(IndexError::Length { expected: HEADER_LEN as u64, found: bytes.len() as u64 });
        }
        if bytes[..4] != MAGIC
            || u16::from_le_bytes([bytes[4], bytes[5]]) != VERSION
            || bytes[24..32].iter().any(|b| *b != 0)
        {
            return Err(IndexError::Header);
        }
        let levels = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
        let (tiles, stored, chunk_total, inline_len) =
            (u32_at(bytes, 8), u32_at(bytes, 12), u32_at(bytes, 16), u32_at(bytes, 20));
        if levels == 0 || levels > MAX_LEVELS {
            return Err(IndexError::Levels { found: levels });
        }
        if u64::from(tiles) > max_tiles {
            return Err(IndexError::TooManyTiles { declared: u64::from(tiles), ceiling: max_tiles });
        }
        let expected = HEADER_LEN as u64
            + LEVEL_LEN as u64 * u64::from(levels)
            + TILE_LEN as u64 * u64::from(tiles)
            + u64::from(inline_len);
        if expected != bytes.len() as u64 {
            return Err(IndexError::Length { expected, found: bytes.len() as u64 });
        }
        // from here the declared counts are bounded by the length of the block itself
        let mut level_table = Vec::with_capacity(levels as usize);
        let (mut first, mut chunks_seen) = (0u64, 0u64);
        for l in 0..levels {
            let at = HEADER_LEN + LEVEL_LEN * l as usize;
            let level = Level {
                cols: u32_at(bytes, at),
                rows: u32_at(bytes, at + 4),
                chunks: u32_at(bytes, at + 8),
                first: u32_at(bytes, at + 12),
            };
            let in_grid = (1..=MAX_SIDE).contains(&level.cols) && (1..=MAX_SIDE).contains(&level.rows);
            if !in_grid || level.chunks > CHUNKS_PER_LEVEL || u64::from(level.first) != first {
                return Err(IndexError::Level { level: l });
            }
            first += u64::from(level.cols) * u64::from(level.rows);
            chunks_seen += u64::from(level.chunks);
            level_table.push(level);
        }
        if first != u64::from(tiles) || chunks_seen != u64::from(chunk_total) {
            return Err(IndexError::Count);
        }
        let table_at = HEADER_LEN + LEVEL_LEN * levels as usize;
        let inline_at = table_at + TILE_LEN * tiles as usize;
        let mut table = Vec::with_capacity(tiles as usize);
        let mut counted = 0u32;
        let mut inline_next = inline_at as u64;
        let mut chunked_before = false;
        for (l, level) in level_table.iter().enumerate() {
            let l = l as u32;
            let side = side_of(level.cols, level.rows);
            let mut previous_key: Option<u64> = None;
            let mut chunk = 0u32;
            let mut chunk_next = 0u64;
            let mut seen_chunk = false;
            let n = u64::from(level.cols) * u64::from(level.rows);
            for k in 0..n {
                let ordinal = (u64::from(level.first) + k) as u32;
                let at = table_at + TILE_LEN * ordinal as usize;
                let t = Tile {
                    chunk: u32_at(bytes, at),
                    offset: u32_at(bytes, at + 4),
                    length: u32_at(bytes, at + 8),
                    col: u32::from(u16::from_le_bytes([bytes[at + 12], bytes[at + 13]])),
                    row: u32::from(u16::from_le_bytes([bytes[at + 14], bytes[at + 15]])),
                    xxh3: u64_at(bytes, at + 16),
                };
                if t.col >= level.cols || t.row >= level.rows {
                    return Err(IndexError::Order { level: l, ordinal });
                }
                let key = hilbert_key(side, t.col, t.row);
                if previous_key.is_some_and(|p| p >= key) {
                    return Err(IndexError::Order { level: l, ordinal });
                }
                previous_key = Some(key);
                if t.length == 0 {
                    if t.xxh3 != 0 || t.offset != 0 || t.chunk != 0 {
                        return Err(IndexError::Packing { ordinal });
                    }
                } else {
                    counted += 1;
                    if t.chunk == INLINE {
                        // the inline part is the inline tiles one after the other, from its first byte to the end,
                        // and the inline levels are the coarsest ones: none comes after a level in chunks
                        if level.chunks != 0 || chunked_before || u64::from(t.offset) != inline_next {
                            return Err(IndexError::Packing { ordinal });
                        }
                        inline_next += u64::from(t.length);
                        if inline_next > bytes.len() as u64 {
                            return Err(IndexError::Packing { ordinal });
                        }
                    } else {
                        if t.chunk >= level.chunks {
                            return Err(IndexError::Packing { ordinal });
                        }
                        // chunks ascend by one, and a chunk's tiles are packed from its first byte
                        if !seen_chunk {
                            if t.chunk != 0 {
                                return Err(IndexError::Packing { ordinal });
                            }
                            seen_chunk = true;
                        } else if t.chunk == chunk + 1 {
                            chunk = t.chunk;
                            chunk_next = 0;
                        } else if t.chunk != chunk {
                            return Err(IndexError::Packing { ordinal });
                        }
                        if u64::from(t.offset) != chunk_next {
                            return Err(IndexError::Packing { ordinal });
                        }
                        chunk_next += u64::from(t.length);
                        if chunk_next > u64::from(u32::MAX) {
                            return Err(IndexError::Packing { ordinal });
                        }
                    }
                }
                table.push(t);
            }
            // every chunk the level names has a tile in it, and none of the level is left unnamed
            if level.chunks != 0 && (!seen_chunk || chunk + 1 != level.chunks) {
                return Err(IndexError::Level { level: l });
            }
            chunked_before |= level.chunks != 0;
        }
        if counted != stored || inline_next != bytes.len() as u64 {
            return Err(IndexError::Count);
        }
        Ok(RasterIndex { levels: level_table, tiles: table, stored, inline: bytes[inline_at..].to_vec() })
    }

    /// The block, byte for byte what [`RasterIndex::parse`] accepts.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            HEADER_LEN + LEVEL_LEN * self.levels.len() + TILE_LEN * self.tiles.len() + self.inline.len(),
        );
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(self.levels.len() as u16).to_le_bytes());
        out.extend_from_slice(&(self.tiles.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.stored.to_le_bytes());
        out.extend_from_slice(&self.levels.iter().map(|l| l.chunks).sum::<u32>().to_le_bytes());
        out.extend_from_slice(&(self.inline.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0; 8]);
        for l in &self.levels {
            for v in [l.cols, l.rows, l.chunks, l.first] {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for t in &self.tiles {
            for v in [t.chunk, t.offset, t.length] {
                out.extend_from_slice(&v.to_le_bytes());
            }
            // a column and a row are at most MAX_SIDE, which the builder and the parser both hold
            out.extend_from_slice(&(t.col as u16).to_le_bytes());
            out.extend_from_slice(&(t.row as u16).to_le_bytes());
            out.extend_from_slice(&t.xxh3.to_le_bytes());
        }
        out.extend_from_slice(&self.inline);
        out
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
        let l = self.levels.get(level as usize)?;
        let first = l.first as usize;
        self.tiles.get(first..first + l.cols as usize * l.rows as usize)
    }

    /// How many tiles have bytes. It is the `count` of the index block's directory entry.
    pub fn stored(&self) -> u32 {
        self.stored
    }

    /// The entry of tile `(col, row)` of `level`: `None` when the level or the tile is outside the pyramid.
    /// A tile that is inside and not stored is an entry with length 0.
    pub fn tile(&self, level: u32, col: u32, row: u32) -> Option<&Tile> {
        let l = self.levels.get(level as usize)?;
        if col >= l.cols || row >= l.rows {
            return None;
        }
        let tiles = self.level_tiles(level)?;
        let side = side_of(l.cols, l.rows);
        let key = hilbert_key(side, col, row);
        let at = tiles.partition_point(|t| hilbert_key(side, t.col, t.row) < key);
        tiles.get(at).filter(|t| t.col == col && t.row == row)
    }

    /// The bytes of an inline tile, or `None` for a tile that is stored in a chunk or not at all.
    pub fn inline_bytes(&self, tile: &Tile) -> Option<&[u8]> {
        if !tile.is_stored() || !tile.is_inline() {
            return None;
        }
        // offsets are positions in the block, and the inline part is its tail
        let start = (tile.offset as usize)
            .checked_sub(HEADER_LEN + LEVEL_LEN * self.levels.len() + TILE_LEN * self.tiles.len())?;
        self.inline.get(start..start.checked_add(tile.length as usize)?)
    }

    /// Checks the levels against the pyramid the placement fixes: `levels` levels over an image of `width` by
    /// `height` pixels in tiles of `tile_edge`, level `levels - 1` at full size and each one above it halved
    /// with a ceiling. `RASTER-02`: the grid of the index is the grid of the placement, not one the file chose.
    pub fn check_grid(&self, width: u32, height: u32, tile_edge: u32) -> Result<(), IndexError> {
        let n = self.levels.len() as u32;
        for (l, level) in self.levels.iter().enumerate() {
            let l = l as u32;
            let (cols, rows) = grid_of(width, height, tile_edge, n, l);
            if (level.cols, level.rows) != (cols, rows) {
                return Err(IndexError::Grid { level: l });
            }
        }
        Ok(())
    }

    /// What the tiles of `level` put in each of its chunks, in one pass over the table: for chunk `k`, the number of
    /// stored tiles and the end of the last one (the length the chunk's block has to be). Empty for a level that
    /// is inline or not in the pyramid.
    pub fn chunk_stats(&self, level: u32) -> Vec<(u32, u64)> {
        let Some(l) = self.levels.get(level as usize) else { return Vec::new() };
        let mut stats = vec![(0u32, 0u64); l.chunks as usize];
        for t in self.level_tiles(level).into_iter().flatten().filter(|t| t.is_stored() && !t.is_inline()) {
            if let Some(slot) = stats.get_mut(t.chunk as usize) {
                slot.0 += 1;
                slot.1 = slot.1.max(u64::from(t.offset) + u64::from(t.length));
            }
        }
        stats
    }

    /// Checks the kind 10 blocks against the index: `length_of(level, chunk)` is the stored length of that
    /// chunk's block, or `None` when the file has none. A chunk is exactly its tiles, so a block shorter than
    /// the last tile ends is refused (a read would run past it) and so is one longer (bytes nobody addresses).
    pub fn check_chunks(&self, length_of: impl Fn(u32, u32) -> Option<u32>) -> Result<(), IndexError> {
        for l in 0..self.levels.len() as u32 {
            for (chunk, (_, end)) in self.chunk_stats(l).into_iter().enumerate() {
                if Some(end) != length_of(l, chunk as u32).map(u64::from) {
                    return Err(IndexError::Chunk { level: l, chunk: chunk as u32 });
                }
            }
        }
        Ok(())
    }
}

/// The columns and rows of level `level` of a pyramid of `levels` levels over `width` by `height` pixels.
pub fn grid_of(width: u32, height: u32, tile_edge: u32, levels: u32, level: u32) -> (u32, u32) {
    let shift = levels.saturating_sub(1).saturating_sub(level).min(63);
    let scaled = |v: u32| (u64::from(v)).div_ceil(1u64 << shift).max(1);
    let cols = scaled(width).div_ceil(u64::from(tile_edge.max(1)));
    let rows = scaled(height).div_ceil(u64::from(tile_edge.max(1)));
    (u32::try_from(cols).unwrap_or(u32::MAX), u32::try_from(rows).unwrap_or(u32::MAX))
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
    let metas: Vec<TileMeta> = tiles
        .iter()
        .map(|t| TileMeta { level: t.level, col: t.col, row: t.row, len: t.bytes.len() as u64 })
        .collect();
    let mut bytes: Vec<Vec<u8>> = tiles.into_iter().map(|t| t.bytes).collect();
    let mut chunks = Vec::new();
    let index = build_with(
        grid,
        &metas,
        &mut |i| Ok::<_, core::convert::Infallible>(std::mem::take(&mut bytes[i])),
        &mut |c| {
            chunks.push(c);
            Ok(())
        },
        inline_levels,
        chunk_bytes,
        max_tiles,
    )
    .map_err(|f| match f {
        BuildFailure::Index(e) => e,
        BuildFailure::Source(never) => match never {},
    })?;
    Ok(Built { index, chunks })
}

/// Where a tile is and how long it is, without its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TileMeta {
    /// The level, 0 being the coarsest.
    pub level: u32,
    /// The column in the level.
    pub col: u32,
    /// The row in the level.
    pub row: u32,
    /// The bytes of the encoded tile; 0 is a tile that is not stored.
    pub len: u64,
}

/// Why [`build_with`] stopped: the container cannot store the tiles, or the caller's source or sink failed.
#[derive(Debug, PartialEq, Eq)]
pub enum BuildFailure<E> {
    /// The tiles are refused.
    Index(IndexError),
    /// The closure that loads a tile, or the one that takes a chunk, failed.
    Source(E),
}

impl<E> From<IndexError> for BuildFailure<E> {
    fn from(e: IndexError) -> BuildFailure<E> {
        BuildFailure::Index(e)
    }
}

/// [`build`] over tiles that are not in memory: `metas` says where each is, `load(i)` returns the bytes of
/// `metas[i]` when the build reaches it, and `sink` takes each chunk as soon as it is full, so no more than one
/// chunk and one tile are held at a time. The chunks come out in the order [`Built::chunks`] has them, and the
/// index is the one `build` returns for the same tiles.
pub fn build_with<E>(
    grid: &[(u32, u32)],
    metas: &[TileMeta],
    load: &mut dyn FnMut(usize) -> Result<Vec<u8>, E>,
    sink: &mut dyn FnMut(ChunkOut) -> Result<(), E>,
    inline_levels: u32,
    chunk_bytes: u32,
    max_tiles: u64,
) -> Result<RasterIndex, BuildFailure<E>> {
    let n_levels = grid.len() as u32;
    if n_levels == 0 || n_levels > MAX_LEVELS {
        return Err(IndexError::Levels { found: n_levels }.into());
    }
    let mut total = 0u64;
    for (l, &(cols, rows)) in grid.iter().enumerate() {
        if !(1..=MAX_SIDE).contains(&cols) || !(1..=MAX_SIDE).contains(&rows) {
            return Err(IndexError::Level { level: l as u32 }.into());
        }
        total += u64::from(cols) * u64::from(rows);
    }
    if total > max_tiles || total > u64::from(u32::MAX) {
        return Err(IndexError::TooManyTiles {
            declared: total,
            ceiling: max_tiles.min(u64::from(u32::MAX)),
        }
        .into());
    }
    // the tiles in the order of the table, so that the table can be walked once
    let mut given: Vec<(u32, u64, usize)> = Vec::with_capacity(metas.len());
    for (i, t) in metas.iter().enumerate() {
        let &(cols, rows) = grid
            .get(t.level as usize)
            .filter(|(c, r)| t.col < *c && t.row < *r)
            .ok_or(IndexError::Tile { level: t.level, col: t.col, row: t.row })?;
        given.push((t.level, hilbert_key(side_of(cols, rows), t.col, t.row), i));
    }
    given.sort_unstable_by_key(|(level, key, i)| (*level, *key, *i));
    if let Some(w) = given.windows(2).find(|w| (w[0].0, w[0].1) == (w[1].0, w[1].1)) {
        let t = metas[w[1].2];
        return Err(IndexError::Tile { level: t.level, col: t.col, row: t.row }.into());
    }
    let mut given = given.into_iter().peekable();

    let mut levels: Vec<Level> = Vec::with_capacity(grid.len());
    let mut table: Vec<Tile> = Vec::with_capacity(total as usize);
    let mut inline: Vec<u8> = Vec::new();
    let mut stored = 0u32;
    let inline_at = (HEADER_LEN + LEVEL_LEN * grid.len() + TILE_LEN * total as usize) as u64;
    for (l, &(cols, rows)) in grid.iter().enumerate() {
        let l = l as u32;
        let side = side_of(cols, rows);
        let mut cells: Vec<(u64, u32, u32)> = Vec::with_capacity(cols as usize * rows as usize);
        for row in 0..rows {
            for col in 0..cols {
                cells.push((hilbert_key(side, col, row), col, row));
            }
        }
        cells.sort_unstable();
        let first = table.len() as u32;
        let is_inline = l < inline_levels;
        let mut level_chunks = 0u32;
        let mut current: Option<ChunkOut> = None;
        for (key, col, row) in cells {
            let mut bytes = Vec::new();
            if let Some((_, _, i)) = given.next_if(|(gl, gk, _)| (*gl, *gk) == (l, key)) {
                // a tile with no bytes is not stored, and a source need not be asked for it
                if metas[i].len != 0 {
                    bytes = load(i).map_err(BuildFailure::Source)?;
                }
            }
            if bytes.is_empty() {
                table.push(Tile { xxh3: 0, col, row, offset: 0, length: 0, chunk: 0 });
                continue;
            }
            let length = u32::try_from(bytes.len()).map_err(|_| IndexError::TooLarge { level: l })?;
            stored += 1;
            let xxh3 = block_checksum(&bytes);
            if is_inline {
                let offset = inline_at + inline.len() as u64;
                let end = offset + u64::from(length);
                let offset = u32::try_from(offset).map_err(|_| IndexError::TooLarge { level: l })?;
                if u32::try_from(end).is_err() {
                    return Err(IndexError::TooLarge { level: l }.into());
                }
                inline.extend_from_slice(&bytes);
                table.push(Tile { xxh3, col, row, offset, length, chunk: INLINE });
                continue;
            }
            if length > chunk_bytes {
                return Err(IndexError::TooLarge { level: l }.into());
            }
            // a chunk takes the next tile unless that would pass the limit, and it is never empty
            if current
                .as_ref()
                .is_some_and(|c| c.bytes.len() as u64 + u64::from(length) > u64::from(chunk_bytes))
                && let Some(done) = current.take()
            {
                sink(done).map_err(BuildFailure::Source)?;
            }
            let c = current.get_or_insert_with(|| {
                let c = ChunkOut {
                    level: l,
                    chunk: level_chunks,
                    count: 0,
                    cols: [col, col],
                    rows: [row, row],
                    bytes: Vec::new(),
                };
                level_chunks += 1;
                c
            });
            if c.chunk >= CHUNKS_PER_LEVEL {
                return Err(IndexError::TooLarge { level: l }.into());
            }
            table.push(Tile { xxh3, col, row, offset: c.bytes.len() as u32, length, chunk: c.chunk });
            c.bytes.extend_from_slice(&bytes);
            c.count += 1;
            c.cols = [c.cols[0].min(col), c.cols[1].max(col)];
            c.rows = [c.rows[0].min(row), c.rows[1].max(row)];
        }
        if let Some(done) = current.take() {
            sink(done).map_err(BuildFailure::Source)?;
        }
        levels.push(Level { cols, rows, chunks: level_chunks, first });
    }
    Ok(RasterIndex { levels, tiles: table, stored, inline })
}
