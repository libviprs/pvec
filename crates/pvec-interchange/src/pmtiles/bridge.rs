//! A raster of a `.pvec` to a PMTiles archive and back (issue R2.3, pvec#222): the container layer of the
//! passthrough. [`export_raster`] reads the tiles through the container reader (so every tile has passed its
//! checksum and the header sniffers) and hands them to the archive writer; [`import_raster`] reads an archive,
//! holds the placement to the rules of a stream row and every tile to the sniffers, and returns what the
//! container writer takes. `docs/INTERCHANGE.md` chapter 8 has the rules.

use super::json::Json;
use super::{
    RasterArchive, RasterArchiveError, RasterReadLimits, RasterTile, RasterTileType, read_raster_archive,
    write_raster_archive,
};
use core::fmt;
use pvec_components::{Band, PixelFormat, Placement, RasterError, TileCodec, encode_rasters};
use pvec_container::blocks::raster::{TileIn, grid_of};
use pvec_container::blocks::sniff::{TileRefusal, sniff_tile};
use pvec_container::limits::Limits;
use pvec_container::reader::{ContainerReader, ReadError};
use pvec_container::source::BlockSource;
use std::collections::BTreeMap;

fn tile_type_of(codec: TileCodec) -> Option<RasterTileType> {
    match codec {
        TileCodec::Png => Some(RasterTileType::Png),
        TileCodec::Jpeg => Some(RasterTileType::Jpeg),
        TileCodec::Webp => Some(RasterTileType::Webp),
        _ => None,
    }
}

/// Why a raster could not be exported. [`ExportError::rule`] is the rule that refuses it.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExportError {
    /// The raster, its index or one of its tiles was refused by the reader (a checksum, a limit, the sniffers).
    Read(ReadError),
    /// The raster's tile codec is one this build does not know, so there is no PMTiles tile type for it.
    UnknownCodec(u8),
    /// The archive writer refused the tiles.
    Archive(RasterArchiveError),
}

impl ExportError {
    /// The rule that refuses it: the reader's own for a raster or a tile, `RASTER-04` for a codec with no tile
    /// type, `PMTILES` for the archive.
    pub fn rule(&self) -> String {
        match self {
            ExportError::Read(e) => e.rule(),
            ExportError::UnknownCodec(_) => "RASTER-04".into(),
            ExportError::Archive(_) => "PMTILES".into(),
        }
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::Read(e) => write!(f, "{}: {e}", e.rule()),
            ExportError::UnknownCodec(id) => write!(f, "RASTER-04: tile codec {id} has no PMTiles tile type"),
            ExportError::Archive(e) => write!(f, "PMTILES: {e}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Writes one PMTiles v3 archive for raster `raster` of the artifact: the tiles as the container stores them,
/// `z` the level, `x` the column and `y` the row, a tile that is not stored left out, the placement under
/// `vnd.pvec` and the attribution as the TileJSON string. The archive is not georeferenced.
pub fn export_raster<S: BlockSource>(
    reader: &ContainerReader<S>,
    raster: u32,
) -> Result<Vec<u8>, ExportError> {
    let placement = reader.raster_placement(raster).map_err(ExportError::Read)?;
    let tile_type =
        tile_type_of(placement.tile_codec).ok_or(ExportError::UnknownCodec(placement.tile_codec.word()))?;
    let index = reader.raster_index(raster).map_err(ExportError::Read)?;
    let mut tiles = Vec::with_capacity(index.stored() as usize);
    for level in 0..index.levels().len() as u32 {
        for t in index.level_tiles(level).into_iter().flatten().filter(|t| t.is_stored()) {
            let bytes = reader.read_raster_tile(raster, level, t.col, t.row).map_err(ExportError::Read)?;
            if let Some(bytes) = bytes {
                tiles.push(RasterTile {
                    z: level as u8,
                    x: t.col,
                    y: t.row,
                    bytes: bytes.as_slice().to_vec(),
                });
            }
        }
    }
    let attribution = (!placement.attribution.is_empty()).then(|| placement.attribution.clone());
    write_raster_archive(&RasterArchive {
        tile_type,
        tiles,
        pvec: Some(placement_json(&placement)),
        attribution,
    })
    .map_err(ExportError::Archive)
}

/// What an import gives the container writer: the placement for the raster stream and the tiles for
/// `ContainerWriter::push_raster_tiles`, their bytes as they were in the archive.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedRaster {
    /// The placement, validated as a stream row.
    pub placement: Placement,
    /// Every tile the archive stores. A tile it does not have is not stored in the pyramid either.
    pub tiles: Vec<TileIn>,
}

/// Why an archive was not imported. [`ImportError::rule`] is the rule that refuses it, and the text starts with it,
/// so a report line made from the error names the rule.
#[derive(Debug)]
#[non_exhaustive]
pub enum ImportError {
    /// The archive itself was refused.
    Archive(RasterArchiveError),
    /// The `vnd.pvec` text is not a placement.
    PlacementText(&'static str),
    /// The placement breaks a rule of the stream row.
    Placement(RasterError),
    /// Neither the archive nor the caller has a placement, and pixels without a map to source space cannot be placed.
    NoPlacement,
    /// The caller's placement and the archive's are not the same, bit for bit.
    PlacementDisagrees,
    /// The archive's tile type is not the placement's tile codec.
    CodecDisagrees,
    /// A tile at an address the pyramid of the placement does not have.
    TileOutsidePyramid {
        /// Zoom.
        z: u8,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
    },
    /// A tile that fails the header checks of its codec (`RASTER-03`) or the size ceiling (`LIMIT-15`).
    Tile {
        /// Zoom.
        z: u8,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
        /// Why.
        refusal: TileRefusal,
    },
}

impl ImportError {
    /// The rule that refuses it: the stream row's for a placement, `RASTER-01` for a tile outside the pyramid,
    /// `RASTER-04` for a codec that is not the placement's, the sniffers' for a tile, and `PMTILES` for the rest.
    pub fn rule(&self) -> &'static str {
        match self {
            ImportError::Placement(e) => e.rule(),
            ImportError::TileOutsidePyramid { .. } => "RASTER-01",
            ImportError::CodecDisagrees => "RASTER-04",
            ImportError::Tile { refusal, .. } => refusal.rule(),
            _ => "PMTILES",
        }
    }
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            ImportError::Archive(e) => write!(f, "{rule}: {e}"),
            ImportError::PlacementText(why) => {
                write!(f, "{rule}: the vnd.pvec placement is not usable: {why}")
            }
            ImportError::Placement(e) => write!(f, "{e}"),
            ImportError::NoPlacement => {
                write!(f, "{rule}: the archive has no vnd.pvec placement and none was given")
            }
            ImportError::PlacementDisagrees => {
                write!(f, "{rule}: the placement given is not the one in the archive")
            }
            ImportError::CodecDisagrees => {
                write!(f, "{rule}: the tile type is not the placement's tile codec")
            }
            ImportError::TileOutsidePyramid { z, x, y } => {
                write!(f, "{rule}: tile {z}/{x}/{y} is outside the pyramid of the placement")
            }
            ImportError::Tile { z, x, y, refusal } => write!(f, "tile {z}/{x}/{y}: {refusal}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Reads `archive` as a raster for a `.pvec`. The placement is the archive's `vnd.pvec` one; when `given` is also
/// passed it must be the same bit for bit; with neither the import is refused. The placement is held to the rules of
/// a stream row, the tile type to its codec, every tile to the address range of the pyramid and to the header
/// sniffers (which, with the placement's legal edge, also settle the tile edge: a tile that says another edge is
/// refused), and nothing is returned if one tile fails.
pub fn import_raster(
    archive: &[u8],
    given: Option<&Placement>,
    limits: &RasterReadLimits,
) -> Result<ImportedRaster, ImportError> {
    let archive = read_raster_archive(archive, limits).map_err(ImportError::Archive)?;
    let from_archive = archive.pvec.as_deref().map(placement_from_json).transpose()?;
    let placement = match (from_archive, given) {
        (Some(a), Some(g)) => {
            let bits =
                |p: &Placement| encode_rasters(std::slice::from_ref(p)).map_err(ImportError::Placement);
            if bits(&a)? != bits(g)? {
                return Err(ImportError::PlacementDisagrees);
            }
            a
        }
        (Some(p), None) => p,
        (None, Some(g)) => g.clone(),
        (None, None) => return Err(ImportError::NoPlacement),
    };
    encode_rasters(std::slice::from_ref(&placement)).map_err(ImportError::Placement)?;
    if tile_type_of(placement.tile_codec) != Some(archive.tile_type) {
        return Err(ImportError::CodecDisagrees);
    }
    let mut tiles = Vec::with_capacity(archive.tiles.len());
    for t in archive.tiles {
        let (cols, rows) = if t.z < placement.levels {
            grid_of(
                placement.width,
                placement.height,
                u32::from(placement.tile_edge),
                u32::from(placement.levels),
                u32::from(t.z),
            )
        } else {
            (0, 0)
        };
        if t.x >= cols || t.y >= rows {
            return Err(ImportError::TileOutsidePyramid { z: t.z, x: t.x, y: t.y });
        }
        sniff_tile(&placement, Limits::DEFAULT.max_tile_bytes, &t.bytes)
            .map_err(|refusal| ImportError::Tile { z: t.z, x: t.x, y: t.y, refusal })?;
        tiles.push(TileIn { level: u32::from(t.z), col: t.x, row: t.y, bytes: t.bytes });
    }
    Ok(ImportedRaster { placement, tiles })
}

// ---- the vnd.pvec text ------------------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The `vnd.pvec` metadata value of a placement: `{"raster": {...}}`, a member per field of the placement table of
/// `docs/RASTER.md`. The six coefficients are the 16 lowercase hex digits of the `f64` bits, so no JSON number
/// rounds them; keys are sorted and there is no whitespace, so the same placement gives the same text.
pub fn placement_json(p: &Placement) -> String {
    let num = |n: u64| Json::Num(n.to_string());
    let mut m = BTreeMap::new();
    m.insert(
        "pixel_to_source".to_string(),
        Json::Arr(p.pixel_to_source.iter().map(|v| Json::Str(format!("{:016x}", v.to_bits()))).collect()),
    );
    m.insert("width".into(), num(p.width.into()));
    m.insert("height".into(), num(p.height.into()));
    m.insert("visible_box".into(), Json::Arr(p.visible_box.iter().map(|v| num((*v).into())).collect()));
    m.insert("tile_edge".into(), num(p.tile_edge.into()));
    m.insert("levels".into(), num(p.levels.into()));
    m.insert("tile_codec".into(), num(p.tile_codec.word().into()));
    m.insert("pixel_format".into(), num(p.pixel_format.word().into()));
    m.insert("bit_depth".into(), num(p.bit_depth.into()));
    m.insert("opacity".into(), num(p.opacity.into()));
    m.insert("band".into(), num(p.band.word().into()));
    if let Some(l) = p.layer_index {
        m.insert("layer_index".into(), num(l.into()));
    }
    if let Some(d) = &p.source_digest {
        m.insert("source_digest".into(), Json::Str(hex(d)));
    }
    m.insert("attribution".into(), Json::Str(p.attribution.clone()));
    let mut o = BTreeMap::new();
    o.insert("raster".to_string(), Json::Obj(m));
    let mut out = String::new();
    Json::Obj(o).write(&mut out);
    out
}

/// The placement of a `vnd.pvec` value, the inverse of [`placement_json`]. It maps text to the type and judges
/// nothing but the shape; the rules of a stream row are [`import_raster`]'s. A member it does not know is ignored.
pub fn placement_from_json(text: &str) -> Result<Placement, ImportError> {
    let bad = ImportError::PlacementText;
    let Json::Obj(top) = Json::parse(text).map_err(bad)? else { return Err(bad("not an object")) };
    let Some(Json::Obj(m)) = top.get("raster") else { return Err(bad("no raster object")) };
    let uint = |k: &'static str| -> Result<u64, ImportError> {
        match m.get(k) {
            Some(Json::Num(n)) => n.parse::<u64>().map_err(|_| bad("a member is not an unsigned integer")),
            _ => Err(bad("a numeric member is missing")),
        }
    };
    let small = |k: &'static str, max: u64| -> Result<u64, ImportError> {
        uint(k).and_then(|v| if v <= max { Ok(v) } else { Err(bad("a member is out of range")) })
    };
    let hexes = |s: &str, n: usize| -> Option<Vec<u8>> {
        (s.len() == n * 2 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
            .then(|| (0..n).map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect())
    };
    let mut pixel_to_source = [0.0f64; 6];
    match m.get("pixel_to_source") {
        Some(Json::Arr(a)) if a.len() == 6 => {
            for (slot, v) in pixel_to_source.iter_mut().zip(a) {
                let Json::Str(s) = v else { return Err(bad("a coefficient is not a hex string")) };
                let b = hexes(s, 8).ok_or(bad("a coefficient is not 16 hex digits"))?;
                *slot = f64::from_bits(u64::from_be_bytes(b.try_into().unwrap_or([0; 8])));
            }
        }
        _ => return Err(bad("pixel_to_source is not six values")),
    }
    let mut visible_box = [0u32; 4];
    match m.get("visible_box") {
        Some(Json::Arr(a)) if a.len() == 4 => {
            for (slot, v) in visible_box.iter_mut().zip(a) {
                let Json::Num(n) = v else { return Err(bad("visible_box holds a non-number")) };
                *slot = n.parse::<u32>().map_err(|_| bad("visible_box holds a bad number"))?;
            }
        }
        _ => return Err(bad("visible_box is not four values")),
    }
    let layer_index = match m.get("layer_index") {
        None => None,
        Some(_) => Some(u32::try_from(uint("layer_index")?).map_err(|_| bad("layer_index is out of range"))?),
    };
    let source_digest = match m.get("source_digest") {
        None => None,
        Some(Json::Str(s)) => Some(
            hexes(s, 32).and_then(|b| b.try_into().ok()).ok_or(bad("source_digest is not 64 hex digits"))?,
        ),
        Some(_) => return Err(bad("source_digest is not a string")),
    };
    let attribution = match m.get("attribution") {
        Some(Json::Str(s)) => s.clone(),
        None => String::new(),
        Some(_) => return Err(bad("attribution is not a string")),
    };
    Ok(Placement {
        pixel_to_source,
        width: small("width", u32::MAX.into())? as u32,
        height: small("height", u32::MAX.into())? as u32,
        visible_box,
        tile_edge: small("tile_edge", u16::MAX.into())? as u16,
        levels: small("levels", u8::MAX.into())? as u8,
        tile_codec: TileCodec::from_word(small("tile_codec", u8::MAX.into())? as u8),
        pixel_format: PixelFormat::from_word(small("pixel_format", u8::MAX.into())? as u8)
            .ok_or(bad("pixel_format is not 0 to 3"))?,
        bit_depth: small("bit_depth", u8::MAX.into())? as u8,
        opacity: small("opacity", u8::MAX.into())? as u8,
        band: Band::from_word(small("band", u8::MAX.into())? as u8).ok_or(bad("band is not 0 or 1"))?,
        layer_index,
        source_digest,
        attribution,
    })
}
