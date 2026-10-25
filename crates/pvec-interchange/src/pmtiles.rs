//! PMTiles v3 packaging of the display-only MVT tiles (issue P6.7).
//!
//! STUB: the red tests of pvec#24 land first; the writer follows.

use crate::mvt::{MvtTile, TileId};
use core::fmt;

/// Bytes a client reads first: the header and the root directory have to fit in this.
pub const FIRST_READ_BYTES: usize = 16_384;

/// What the archive must state about what it approximated. Every field is required.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Provenance {
    /// The digest of the source `.pvec` artifact (P4.6).
    pub source_digest: Option<[u8; 32]>,
    /// The screen tolerance the tiles were tessellated to, in pixels.
    pub tessellation_tolerance_px: Option<f64>,
    /// The zoom range the export was tested over, both ends included.
    pub tested_zooms: Option<(u8, u8)>,
    /// A creation stamp the caller chose. The writer never reads a clock.
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
pub enum PmtilesError {
    /// Not written yet.
    NotImplemented,
    /// A provenance field that is required was not given.
    MissingProvenance(&'static str),
    /// A provenance value that makes no sense.
    BadProvenance(&'static str),
    /// Two tiles with the same address.
    DuplicateTile(TileId),
    /// A tile outside the tested zoom range.
    TileOutsideTestedRange(TileId),
    /// A tile whose bytes are not a vector tile this export wrote.
    BadTileBytes(TileId),
}

impl fmt::Display for PmtilesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for PmtilesError {}

/// The PMTiles tile id of an address (Hilbert order within a zoom, zooms stacked).
pub fn tile_id(_tile: TileId) -> u64 {
    todo!("pvec#24")
}

/// Packages tiles as one PMTiles v3 archive.
pub fn write_pmtiles(
    _tiles: &[MvtTile],
    _provenance: &Provenance,
    _options: &PmtilesOptions,
) -> Result<Vec<u8>, PmtilesError> {
    Err(PmtilesError::NotImplemented)
}
