//! The raster placement stream (issue 217, ADR 0018): where an image sits in the drawing and how its tiles are
//! laid out, and nothing of the pixels.
//!
//! A raster is not a component of an entity: it is a placement in the drawing's own source space. The stream is
//! capability 21, keyed by a dense [`RasterId`] that is the position of the record in the stream, and it is
//! read whole at open because it is small. The tiles themselves are in two block kinds that capability owns
//! (issue 218) and are stored encoded, so this crate has no image decoder and no geometry: the only thing it
//! asks of the numeric policy is NP-19 for the map from pixels into source space. The byte layout is
//! `docs/RASTER.md` section 1.
//!
//! The rules refuse the raster and never the file: [`RasterError`] carries the `RASTER-NN` rule, LIMIT-13 for
//! the count, and the `COMP-NN` rule of a damaged payload, and a reader that meets one drops this stream and
//! keeps the linework. An unknown tile codec is not an error at all: the raster keeps its id and
//! [`Placement::skip`] says why it is not drawn.

use crate::error::ComponentError;
use crate::frame::Opened;
use crate::payloads::{R, W};
use pvec_capabilities::CapabilityId;
use pvec_numeric::{NumericError, Policy, check_value};
use std::fmt;

/// The registry capability of the raster stream: the next dense id after `semantic_class` (20).
pub const RASTER_CAPABILITY: CapabilityId = CapabilityId::new(21);

/// LIMIT-13: the most placements one artifact may declare. `Limits::max_rasters` of `pvec-container` takes its
/// default from this, so the two cannot disagree.
pub const MAX_RASTERS: u32 = 4_096;

/// Bytes of the fixed part of one record, before the digest and the attribution.
const ROW: usize = 88;
/// The stored `layer_index` that means none.
const NO_LAYER: u32 = u32::MAX;
/// Largest `cols` or `rows` of a level, which the 16-bit fields of the index block can hold.
const MAX_GRID: u64 = 65_535;
/// Longest attribution, in bytes, and the length of a source digest.
const MAX_ATTRIBUTION: usize = 255;
const DIGEST: usize = 32;

/// The position of a raster in the stream: dense from 0 in the producer's source draw order. A raster is not an
/// entity, so this is not an `EntityId`; it is the `cell` of its directory entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RasterId(u32);

impl RasterId {
    /// The id at `index`. LIMIT-13 is enforced here, so an id that exists is one a directory entry can name.
    pub fn new(index: u32) -> Result<RasterId, RasterError> {
        if index < MAX_RASTERS {
            Ok(RasterId(index))
        } else {
            Err(RasterError::TooMany { declared: u64::from(index) + 1 })
        }
    }

    /// The position in the stream.
    pub fn index(self) -> u32 {
        self.0
    }

    /// The `cell` field of the directory entries of this raster's blocks. The field is namespaced by block
    /// kind and a reader filters by kind first, so it cannot be confused with a leaf of the spatial directory.
    pub fn cell(self) -> u32 {
        self.0
    }
}

/// How the tiles are encoded: the values PMTiles types 2, 3 and 4 name. An id this build does not know is kept,
/// so a placement written by a newer producer survives a round trip, and the raster is skipped and reported
/// (see [`Placement::skip`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileCodec {
    /// PNG.
    Png,
    /// JPEG: only with gray or RGB at eight bits (RASTER-04).
    Jpeg,
    /// WebP, lossless or lossy.
    Webp,
    /// An id this build does not know.
    Unknown(u8),
}

impl TileCodec {
    /// The wire byte.
    pub fn word(self) -> u8 {
        match self {
            TileCodec::Png => 1,
            TileCodec::Jpeg => 2,
            TileCodec::Webp => 3,
            TileCodec::Unknown(w) => w,
        }
    }

    /// The codec with this wire byte; every byte maps to something, because an unknown id skips the raster
    /// and is not an error.
    pub fn from_word(w: u8) -> TileCodec {
        match w {
            1 => TileCodec::Png,
            2 => TileCodec::Jpeg,
            3 => TileCodec::Webp,
            other => TileCodec::Unknown(other),
        }
    }
}

/// The channels of a decoded tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    /// One channel.
    Gray,
    /// Gray and alpha.
    GrayAlpha,
    /// Red, green, blue.
    Rgb,
    /// Red, green, blue and alpha.
    Rgba,
}

impl PixelFormat {
    /// The wire byte.
    pub fn word(self) -> u8 {
        match self {
            PixelFormat::Gray => 0,
            PixelFormat::GrayAlpha => 1,
            PixelFormat::Rgb => 2,
            PixelFormat::Rgba => 3,
        }
    }

    /// The format with this wire byte, or `None` for one outside the closed set.
    pub fn from_word(w: u8) -> Option<PixelFormat> {
        match w {
            0 => Some(PixelFormat::Gray),
            1 => Some(PixelFormat::GrayAlpha),
            2 => Some(PixelFormat::Rgb),
            3 => Some(PixelFormat::Rgba),
            _ => None,
        }
    }
}

/// Where the raster is drawn against the vectors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Band {
    /// Below every hatch fill and every stroke.
    Underlay,
    /// Above both.
    Overlay,
}

impl Band {
    /// The wire byte.
    pub fn word(self) -> u8 {
        match self {
            Band::Underlay => 0,
            Band::Overlay => 1,
        }
    }

    /// The band with this wire byte, or `None` for one outside the closed set.
    pub fn from_word(w: u8) -> Option<Band> {
        match w {
            0 => Some(Band::Underlay),
            1 => Some(Band::Overlay),
            _ => None,
        }
    }
}

/// Why a raster is kept in the stream and not drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterSkip {
    /// The tile codec id is one this build does not know.
    UnknownCodec(u8),
}

/// One raster: the map from its pixels into source space and the shape of its tile grid. The fields vary
/// together per raster, which is why they are one struct.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// `[a b c d tx ty]` in `Transform2` order: `x = a*px + b*py + tx`, `y = c*px + d*py + ty`, the pixel origin
    /// at the top-left corner of pixel (0, 0) and pixel y down. Source space is y-up, so an upright image has
    /// `d < 0`. NP-19 holds it, with the 2^20 bound on the four linear entries (RASTER-05).
    pub pixel_to_source: [f64; 6],
    /// Pixels across the full resolution level, at least 1.
    pub width: u32,
    /// Pixels down the full resolution level, at least 1.
    pub height: u32,
    /// `[x0 y0 x1 y1]`: the pixel rectangle that is drawn and that counts toward the extent, `x0` and `y0`
    /// inclusive, `x1` and `y1` exclusive, non-empty and inside the image (RASTER-01).
    pub visible_box: [u32; 4],
    /// 256, 512 or 1024 (RASTER-01).
    pub tile_edge: u16,
    /// The number of pyramid levels: the pyramid stops at the first level that is one tile (RASTER-01).
    pub levels: u8,
    /// How the tiles are encoded.
    pub tile_codec: TileCodec,
    /// The channels of a decoded tile.
    pub pixel_format: PixelFormat,
    /// Bits per channel: 1 or 8.
    pub bit_depth: u8,
    /// 255 is opaque, multiplied into the fragment alpha.
    pub opacity: u8,
    /// Underlay or overlay.
    pub band: Band,
    /// A row of the layer stream's name table, ignored when that stream is skipped. A raster on a layer the
    /// view profile excludes is not written.
    pub layer_index: Option<u32>,
    /// SHA-256 of the source raster, provenance only: shown as hex and never followed as a link.
    pub source_digest: Option<[u8; 32]>,
    /// Plain text of at most 255 bytes, empty for none (RASTER-07). A host escapes it before an HTML sink.
    pub attribution: String,
}

impl Placement {
    /// Why this raster is not drawn, when it is not.
    pub fn skip(&self) -> Option<RasterSkip> {
        match self.tile_codec {
            TileCodec::Unknown(id) => Some(RasterSkip::UnknownCodec(id)),
            _ => None,
        }
    }

    /// The fewest levels whose coarsest is one tile: the smallest `n` with `max(width, height) <= tile_edge * 2^(n-1)`.
    fn expected_levels(&self) -> u8 {
        let longest = u64::from(self.width.max(self.height));
        let mut levels = 1u8;
        while (u64::from(self.tile_edge) << (levels - 1)) < longest {
            levels += 1;
        }
        levels
    }

    /// The rules that refuse one raster, in the order RASTER-01, 04, 05, 07.
    fn check(&self) -> Result<(), RasterError> {
        let [x0, y0, x1, y1] = self.visible_box;
        let cells = |n: u32| u64::from(n).div_ceil(u64::from(self.tile_edge));
        if !matches!(self.tile_edge, 256 | 512 | 1024)
            || self.width == 0
            || self.height == 0
            || cells(self.width) > MAX_GRID
            || cells(self.height) > MAX_GRID
            || self.levels != self.expected_levels()
            || x0 >= x1
            || y0 >= y1
            || x1 > self.width
            || y1 > self.height
        {
            return Err(RasterError::BadGrid);
        }
        // JPEG has no alpha and no bilevel form; an unknown codec is skipped, so its pair is not judged
        if self.tile_codec == TileCodec::Jpeg
            && (!matches!(self.pixel_format, PixelFormat::Gray | PixelFormat::Rgb) || self.bit_depth != 8)
        {
            return Err(RasterError::BadFormat);
        }
        Policy::V1.validate_affine(self.pixel_to_source).map_err(RasterError::BadTransform)?;
        let [a, b, c, d, tx, ty] = self.pixel_to_source;
        let (w, h) = (f64::from(self.width), f64::from(self.height));
        for (px, py) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
            check_value(a * px + b * py + tx).map_err(RasterError::BadTransform)?;
            check_value(c * px + d * py + ty).map_err(RasterError::BadTransform)?;
        }
        let bad_char = |ch: char| {
            ch.is_control()
                || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        };
        if self.attribution.len() > MAX_ATTRIBUTION || self.attribution.chars().any(bad_char) {
            return Err(RasterError::BadText);
        }
        Ok(())
    }
}

/// Why a raster, or the stream it is in, was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterError {
    /// LIMIT-13: more placements than `MAX_RASTERS`. `declared` is what the stream asked for.
    TooMany {
        /// What the stream declared.
        declared: u64,
    },
    /// RASTER-01: the tile edge, the levels, the grid or the visible box is not one the format allows.
    BadGrid,
    /// RASTER-04: a codec and pixel format that cannot go together.
    BadFormat,
    /// RASTER-05: the map from pixels to source space fails NP-19, or a corner of the footprint fails NP-02.
    BadTransform(NumericError),
    /// RASTER-07: an attribution that is not plain text of at most 255 bytes, or a digest that is not 32 bytes.
    BadText,
    /// The payload as a whole is damaged: truncated, with trailing bytes, a reserved field set or an undefined
    /// tag. The rule is the `COMP-NN` one.
    Stream(ComponentError),
}

impl From<ComponentError> for RasterError {
    fn from(e: ComponentError) -> RasterError {
        RasterError::Stream(e)
    }
}

impl RasterError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            RasterError::TooMany { .. } => "LIMIT-13",
            RasterError::BadGrid => "RASTER-01",
            RasterError::BadFormat => "RASTER-04",
            RasterError::BadTransform(_) => "RASTER-05",
            RasterError::BadText => "RASTER-07",
            RasterError::Stream(e) => e.rule(),
        }
    }
}

impl fmt::Display for RasterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.rule();
        match self {
            RasterError::TooMany { declared } => {
                write!(f, "{declared} rasters against a ceiling of {MAX_RASTERS} ({r})")
            }
            RasterError::BadGrid => write!(f, "the tile grid or the visible box is not valid ({r})"),
            RasterError::BadFormat => write!(f, "the tile codec cannot carry this pixel format ({r})"),
            RasterError::BadTransform(e) => write!(f, "the pixel to source map is refused: {e:?} ({r})"),
            RasterError::BadText => write!(f, "the attribution or the digest is not valid ({r})"),
            RasterError::Stream(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RasterError {}

/// The rows of the error table of `docs/RASTER.md`: rule, error and meaning. A test renders the table from this
/// and fails if the document differs.
pub const RASTER_ERRORS: &[(&str, &str, &str)] = &[
    ("LIMIT-13", "TooMany", "more placements than the ceiling of 4,096, refused before the table is allocated"),
    (
        "RASTER-01",
        "BadGrid",
        "a tile edge outside 256, 512 and 1024, a level count other than the fewest whose coarsest level is one tile, a grid wider than 65,535 tiles, or a visible box that is empty or outside the image",
    ),
    (
        "RASTER-04",
        "BadFormat",
        "JPEG with anything but gray or RGB at eight bits; an unknown codec id is not an error, it skips the raster",
    ),
    (
        "RASTER-05",
        "BadTransform",
        "a pixel to source map that fails NP-19 (a non-finite entry, a linear entry above 2^20, a singular map under PVec's rule) or a footprint corner that fails NP-02",
    ),
    (
        "RASTER-07",
        "BadText",
        "an attribution over 255 bytes, not UTF-8, or with a control character, a line or paragraph separator or a bidi override; a digest that is not 32 bytes",
    ),
    (
        "COMP-NN",
        "Stream",
        "the payload is damaged: truncated (COMP-01), with trailing bytes (COMP-02), a reserved field set (COMP-12) or an undefined tag (COMP-11)",
    ),
];

/// The payload of the raster stream for `placements`, in `RasterId` order. Refuses what a reader would refuse,
/// and writes `-0.0` as `+0.0` (NP-03) so the bits compare equal after a round trip.
pub fn encode_rasters(placements: &[Placement]) -> Result<Vec<u8>, RasterError> {
    if placements.len() > MAX_RASTERS as usize {
        return Err(RasterError::TooMany { declared: placements.len() as u64 });
    }
    let mut w = W::default();
    w.u32(placements.len() as u32);
    for p in placements {
        p.check()?;
        if p.layer_index == Some(NO_LAYER) {
            return Err(RasterError::Stream(ComponentError::IndexOutOfRange));
        }
        for x in p.pixel_to_source {
            w.f64(check_value(x).map_err(RasterError::BadTransform)?);
        }
        w.u32(p.width);
        w.u32(p.height);
        p.visible_box.iter().for_each(|v| w.u32(*v));
        w.0.extend_from_slice(&p.tile_edge.to_le_bytes());
        w.u8(p.levels);
        w.u8(p.tile_codec.word());
        w.u8(p.pixel_format.word());
        w.u8(p.bit_depth);
        w.u8(p.opacity);
        w.u8(p.band.word());
        w.u32(p.layer_index.unwrap_or(NO_LAYER));
        w.u8(if p.source_digest.is_some() { DIGEST as u8 } else { 0 });
        w.u8(p.attribution.len() as u8);
        w.0.extend_from_slice(&[0, 0]);
        if let Some(d) = &p.source_digest {
            w.0.extend_from_slice(d);
        }
        w.0.extend_from_slice(p.attribution.as_bytes());
    }
    Ok(w.0)
}

/// Reads a payload, which must be consumed exactly. The count is held to LIMIT-13 before the bytes are looked at
/// and before anything is allocated from it, and every record is held to the rules of [`Placement`].
pub fn decode_rasters(bytes: &[u8]) -> Result<Vec<Placement>, RasterError> {
    let mut r = R::new(bytes);
    let n = r.u32()?;
    if n > MAX_RASTERS {
        return Err(RasterError::TooMany { declared: u64::from(n) });
    }
    if (n as usize) * ROW > r.left() {
        return Err(ComponentError::Truncated.into());
    }
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let mut pixel_to_source = [0.0; 6];
        for x in &mut pixel_to_source {
            *x = r.f64()?;
        }
        let (width, height) = (r.u32()?, r.u32()?);
        let visible_box = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let tile_edge = u16::from_le_bytes([r.u8()?, r.u8()?]);
        let (levels, codec, format, bit_depth, opacity, band) =
            (r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
        let layer = r.u32()?;
        let (digest_len, attribution_len) = (usize::from(r.u8()?), usize::from(r.u8()?));
        if r.take(2)? != [0, 0] {
            return Err(ComponentError::ReservedNotZero.into());
        }
        let pixel_format = PixelFormat::from_word(format).ok_or(ComponentError::BadTag(u32::from(format)))?;
        let band = Band::from_word(band).ok_or(ComponentError::BadTag(u32::from(band)))?;
        if !matches!(bit_depth, 1 | 8) {
            return Err(ComponentError::BadTag(u32::from(bit_depth)).into());
        }
        if digest_len != 0 && digest_len != DIGEST {
            return Err(RasterError::BadText);
        }
        let source_digest: Option<[u8; DIGEST]> = match digest_len {
            0 => None,
            _ => r.take(DIGEST)?.try_into().ok(),
        };
        let attribution =
            String::from_utf8(r.take(attribution_len)?.to_vec()).map_err(|_| RasterError::BadText)?;
        let mut p = Placement {
            pixel_to_source,
            width,
            height,
            visible_box,
            tile_edge,
            levels,
            tile_codec: TileCodec::from_word(codec),
            pixel_format,
            bit_depth,
            opacity,
            band,
            layer_index: (layer != NO_LAYER).then_some(layer),
            source_digest,
            attribution,
        };
        p.check()?;
        for x in &mut p.pixel_to_source {
            *x = check_value(*x).map_err(RasterError::BadTransform)?;
        }
        out.push(p);
    }
    r.done()?;
    Ok(out)
}

/// What a reader gets from the raster stream of an opened set. A damaged payload fails this stream only: it is
/// reported here, and nothing about it reaches the geometry or the other streams.
#[derive(Clone, Debug, PartialEq)]
pub enum RasterRead {
    /// The set holds no raster stream this reader opened (absent, unknown, or skipped).
    Absent,
    /// The stream decoded: the placements in `RasterId` order.
    Present(Vec<Placement>),
    /// The stream was opened at a compatible version but its payload is refused, and is dropped.
    Damaged(RasterError),
}

/// Decodes the raster stream of an opened set, if it holds one.
pub fn read_rasters(opened: &Opened) -> RasterRead {
    match opened.streams.iter().find(|s| s.capability == RASTER_CAPABILITY) {
        None => RasterRead::Absent,
        Some(s) => match decode_rasters(&s.payload) {
            Ok(p) => RasterRead::Present(p),
            Err(e) => RasterRead::Damaged(e),
        },
    }
}
