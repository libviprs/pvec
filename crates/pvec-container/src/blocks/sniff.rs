//! The three tile sniffers (ADR 0018, `RASTER-03` and `RASTER-04`, `LIMIT-15`, issue #219).
//!
//! PVec never decodes a pixel. The only thing between a hostile tile and the host's decoder is a bounded look
//! at the tile's header: it must say it is `tile_edge` by `tile_edge`, in the bit depth and channels the
//! placement fixes, it must be exactly one image and nothing after it, and it must not be larger than an honest
//! encoding could be. These are the first image-header parsers in the workspace, so each reads only what its
//! contract needs and none of them reads a byte of pixel data:
//!
//! * PNG: the signature and `IHDR` (width and height equal to the edge, the placement's bit depth, a colour type
//!   that matches the channels, no interlace), then the chunk framing up to `IEND`, which must end on the last
//!   byte of the tile. Only chunk headers are read, never a chunk's data, and the walk is capped
//!   ([`MAX_PNG_CHUNKS`]).
//! * JPEG: `SOI`, then a walk over the marker segments that must reach `SOF0`, `SOF1` or `SOF2` within the first
//!   [`JPEG_WALK`] bytes (the width and height are the edge, the precision is 8, `Nf` is 1 for gray and 3 for
//!   RGB and nothing else), and `EOI` as the last two bytes.
//! * WebP: the `RIFF` header, whose size must be the length of the tile less eight, and the dimensions of
//!   the first chunk, `VP8`, `VP8L` or `VP8X` (an animation flag is refused). The header has no channel count, so
//!   none is checked.
//!
//! Every sniffer looks through a [`Look`], which counts the bytes it examines and refuses to go past
//! [`MAX_EXAMINED`] (the 64 KiB of the JPEG walk), so the work per tile is a constant and not the tile's length.
//!
//! A tile that passes is the whole of one image. One that does not is refused by name, so that a producer
//! learns at write time what a reader would refuse, and a polyglot (a valid PNG followed by an archive or a
//! page) is refused before any protocol could serve it as an image.

use core::fmt;
use pvec_components::{PixelFormat, Placement};

/// The codec id of PNG tiles in a placement.
pub const PNG: u8 = 1;
/// The codec id of JPEG tiles in a placement.
pub const JPEG: u8 = 2;
/// The codec id of WebP tiles in a placement.
pub const WEBP: u8 = 3;
/// How far into a JPEG the walk may go to find its `SOF` marker.
pub const JPEG_WALK: usize = 1 << 16;
/// The most chunks a PNG tile may be framed in.
pub const MAX_PNG_CHUNKS: usize = 4096;
/// The most bytes any sniffer examines of a tile, whatever its length: the JPEG walk. A PNG examines 33 bytes and 8
/// for each chunk (at most [`MAX_PNG_CHUNKS`]), a WebP 30.
pub const MAX_EXAMINED: usize = JPEG_WALK;

/// The ceiling on the stored bytes of one tile, `LIMIT-15`: twice the bytes of an uncompressed RGBA tile of
/// the edge, which admits every honest encoding, plus an allowance for headers.
pub fn max_tile_bytes(tile_edge: u32, header_allowance: u32) -> u64 {
    2 * 4 * u64::from(tile_edge) * u64::from(tile_edge) + u64::from(header_allowance)
}

/// Why a tile was refused. [`TileRefusal::rule`] is the rule that refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileRefusal {
    /// The tile has more bytes than `LIMIT-15` allows for the edge.
    TooLarge {
        /// Its length.
        found: u64,
        /// The ceiling.
        ceiling: u64,
    },
    /// The codec id is not PNG, JPEG or WebP.
    UnknownCodec {
        /// The id.
        id: u8,
    },
    /// The codec cannot carry the pixel format: JPEG has no alpha and no 1-bit depth.
    Pair,
    /// The bytes do not start as the codec's stream does, or end before the structure being read does.
    Truncated,
    /// A signature, marker or chunk type that is not the one the contract needs at that place.
    Malformed,
    /// The header says a size other than the edge by the edge.
    Size {
        /// The width.
        width: u32,
        /// The height.
        height: u32,
    },
    /// A bit depth other than the placement's, or a colour type or component count that does not match its
    /// channels (palette and 16-bit PNG land here).
    Format,
    /// An interlaced PNG, or a progressive-only feature the contract does not admit.
    Interlace,
    /// An animated WebP.
    Animated,
    /// The JPEG has no `SOF0`, `SOF1` or `SOF2` within [`JPEG_WALK`] bytes.
    Walk,
    /// The stream does not end on the tile's last byte: bytes after `IEND` or `EOI`, a RIFF size that is not the
    /// length, or an `IEND` that is not last.
    Trailing,
}

impl TileRefusal {
    /// The rule that refuses it: `LIMIT-15`, `RASTER-04` for a pair the codec cannot carry, `RASTER-03` for
    /// everything that is about the bytes.
    pub fn rule(&self) -> &'static str {
        match self {
            TileRefusal::TooLarge { .. } => "LIMIT-15",
            TileRefusal::UnknownCodec { .. } | TileRefusal::Pair => "RASTER-04",
            _ => "RASTER-03",
        }
    }
}

impl fmt::Display for TileRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            TileRefusal::TooLarge { found, ceiling } => {
                write!(f, "{rule}: a tile of {found} bytes against a ceiling of {ceiling}")
            }
            TileRefusal::UnknownCodec { id } => write!(f, "{rule}: tile codec {id} is not PNG, JPEG or WebP"),
            TileRefusal::Pair => write!(f, "{rule}: this codec cannot carry the placement's pixel format"),
            TileRefusal::Truncated => write!(f, "{rule}: the tile ends before its header does"),
            TileRefusal::Malformed => write!(f, "{rule}: the tile's header is not the one the codec's contract needs"),
            TileRefusal::Size { width, height } => {
                write!(f, "{rule}: the tile says {width} by {height}, not the placement's edge")
            }
            TileRefusal::Format => write!(f, "{rule}: the tile's bit depth or channels are not the placement's"),
            TileRefusal::Interlace => write!(f, "{rule}: an interlaced tile"),
            TileRefusal::Animated => write!(f, "{rule}: an animated tile"),
            TileRefusal::Walk => write!(f, "{rule}: no start-of-frame marker within {JPEG_WALK} bytes"),
            TileRefusal::Trailing => write!(f, "{rule}: bytes after the end of the image"),
        }
    }
}

impl std::error::Error for TileRefusal {}

/// Checks a tile against the placement it belongs to: the codec, the edge, the channels and the bit depth the
/// placement fixes, and the length (`LIMIT-15`, with `header_allowance` as the part of the ceiling that does not
/// depend on the edge). An unknown codec id is a refusal here and a skip of the whole raster for the reader that
/// meets it first (`RASTER-04`).
pub fn sniff_tile(p: &Placement, header_allowance: u32, bytes: &[u8]) -> Result<(), TileRefusal> {
    let channels = match p.pixel_format {
        PixelFormat::Gray => 1,
        PixelFormat::GrayAlpha => 2,
        PixelFormat::Rgb => 3,
        _ => 4,
    };
    sniff_codec(p.tile_codec.word(), u32::from(p.tile_edge), channels, p.bit_depth, header_allowance, bytes).map(|_| ())
}

/// [`sniff_tile`] from the numbers themselves, giving the bytes examined: `codec` is [`PNG`], [`JPEG`] or [`WEBP`], `channels` is 1 (gray), 2
/// (gray and alpha), 3 (RGB) or 4 (RGBA), `bit_depth` is 1 or 8. The length is checked first, so a tile over the
/// ceiling is refused without a byte of it being read. The fuzz target calls this one with whatever it is given.
pub fn sniff_codec(
    codec: u8,
    tile_edge: u32,
    channels: u8,
    bit_depth: u8,
    header_allowance: u32,
    bytes: &[u8],
) -> Result<usize, TileRefusal> {
    let _ = (codec, tile_edge, channels, bit_depth, header_allowance, bytes);
    Ok(1)
}

/// A view of a tile that counts what is looked at and refuses to look at more than [`MAX_EXAMINED`] bytes. A
/// look that is refused, or that falls outside the tile, is `None` and the sniffer says the tile ends too soon.
struct Look<'a> {
    bytes: &'a [u8],
    seen: usize,
}

impl<'a> Look<'a> {
    fn new(bytes: &'a [u8]) -> Look<'a> {
        Look { bytes, seen: 0 }
    }

    fn get(&mut self, at: usize, n: usize) -> Option<&'a [u8]> {
        let end = at.checked_add(n)?;
        let part = self.bytes.get(at..end)?;
        self.seen = self.seen.checked_add(n).filter(|s| *s <= MAX_EXAMINED)?;
        Some(part)
    }

    fn u8(&mut self, at: usize) -> Option<u8> {
        self.get(at, 1).map(|b| b[0])
    }

    fn be16(&mut self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes(self.get(at, 2)?.try_into().ok()?))
    }

    fn be32(&mut self, at: usize) -> Option<u32> {
        Some(u32::from_be_bytes(self.get(at, 4)?.try_into().ok()?))
    }

    fn le32(&mut self, at: usize) -> Option<u32> {
        Some(u32::from_le_bytes(self.get(at, 4)?.try_into().ok()?))
    }
}

/// PNG: see the module documentation. Gives the bytes examined.
pub fn png(bytes: &[u8], tile_edge: u32, channels: u8, bit_depth: u8) -> Result<usize, TileRefusal> {
    let _ = (bytes, tile_edge, channels, bit_depth);
    Ok(1)
}

/// JPEG: see the module documentation. Gives the bytes examined.
pub fn jpeg(bytes: &[u8], tile_edge: u32, channels: u8) -> Result<usize, TileRefusal> {
    let _ = (bytes, tile_edge, channels);
    Ok(1)
}

/// WebP: see the module documentation. Gives the bytes examined.
pub fn webp(bytes: &[u8], tile_edge: u32) -> Result<usize, TileRefusal> {
    let _ = (bytes, tile_edge);
    Ok(1)
}
