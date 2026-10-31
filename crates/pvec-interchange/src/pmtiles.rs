//! PMTiles v3 packaging of the display-only MVT tiles (issue P6.7).
//!
//! One archive per exported view: the tiles of [`crate::mvt`] laid out as a single file a client can
//! stream with HTTP range requests. The archive stays what the tiles are, a derivative for drawing.
//! It is display fidelity only, it is not an input to any conformance comparison, and it is not re-imported as
//! geometry, which is why the vector half of this module can write an archive and cannot read one.
//! `docs/INTERCHANGE.md` chapter 6 has the rule and the layout.
//!
//! The raster half ([`write_raster_archive`], [`read_raster_archive`]) is the opposite: a raster pyramid is
//! encoded tiles that PVec never decodes, so it goes out and comes back byte for byte, chapter 8.
//!
//! The layout is the one the specification asks for: a 127 byte header, then the root directory (so
//! both fall inside the first [`FIRST_READ_BYTES`] a client fetches), then the JSON metadata, then
//! the leaf directories (only when the root alone would not fit), then the tile data in tile id
//! order. Everything is stored uncompressed, so the writer has no codec and no dependency.
//!
//! Deterministic: tiles are sorted by tile id before anything is written, directories are built from
//! that order alone, and the metadata keys are written sorted. The writer never reads a clock; the
//! only date in an archive is the string the caller passes in [`Provenance::created`].

mod json;
mod raster;
pub use raster::*;

use crate::fidelity::FidelityKind;
use crate::mvt::{MvtTile, TileId};
use crate::report::{Report, ReportError};
use core::fmt;
use std::collections::BTreeMap;

/// Bytes a client reads first: the header and the root directory have to fit in this.
pub const FIRST_READ_BYTES: usize = 16_384;

const HEADER_LEN: usize = 127;
/// The deepest zoom PMTiles can address.
const MAX_RASTER_ZOOM: u8 = 31;
/// The smallest leaf directory the writer tries when the root does not fit.
const FIRST_LEAF_SIZE: usize = 1024;

/// What the archive must state about what it approximated. Every field but `created` is required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Provenance {
    /// The digest of the source `.pvec` artifact (P4.6).
    pub source_digest: Option<[u8; 32]>,
    /// The screen tolerance the tiles were tessellated to, in pixels.
    pub tessellation_tolerance_px: Option<f64>,
    /// The zoom range the export was tested over, both ends included.
    pub tested_zooms: Option<(u8, u8)>,
    /// A creation stamp the caller chose. The writer never reads a clock, so without this there is
    /// no date anywhere in the archive.
    pub created: Option<String>,
}

/// Packaging options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PmtilesOptions {
    /// The archive name written into the metadata, if any.
    pub name: Option<String>,
}

/// Why an archive was not written.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PmtilesError {
    /// A provenance field that is required was not given.
    MissingProvenance(&'static str),
    /// A provenance value that makes no sense.
    BadProvenance(&'static str),
    /// Two tiles with the same address.
    DuplicateTile(TileId),
    /// A tile outside the tested zoom range, which would claim a zoom nobody checked.
    TileOutsideTestedRange(TileId),
    /// A tile whose bytes are not a vector tile this export wrote.
    BadTileBytes(TileId),
    /// The fidelity report of the tiles was refused.
    Report(ReportError),
}

impl fmt::Display for PmtilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = |t: &TileId| format!("{}/{}/{}", t.z(), t.x(), t.y());
        match self {
            PmtilesError::MissingProvenance(n) => write!(f, "provenance field `{n}` is required"),
            PmtilesError::BadProvenance(n) => write!(f, "provenance field `{n}` has an invalid value"),
            PmtilesError::DuplicateTile(t) => write!(f, "tile {} given twice", at(t)),
            PmtilesError::TileOutsideTestedRange(t) => {
                write!(f, "tile {} is outside the tested zoom range", at(t))
            }
            PmtilesError::BadTileBytes(t) => write!(f, "tile {} is not a vector tile", at(t)),
            PmtilesError::Report(e) => write!(f, "no fidelity report: {e}"),
        }
    }
}

impl std::error::Error for PmtilesError {}

/// The PMTiles tile id of an address: the position along a Hilbert curve within the zoom, after all
/// the tiles of the smaller zooms.
pub fn tile_id(tile: TileId) -> u64 {
    zxy_id(tile.z(), tile.x(), tile.y())
}

/// [`tile_id`] for any address PMTiles can hold (zoom up to 31), which is wider than the vector export's.
fn zxy_id(z: u8, x: u32, y: u32) -> u64 {
    let z = u32::from(z);
    let before = ((1u64 << (2 * z)) - 1) / 3;
    let n = 1u64 << z;
    let (mut x, mut y, mut d) = (u64::from(x), u64::from(y), 0u64);
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(x & s > 0);
        let ry = u64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            core::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    before + d
}

/// The address of a tile id, the inverse of [`zxy_id`], or `None` past zoom 31.
fn id_zxy(id: u64) -> Option<(u8, u32, u32)> {
    let mut before = 0u64;
    for z in 0..=MAX_RASTER_ZOOM {
        let count = 1u64 << (2 * u32::from(z));
        if id >= before && id - before < count {
            let (mut t, n) = (id - before, 1u64 << z);
            let (mut x, mut y, mut s) = (0u64, 0u64, 1u64);
            while s < n {
                let rx = 1 & (t / 2);
                let ry = 1 & (t ^ rx);
                if ry == 0 {
                    if rx == 1 {
                        x = s - 1 - x;
                        y = s - 1 - y;
                    }
                    core::mem::swap(&mut x, &mut y);
                }
                x += s * rx;
                y += s * ry;
                t /= 4;
                s *= 2;
            }
            return Some((z, u32::try_from(x).ok()?, u32::try_from(y).ok()?));
        }
        before += count;
    }
    None
}

// ---- reading the tiles' own layers -------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldType {
    Number,
    Boolean,
    String,
}

impl FieldType {
    fn name(self) -> &'static str {
        match self {
            FieldType::Number => "Number",
            FieldType::Boolean => "Boolean",
            FieldType::String => "String",
        }
    }
}

struct LayerSummary {
    fields: BTreeMap<String, FieldType>,
    min_zoom: u8,
    max_zoom: u8,
}

struct Pb<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Pb<'a> {
    fn done(&self) -> bool {
        self.i >= self.b.len()
    }
    fn varint(&mut self) -> Option<u64> {
        let (mut v, mut shift) = (0u64, 0u32);
        loop {
            let byte = *self.b.get(self.i)?;
            self.i += 1;
            v |= u64::from(byte & 0x7f).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    }
    /// The next field as (number, wire type, payload); a varint's payload is empty and its value is
    /// returned in the last slot.
    fn field(&mut self) -> Option<(u64, u64, &'a [u8], u64)> {
        let key = self.varint()?;
        let (num, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                let v = self.varint()?;
                Some((num, wire, &[], v))
            }
            1 => {
                let s = self.b.get(self.i..self.i.checked_add(8)?)?;
                self.i += 8;
                Some((num, wire, s, 0))
            }
            2 => {
                let n = usize::try_from(self.varint()?).ok()?;
                let s = self.b.get(self.i..self.i.checked_add(n)?)?;
                self.i += n;
                Some((num, wire, s, 0))
            }
            5 => {
                let s = self.b.get(self.i..self.i.checked_add(4)?)?;
                self.i += 4;
                Some((num, wire, s, 0))
            }
            _ => None,
        }
    }
}

/// The layers a tile holds, with the type of every attribute that some feature really uses.
fn scan_tile(bytes: &[u8]) -> Option<BTreeMap<String, BTreeMap<String, FieldType>>> {
    let mut out: BTreeMap<String, BTreeMap<String, FieldType>> = BTreeMap::new();
    let mut tile = Pb { b: bytes, i: 0 };
    while !tile.done() {
        let (num, wire, layer, _) = tile.field()?;
        if (num, wire) != (3, 2) {
            return None;
        }
        let mut name: Option<&str> = None;
        let mut keys: Vec<&str> = Vec::new();
        let mut values: Vec<FieldType> = Vec::new();
        let mut tags: Vec<(usize, usize)> = Vec::new();
        let mut l = Pb { b: layer, i: 0 };
        while !l.done() {
            let (num, wire, body, _) = l.field()?;
            match (num, wire) {
                (1, 2) => name = Some(core::str::from_utf8(body).ok()?),
                (3, 2) => keys.push(core::str::from_utf8(body).ok()?),
                (4, 2) => {
                    let mut v = Pb { b: body, i: 0 };
                    let (vn, _, _, _) = v.field()?;
                    values.push(match vn {
                        1 => FieldType::String,
                        2..=6 => FieldType::Number,
                        7 => FieldType::Boolean,
                        _ => return None,
                    });
                }
                (2, 2) => {
                    let mut f = Pb { b: body, i: 0 };
                    while !f.done() {
                        let (fnum, fwire, packed, _) = f.field()?;
                        if (fnum, fwire) == (2, 2) {
                            let mut t = Pb { b: packed, i: 0 };
                            while !t.done() {
                                let k = usize::try_from(t.varint()?).ok()?;
                                let v = usize::try_from(t.varint()?).ok()?;
                                tags.push((k, v));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let name = name.filter(|n| !n.is_empty())?;
        let entry = out.entry(name.to_string()).or_default();
        for (k, v) in tags {
            let (key, ty) = (keys.get(k)?, *values.get(v)?);
            match entry.get(*key) {
                Some(old) if *old != ty => return None,
                _ => {
                    entry.insert((*key).to_string(), ty);
                }
            }
        }
    }
    Some(out)
}

// ---- encoding helpers ---------------------------------------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[derive(Clone, Copy)]
struct Entry {
    id: u64,
    offset: u64,
    length: u64,
    run: u64,
}

/// A PMTiles directory: counts, then column by column (id deltas, run lengths, lengths, offsets).
fn encode_directory(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, entries.len() as u64);
    let mut last = 0;
    for e in entries {
        put_varint(&mut out, e.id - last);
        last = e.id;
    }
    for e in entries {
        put_varint(&mut out, e.run);
    }
    for e in entries {
        put_varint(&mut out, e.length);
    }
    for (i, e) in entries.iter().enumerate() {
        let contiguous = i > 0 && e.offset == entries[i - 1].offset + entries[i - 1].length;
        put_varint(&mut out, if contiguous { 0 } else { e.offset + 1 });
    }
    out
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

type Checked = ([u8; 32], f64, (u8, u8));

fn check_provenance(p: &Provenance) -> Result<Checked, PmtilesError> {
    let digest = p.source_digest.ok_or(PmtilesError::MissingProvenance("source_digest"))?;
    let tol =
        p.tessellation_tolerance_px.ok_or(PmtilesError::MissingProvenance("tessellation_tolerance_px"))?;
    let zooms = p.tested_zooms.ok_or(PmtilesError::MissingProvenance("tested_zooms"))?;
    if !(tol.is_finite() && tol > 0.0) {
        return Err(PmtilesError::BadProvenance("tessellation_tolerance_px"));
    }
    if zooms.0 > zooms.1 {
        return Err(PmtilesError::BadProvenance("tested_zooms"));
    }
    Ok((digest, tol, zooms))
}

/// Packages tiles as one PMTiles v3 archive.
///
/// Empty tiles are not addressed (a client that asks for one gets "no such tile", which draws the
/// same as nothing). The metadata declares exactly the layers the tiles hold, and states the
/// provenance. Deterministic: see the module docs.
pub fn write_pmtiles(
    tiles: &[MvtTile],
    provenance: &Provenance,
    options: &PmtilesOptions,
) -> Result<Vec<u8>, PmtilesError> {
    let (digest, tolerance, zooms) = check_provenance(provenance)?;

    let mut sorted: Vec<&MvtTile> = tiles.iter().collect();
    sorted.sort_by_key(|t| tile_id(t.tile));
    for w in sorted.windows(2) {
        if w[0].tile == w[1].tile {
            return Err(PmtilesError::DuplicateTile(w[0].tile));
        }
    }
    if let Some(t) = sorted.iter().find(|t| t.tile.z() < zooms.0 || t.tile.z() > zooms.1) {
        return Err(PmtilesError::TileOutsideTestedRange(t.tile));
    }
    let sorted: Vec<&MvtTile> = sorted.into_iter().filter(|t| !t.bytes.is_empty()).collect();

    let mut layers: BTreeMap<String, LayerSummary> = BTreeMap::new();
    let mut ledger_total = 0usize;
    let mut per_kind = [0usize; 5];
    let mut max_bound = 0.0f64;
    for t in tiles {
        for r in t.ledger.ledger().records() {
            ledger_total += 1;
            if let Some(i) = FidelityKind::ALL.iter().position(|k| *k == r.fidelity().kind()) {
                per_kind[i] += 1;
            }
            if let Some(b) = r.error_bound() {
                max_bound = max_bound.max(b.get());
            }
        }
    }
    for t in &sorted {
        let found = scan_tile(&t.bytes).ok_or(PmtilesError::BadTileBytes(t.tile))?;
        for (name, fields) in found {
            let l = layers.entry(name).or_insert_with(|| LayerSummary {
                fields: BTreeMap::new(),
                min_zoom: t.tile.z(),
                max_zoom: t.tile.z(),
            });
            l.min_zoom = l.min_zoom.min(t.tile.z());
            l.max_zoom = l.max_zoom.max(t.tile.z());
            for (k, ty) in fields {
                match l.fields.get(&k) {
                    Some(old) if *old != ty => return Err(PmtilesError::BadTileBytes(t.tile)),
                    _ => {
                        l.fields.insert(k, ty);
                    }
                }
            }
        }
    }

    // tile data and the entries that address it
    let mut data = Vec::new();
    let mut entries = Vec::with_capacity(sorted.len());
    for t in &sorted {
        entries.push(Entry {
            id: tile_id(t.tile),
            offset: data.len() as u64,
            length: t.bytes.len() as u64,
            run: 1,
        });
        data.extend_from_slice(&t.bytes);
    }

    let metadata = metadata_json(
        options,
        provenance,
        digest,
        tolerance,
        zooms,
        &layers,
        ledger_total,
        per_kind,
        max_bound,
    );

    let (min_z, max_z) = match (sorted.first(), sorted.iter().map(|t| t.tile.z()).max()) {
        (Some(_), Some(hi)) => (sorted.iter().map(|t| t.tile.z()).min().unwrap_or(0), hi),
        _ => (0, 0),
    };
    Ok(assemble(&entries, &data, &metadata, 1, (min_z, max_z)))
}

/// The archive around already built entries and tile data: the directories (everything in the root if
/// it fits the first read, else leaves of a growing size), the metadata and the 127 byte header, laid out
/// as chapter 6 of `docs/INTERCHANGE.md` says. `tile_type` is the PMTiles tile type byte. Tiles and
/// directories are stored uncompressed.
fn assemble(
    entries: &[Entry],
    data: &[u8],
    metadata: &str,
    tile_type: u8,
    (min_z, max_z): (u8, u8),
) -> Vec<u8> {
    // directories: everything in the root if it fits, else leaves of a growing size
    let room = (FIRST_READ_BYTES - HEADER_LEN) as u64;
    let whole = encode_directory(entries);
    let (root, leaves) = if whole.len() as u64 <= room {
        (whole, Vec::new())
    } else {
        let mut size = FIRST_LEAF_SIZE;
        loop {
            let mut leaves = Vec::new();
            let mut pointers = Vec::new();
            for chunk in entries.chunks(size) {
                let dir = encode_directory(chunk);
                pointers.push(Entry {
                    id: chunk[0].id,
                    offset: leaves.len() as u64,
                    length: dir.len() as u64,
                    run: 0,
                });
                leaves.extend_from_slice(&dir);
            }
            let root = encode_directory(&pointers);
            if root.len() as u64 <= room || size >= entries.len() {
                break (root, leaves);
            }
            size *= 2;
        }
    };

    let root_off = HEADER_LEN as u64;
    let meta_off = root_off + root.len() as u64;
    let leaf_off = meta_off + metadata.len() as u64;
    let data_off = leaf_off + leaves.len() as u64;

    let mut out = Vec::with_capacity(data_off as usize + data.len());
    out.extend_from_slice(b"PMTiles");
    out.push(3);
    for v in [
        root_off,
        root.len() as u64,
        meta_off,
        metadata.len() as u64,
        leaf_off,
        leaves.len() as u64,
        data_off,
        data.len() as u64,
        entries.len() as u64, // addressed tiles
        entries.len() as u64, // directory entries
        entries.len() as u64, // distinct tile contents
    ] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&[1, 1, 1, tile_type]); // clustered, internal and tile compression none
    out.push(min_z);
    out.push(max_z);
    // The drawing's map is a plain square, not a place on Earth; the archive says it covers the
    // whole web mercator world square, which is what a tile client assumes.
    for v in [-1_800_000_000i32, -850_511_288, 1_800_000_000, 850_511_288] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.push(min_z);
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(&root);
    out.extend_from_slice(metadata.as_bytes());
    out.extend_from_slice(&leaves);
    out.extend_from_slice(data);
    out
}

#[allow(clippy::too_many_arguments)]
fn metadata_json(
    options: &PmtilesOptions,
    provenance: &Provenance,
    digest: [u8; 32],
    tolerance: f64,
    zooms: (u8, u8),
    layers: &BTreeMap<String, LayerSummary>,
    records: usize,
    per_kind: [usize; 5],
    max_bound: f64,
) -> String {
    let mut j = String::from("{");
    if let Some(name) = &options.name {
        j.push_str("\"name\":");
        json_str(&mut j, name);
        j.push(',');
    }
    j.push_str("\"pvec\":{\"canonical\":false,");
    if let Some(c) = &provenance.created {
        j.push_str("\"created\":");
        json_str(&mut j, c);
        j.push(',');
    }
    j.push_str("\"display_only\":true,\"fidelity\":{");
    let mut parts: Vec<(String, String)> =
        FidelityKind::ALL.iter().zip(per_kind).map(|(k, n)| (k.name().to_string(), n.to_string())).collect();
    parts.push(("max_error_bound".into(), format!("{max_bound}")));
    parts.push(("records".into(), records.to_string()));
    parts.sort();
    j.push_str(&parts.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect::<Vec<_>>().join(","));
    j.push_str("},\"source_digest\":\"sha256:");
    j.push_str(&hex(&digest));
    j.push_str(&format!("\",\"tessellation_tolerance_px\":{tolerance},"));
    j.push_str(&format!("\"tested_zoom_max\":{},\"tested_zoom_min\":{}}},", zooms.1, zooms.0));
    j.push_str("\"vector_layers\":[");
    let mut first = true;
    for (name, l) in layers {
        if !first {
            j.push(',');
        }
        first = false;
        j.push_str("{\"fields\":{");
        j.push_str(
            &l.fields
                .iter()
                .map(|(k, t)| {
                    let mut s = String::new();
                    json_str(&mut s, k);
                    format!("{s}:\"{}\"", t.name())
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        j.push_str("},\"id\":");
        json_str(&mut j, name);
        j.push_str(&format!(",\"maxzoom\":{},\"minzoom\":{}}}", l.max_zoom, l.min_zoom));
    }
    j.push_str("]}");
    j
}

/// The fidelity report of a PMTiles archive's tiles (P6.8), with the digest the archive's provenance
/// states. The counts follow [`crate::mvt::report`]. A tile set that [`write_pmtiles`] would refuse for
/// its provenance has no report either.
pub fn report(
    tiles: &[MvtTile],
    provenance: &Provenance,
    artifact_entities: u32,
) -> Result<Report, PmtilesError> {
    let (digest, _, _) = check_provenance(provenance)?;
    Report::new(
        "PMTiles",
        "3",
        digest,
        artifact_entities,
        tiles.iter().flat_map(|t| t.ledger.ledger().records()),
    )
    .map_err(PmtilesError::Report)
}
