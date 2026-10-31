//! PMTiles passthrough for a raster pyramid (issue R2.3, pvec#222): the archive layer.
//!
//! A raster in a `.pvec` is encoded tiles (PNG, JPEG or WebP) that PVec never decodes, so moving one to a
//! PMTiles archive and back is a copy of bytes plus addressing. This layer writes and reads the archive
//! around tiles that are already addressed by `z/x/y`; the placement (the `vnd.pvec` key) is carried as
//! JSON text and means nothing here. `docs/INTERCHANGE.md` chapter 8 has the rules.
//!
//! The reader takes hostile bytes. Every region of the header is checked against the file before it is
//! read, every count against a limit before the allocation it would cause, a run length cannot multiply
//! bytes past the limits, a leaf directory cannot be visited twice, and the compressed directories and
//! metadata (gzip, which libviprs writes) are inflated with a ceiling.

use super::json::Json;
use super::{Entry, MAX_RASTER_ZOOM, assemble, id_zxy, json_str, zxy_id};
use core::fmt;
use std::collections::BTreeSet;

/// The directories of one archive: no more than the root, a leaf and a leaf of a leaf.
const MAX_DIRECTORY_DEPTH: usize = 3;
/// The most bytes one inflated directory may take.
const MAX_DIRECTORY_BYTES: usize = 16 << 20;

/// The tile types a raster archive can have, by the PMTiles tile type byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterTileType {
    /// PMTiles tile type 2, `tile_codec` 1.
    Png,
    /// PMTiles tile type 3, `tile_codec` 2.
    Jpeg,
    /// PMTiles tile type 4, `tile_codec` 3.
    Webp,
}

impl RasterTileType {
    fn byte(self) -> u8 {
        match self {
            RasterTileType::Png => 2,
            RasterTileType::Jpeg => 3,
            RasterTileType::Webp => 4,
        }
    }
}

/// One encoded tile at its PMTiles address. `bytes` are the tile exactly as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterTile {
    /// Zoom.
    pub z: u8,
    /// Column.
    pub x: u32,
    /// Row, counted down from the top.
    pub y: u32,
    /// The encoded tile. An empty tile is not stored.
    pub bytes: Vec<u8>,
}

/// What a raster archive holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RasterArchive {
    /// The codec of every tile.
    pub tile_type: RasterTileType,
    /// The stored tiles.
    pub tiles: Vec<RasterTile>,
    /// The value of the `vnd.pvec` metadata key as JSON text (an object), if there is one.
    pub pvec: Option<String>,
    /// The TileJSON `attribution` string, if there is one.
    pub attribution: Option<String>,
}

/// What the reader accepts. Every limit is checked before the work or the allocation it protects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterReadLimits {
    /// Tiles in the archive, run lengths expanded (`LIMIT-14`'s ceiling).
    pub max_tiles: u64,
    /// Bytes of one tile (`LIMIT-15`'s ceiling at the largest legal edge).
    pub max_tile_bytes: u64,
    /// Bytes of all tiles together, run lengths expanded.
    pub max_total_bytes: u64,
    /// Bytes of the metadata, inflated.
    pub max_metadata_bytes: u64,
}

impl Default for RasterReadLimits {
    fn default() -> RasterReadLimits {
        RasterReadLimits {
            max_tiles: 1_048_576,
            max_tile_bytes: 2 * 4 * 1024 * 1024 + 65_536,
            max_total_bytes: 4 << 30,
            max_metadata_bytes: 1 << 20,
        }
    }
}

/// Why an archive was not written or not read.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RasterArchiveError {
    /// Two tiles with one address.
    DuplicateTile {
        /// Zoom.
        z: u8,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
    },
    /// A tile outside the grid of its zoom.
    TileOutsideZoom {
        /// Zoom.
        z: u8,
        /// Column.
        x: u32,
        /// Row.
        y: u32,
    },
    /// A zoom PMTiles cannot address.
    ZoomTooDeep(u8),
    /// The `vnd.pvec` text is not a JSON object.
    BadPlacementKey,
    /// Not a PMTiles v3 archive.
    NotAnArchive,
    /// A region of the header lies outside the file.
    RegionOutsideFile(&'static str),
    /// The archive holds vector (or other non raster) tiles.
    NotARaster,
    /// The `Raw` (unknown) tile type: no dimensions, so no tile can be checked.
    RawTileType,
    /// A tile type that is none of PNG, JPEG, WebP, and is not vector.
    UnsupportedTileType(u8),
    /// Tiles are compressed by the container, which a raster tile never is.
    TileCompression(u8),
    /// A directory or metadata compression this reader does not read.
    InternalCompression(u8),
    /// A directory that does not parse or that names things it cannot.
    BadDirectory(&'static str),
    /// A tile that is not inside the data region.
    TileOutsideData {
        /// The tile id.
        id: u64,
    },
    /// More tiles than the limit.
    TooManyTiles {
        /// The limit.
        limit: u64,
    },
    /// A tile bigger than the limit.
    TileTooLarge {
        /// The tile id.
        id: u64,
        /// The limit.
        limit: u64,
    },
    /// More tile bytes than the limit, counting every address of a run.
    TooManyBytes {
        /// The limit.
        limit: u64,
    },
    /// Metadata that is too big or is not what the key rules allow.
    BadMetadata(&'static str),
}

impl fmt::Display for RasterArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use RasterArchiveError as E;
        match self {
            E::DuplicateTile { z, x, y } => write!(f, "tile {z}/{x}/{y} given twice"),
            E::TileOutsideZoom { z, x, y } => write!(f, "tile {z}/{x}/{y} is outside the grid of its zoom"),
            E::ZoomTooDeep(z) => write!(f, "zoom {z} is past the 31 PMTiles can address"),
            E::BadPlacementKey => write!(f, "the vnd.pvec value must be a JSON object"),
            E::NotAnArchive => write!(f, "not a PMTiles v3 archive"),
            E::RegionOutsideFile(r) => write!(f, "the {r} region lies outside the file"),
            E::NotARaster => write!(f, "the archive holds vector tiles, not a raster"),
            E::RawTileType => write!(f, "the Raw tile type has no dimensions and is refused"),
            E::UnsupportedTileType(t) => write!(f, "tile type {t} is not PNG, JPEG or WebP"),
            E::TileCompression(c) => write!(f, "tile compression {c} is not none"),
            E::InternalCompression(c) => write!(f, "internal compression {c} is neither none nor gzip"),
            E::BadDirectory(why) => write!(f, "bad directory: {why}"),
            E::TileOutsideData { id } => write!(f, "tile id {id} is not inside the data region"),
            E::TooManyTiles { limit } => write!(f, "more than {limit} tiles"),
            E::TileTooLarge { id, limit } => write!(f, "tile id {id} is over {limit} bytes"),
            E::TooManyBytes { limit } => write!(f, "more than {limit} tile bytes in all"),
            E::BadMetadata(why) => write!(f, "bad metadata: {why}"),
        }
    }
}

impl std::error::Error for RasterArchiveError {}

/// Writes one PMTiles v3 archive for one raster pyramid.
///
/// Every non-empty tile is copied unchanged; an empty tile is not addressed, so a sparse pyramid stays
/// sparse. Tiles are sorted by tile id before anything is written, so the order given does not matter and
/// the same tiles give the same bytes. The metadata is `attribution` and `vnd.pvec` when given, keys
/// sorted. Header bounds are the whole world square as in chapter 6 (the archive is not georeferenced).
pub fn write_raster_archive(archive: &RasterArchive) -> Result<Vec<u8>, RasterArchiveError> {
    let mut sorted: Vec<(u64, &RasterTile)> = Vec::with_capacity(archive.tiles.len());
    for t in &archive.tiles {
        if t.z > MAX_RASTER_ZOOM {
            return Err(RasterArchiveError::ZoomTooDeep(t.z));
        }
        if u64::from(t.x) >= 1u64 << t.z || u64::from(t.y) >= 1u64 << t.z {
            return Err(RasterArchiveError::TileOutsideZoom { z: t.z, x: t.x, y: t.y });
        }
        sorted.push((zxy_id(t.z, t.x, t.y), t));
    }
    sorted.sort_by_key(|(id, _)| *id);
    if let Some(w) = sorted.windows(2).find(|w| w[0].0 == w[1].0) {
        let t = w[0].1;
        return Err(RasterArchiveError::DuplicateTile { z: t.z, x: t.x, y: t.y });
    }
    let pvec = match &archive.pvec {
        None => None,
        Some(text) => match Json::parse(text) {
            Ok(v @ Json::Obj(_)) => Some(v),
            _ => return Err(RasterArchiveError::BadPlacementKey),
        },
    };

    let mut data = Vec::new();
    let mut entries = Vec::with_capacity(sorted.len());
    let (mut min_z, mut max_z) = (u8::MAX, 0u8);
    for (id, t) in sorted.into_iter().filter(|(_, t)| !t.bytes.is_empty()) {
        entries.push(Entry { id, offset: data.len() as u64, length: t.bytes.len() as u64, run: 1 });
        data.extend_from_slice(&t.bytes);
        min_z = min_z.min(t.z);
        max_z = max_z.max(t.z);
    }
    if entries.is_empty() {
        min_z = 0;
    }

    let mut meta = String::from("{");
    if let Some(a) = &archive.attribution {
        meta.push_str("\"attribution\":");
        json_str(&mut meta, a);
        if pvec.is_some() {
            meta.push(',');
        }
    }
    if let Some(v) = &pvec {
        meta.push_str("\"vnd.pvec\":");
        v.write(&mut meta);
    }
    meta.push('}');
    Ok(assemble(&entries, &data, &meta, archive.tile_type.byte(), (min_z, max_z)))
}

// ---- the reader -----------------------------------------------------------------------------------------

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// A region of the file, `(offset, length)`, checked to lie inside it.
fn region<'a>(file: &'a [u8], at: usize, name: &'static str) -> Result<&'a [u8], RasterArchiveError> {
    let (off, len) = (u64_at(file, at), u64_at(file, at + 8));
    let end = off.checked_add(len).ok_or(RasterArchiveError::RegionOutsideFile(name))?;
    if end > file.len() as u64 {
        return Err(RasterArchiveError::RegionOutsideFile(name));
    }
    Ok(&file[off as usize..end as usize])
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// One gzip member, inflated to at most `limit` bytes and checked against its trailer.
fn gunzip(b: &[u8], limit: usize) -> Result<Vec<u8>, &'static str> {
    if b.len() < 18 || b[0] != 0x1f || b[1] != 0x8b || b[2] != 8 {
        return Err("not a gzip member");
    }
    let flags = b[3];
    let mut i = 10usize;
    if flags & 4 != 0 {
        let n = usize::from(*b.get(i).ok_or("short gzip header")?)
            | usize::from(*b.get(i + 1).ok_or("short gzip header")?) << 8;
        i = i.checked_add(2 + n).ok_or("short gzip header")?;
    }
    for bit in [8u8, 16] {
        if flags & bit != 0 {
            let z = b
                .get(i..)
                .ok_or("short gzip header")?
                .iter()
                .position(|c| *c == 0)
                .ok_or("short gzip header")?;
            i += z + 1;
        }
    }
    if flags & 2 != 0 {
        i += 2;
    }
    if i + 8 > b.len() {
        return Err("short gzip member");
    }
    let (body, trailer) = b[i..].split_at(b.len() - i - 8);
    let out = miniz_oxide::inflate::decompress_to_vec_with_limit(body, limit)
        .map_err(|_| "bad or oversized deflate stream")?;
    let crc = u32::from_le_bytes([trailer[0], trailer[1], trailer[2], trailer[3]]);
    let size = u32::from_le_bytes([trailer[4], trailer[5], trailer[6], trailer[7]]);
    if crc != crc32(&out) || size != out.len() as u32 {
        return Err("gzip trailer does not match");
    }
    Ok(out)
}

/// A region as bytes: as is when the internal compression is none, inflated (with a ceiling) when gzip.
fn inflate(raw: &[u8], gzip: bool, limit: usize) -> Result<std::borrow::Cow<'_, [u8]>, &'static str> {
    if !gzip {
        return if raw.len() > limit { Err("larger than the limit") } else { Ok(raw.into()) };
    }
    gunzip(raw, limit).map(Into::into)
}

struct Varints<'a> {
    b: &'a [u8],
    i: usize,
}

impl Varints<'_> {
    fn next(&mut self) -> Option<u64> {
        let (mut v, mut shift) = (0u64, 0u32);
        loop {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            if shift == 63 && byte > 1 {
                return None;
            }
            v |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
}

struct Listed {
    id: u64,
    offset: u64,
    length: u64,
    run: u64,
}

fn parse_directory(dir: &[u8]) -> Result<Vec<Listed>, RasterArchiveError> {
    let bad = RasterArchiveError::BadDirectory;
    let mut v = Varints { b: dir, i: 0 };
    let n = v.next().ok_or(bad("no entry count"))?;
    // every entry takes at least one byte in each of four columns, so a count above this cannot be honest
    if n > (dir.len() / 4) as u64 {
        return Err(bad("more entries than bytes"));
    }
    let n = n as usize;
    let mut es: Vec<Listed> = Vec::with_capacity(n);
    let mut last = 0u64;
    for k in 0..n {
        let delta = v.next().ok_or(bad("short id column"))?;
        if k > 0 && delta == 0 {
            return Err(bad("tile ids are not strictly ascending"));
        }
        last = last.checked_add(delta).ok_or(bad("tile id overflow"))?;
        es.push(Listed { id: last, offset: 0, length: 0, run: 0 });
    }
    for e in es.iter_mut() {
        e.run = v.next().ok_or(bad("short run column"))?;
    }
    for e in es.iter_mut() {
        e.length = v.next().ok_or(bad("short length column"))?;
    }
    for k in 0..n {
        let o = v.next().ok_or(bad("short offset column"))?;
        es[k].offset = if o == 0 {
            if k == 0 {
                return Err(bad("the first offset cannot be implied"));
            }
            es[k - 1].offset.checked_add(es[k - 1].length).ok_or(bad("offset overflow"))?
        } else {
            o - 1
        };
    }
    if v.i != dir.len() {
        return Err(bad("bytes after the last column"));
    }
    Ok(es)
}

/// Reads a raster archive.
///
/// The tile bytes come back exactly as stored. A run length expands to one tile per address. A tile type
/// that is not PNG, JPEG or WebP, a tile compression other than none and an internal compression other
/// than none or gzip are refused by name. The reader does not look inside a tile: the header checks of
/// `RASTER-03` belong to the caller that knows the tile edge.
pub fn read_raster_archive(
    file: &[u8],
    limits: &RasterReadLimits,
) -> Result<RasterArchive, RasterArchiveError> {
    use RasterArchiveError as E;
    if file.len() < 127 || &file[0..7] != b"PMTiles" || file[7] != 3 {
        return Err(E::NotAnArchive);
    }
    let root_raw = region(file, 8, "root directory")?;
    let meta_raw = region(file, 24, "metadata")?;
    let leaf_raw = region(file, 40, "leaf directories")?;
    let data = region(file, 56, "tile data")?;
    let (internal, tile_comp, ty) = (file[97], file[98], file[99]);
    let tile_type = match ty {
        0 => return Err(E::RawTileType),
        1 => return Err(E::NotARaster),
        2 => RasterTileType::Png,
        3 => RasterTileType::Jpeg,
        4 => RasterTileType::Webp,
        t => return Err(E::UnsupportedTileType(t)),
    };
    if tile_comp != 1 {
        return Err(E::TileCompression(tile_comp));
    }
    if internal != 1 && internal != 2 {
        return Err(E::InternalCompression(internal));
    }
    let gzip = internal == 2;
    let bad_dir = |_| E::BadDirectory("bad compressed directory");

    // the tile entries of every directory, depth first, each leaf visited at most once
    let mut leaves_seen: BTreeSet<(u64, u64)> = BTreeSet::new();
    let mut tiles: Vec<Listed> = Vec::new();
    let mut stack: Vec<(Vec<Listed>, usize)> =
        vec![(parse_directory(&inflate(root_raw, gzip, MAX_DIRECTORY_BYTES).map_err(bad_dir)?)?, 1)];
    while let Some((dir, depth)) = stack.pop() {
        for e in dir {
            if e.run == 0 {
                if depth >= MAX_DIRECTORY_DEPTH {
                    return Err(E::BadDirectory("directories nested too deeply"));
                }
                if !leaves_seen.insert((e.offset, e.length)) {
                    return Err(E::BadDirectory("a leaf directory is named twice"));
                }
                let end = e.offset.checked_add(e.length).ok_or(E::BadDirectory("leaf outside its region"))?;
                if end > leaf_raw.len() as u64 {
                    return Err(E::BadDirectory("leaf outside its region"));
                }
                let raw = &leaf_raw[e.offset as usize..end as usize];
                stack.push((
                    parse_directory(&inflate(raw, gzip, MAX_DIRECTORY_BYTES).map_err(bad_dir)?)?,
                    depth + 1,
                ));
            } else {
                if tiles.len() as u64 >= limits.max_tiles {
                    return Err(E::TooManyTiles { limit: limits.max_tiles });
                }
                tiles.push(e);
            }
        }
    }

    // counted in checked arithmetic before a single tile is copied
    tiles.sort_by_key(|e| e.id);
    let (mut count, mut bytes_total) = (0u64, 0u64);
    for (k, e) in tiles.iter().enumerate() {
        let last = e.id.checked_add(e.run - 1).ok_or(E::BadDirectory("run past the id space"))?;
        if tiles.get(k + 1).is_some_and(|n| n.id <= last) {
            return Err(E::BadDirectory("runs overlap"));
        }
        count = count
            .checked_add(e.run)
            .filter(|c| *c <= limits.max_tiles)
            .ok_or(E::TooManyTiles { limit: limits.max_tiles })?;
        if e.length == 0 {
            continue;
        }
        if e.length > limits.max_tile_bytes {
            return Err(E::TileTooLarge { id: e.id, limit: limits.max_tile_bytes });
        }
        let end = e.offset.checked_add(e.length).ok_or(E::TileOutsideData { id: e.id })?;
        if end > data.len() as u64 {
            return Err(E::TileOutsideData { id: e.id });
        }
        bytes_total = e
            .run
            .checked_mul(e.length)
            .and_then(|b| bytes_total.checked_add(b))
            .filter(|b| *b <= limits.max_total_bytes)
            .ok_or(E::TooManyBytes { limit: limits.max_total_bytes })?;
    }

    let mut out = Vec::with_capacity(count as usize);
    for e in tiles.iter().filter(|e| e.length > 0) {
        let bytes = &data[e.offset as usize..(e.offset + e.length) as usize];
        for id in e.id..e.id + e.run {
            let (z, x, y) = id_zxy(id).ok_or(E::BadDirectory("tile id past zoom 31"))?;
            out.push(RasterTile { z, x, y, bytes: bytes.to_vec() });
        }
    }

    let (pvec, attribution) = read_metadata(meta_raw, gzip, limits.max_metadata_bytes)?;
    Ok(RasterArchive { tile_type, tiles: out, pvec, attribution })
}

fn read_metadata(
    raw: &[u8],
    gzip: bool,
    limit: u64,
) -> Result<(Option<String>, Option<String>), RasterArchiveError> {
    use RasterArchiveError::BadMetadata as bad;
    if raw.is_empty() {
        return Ok((None, None));
    }
    let limit = usize::try_from(limit).unwrap_or(usize::MAX);
    let bytes = inflate(raw, gzip, limit).map_err(bad)?;
    let text = core::str::from_utf8(&bytes).map_err(|_| bad("not UTF-8"))?;
    let Json::Obj(mut m) = Json::parse(text).map_err(bad)? else {
        return Err(bad("not an object"));
    };
    let attribution = match m.remove("attribution") {
        None => None,
        Some(Json::Str(s)) => Some(s),
        Some(_) => return Err(bad("attribution is not a string")),
    };
    let pvec = match m.remove("vnd.pvec") {
        None => None,
        Some(v @ Json::Obj(_)) => {
            let mut s = String::new();
            v.write(&mut s);
            Some(s)
        }
        Some(_) => return Err(bad("vnd.pvec is not an object")),
    };
    Ok((pvec, attribution))
}
