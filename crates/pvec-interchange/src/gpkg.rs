//! The GeoPackage export (issue P6.5).
//!
//! STUB: the red tests of `tests/gpkg.rs` come first, the implementation follows.

use crate::{Fidelity, FidelityLedger};
use pvec_capabilities::CapabilityId;
use pvec_entity::EntityId;
use pvec_geometry::Curve;

/// The feature table the export writes.
pub const TABLE_NAME: &str = "pvec_features";
/// Its geometry column.
pub const GEOMETRY_COLUMN: &str = "geom";
/// The `last_change` stamp used when the caller gives none: a fixed value, so the file is reproducible.
pub const DEFAULT_LAST_CHANGE: &str = "1970-01-01T00:00:00.000Z";

/// The SQLite write settings the file digest is stable under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WriteSettings {
    /// Page size in bytes.
    pub page_size: u32,
    /// `PRAGMA application_id`: `GPKG`.
    pub application_id: u32,
    /// `PRAGMA user_version`: the GeoPackage version, 1.4.0 as 10400.
    pub user_version: u32,
    /// Schema format number (4).
    pub schema_format: u32,
    /// Text encoding (1, UTF-8).
    pub text_encoding: u32,
    /// The library version number stamped in the header.
    pub sqlite_version_number: u32,
}

/// The settings every file is written with.
pub const WRITE_SETTINGS: WriteSettings = WriteSettings {
    page_size: 4096,
    application_id: 0x4750_4B47,
    user_version: 10400,
    schema_format: 4,
    text_encoding: 1,
    sqlite_version_number: 3_045_000,
};

/// Why an export was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GpkgError {
    /// Not written yet.
    NotImplemented,
    /// The tolerance is not a finite, strictly positive number.
    BadTolerance,
    /// The `last_change` stamp is not an ISO-8601 timestamp.
    BadTimestamp,
    /// The coordinates cannot resolve the tolerance.
    ToleranceUnreachable {
        /// What was asked.
        tolerance: f64,
    },
    /// Two entities share an id.
    DuplicateEntity(EntityId),
}

/// What a caller chooses.
#[derive(Clone, Debug, PartialEq)]
pub struct GpkgOptions {
    tolerance: f64,
    last_change: String,
}

impl GpkgOptions {
    /// Options with the chord tolerance, in drawing units.
    pub fn new(tolerance: f64) -> Result<GpkgOptions, GpkgError> {
        let _ = tolerance;
        Err(GpkgError::NotImplemented)
    }
    /// The same options with a `last_change` stamp (`YYYY-MM-DDTHH:MM:SS.SSSZ`).
    pub fn with_last_change(self, stamp: &str) -> Result<GpkgOptions, GpkgError> {
        let _ = stamp;
        Err(GpkgError::NotImplemented)
    }
    /// The tolerance.
    pub fn tolerance(&self) -> f64 {
        self.tolerance
    }
    /// The stamp.
    pub fn last_change(&self) -> &str {
        &self.last_change
    }
}

/// One entity to export.
#[derive(Clone, Debug, PartialEq)]
pub struct GpkgEntity {
    /// Its id in the source file.
    pub entity_id: EntityId,
    /// Its curve.
    pub curve: Curve,
}

/// The geometry types the export writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GeometryType {
    /// `LINESTRING`.
    LineString,
    /// `CIRCULARSTRING`.
    CircularString,
    /// `COMPOUNDCURVE`.
    CompoundCurve,
}

impl GeometryType {
    /// The name in the target format.
    pub fn name(self) -> &'static str {
        match self {
            GeometryType::LineString => "LINESTRING",
            GeometryType::CircularString => "CIRCULARSTRING",
            GeometryType::CompoundCurve => "COMPOUNDCURVE",
        }
    }
    /// The ISO WKB type code.
    pub fn wkb_code(self) -> u32 {
        match self {
            GeometryType::LineString => 2,
            GeometryType::CircularString => 8,
            GeometryType::CompoundCurve => 9,
        }
    }
}

/// A geometry as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    /// Its type.
    pub kind: GeometryType,
    /// The ISO WKB, little endian.
    pub wkb: Vec<u8>,
    /// `[min x, max x, min y, max y]` of the geometry as written.
    pub envelope: [f64; 4],
}

/// What one curve became.
#[derive(Clone, Debug, PartialEq)]
pub struct Translation {
    /// What the export claims.
    pub fidelity: Fidelity,
    /// The capability the curve uses.
    pub source_capability: CapabilityId,
    /// The geometry, absent when the curve was not translated.
    pub geometry: Option<Geometry>,
    /// The measured largest distance between the written geometry and the exact curve.
    pub measured_deviation: f64,
}

/// Translates one curve at a chord tolerance.
pub fn translate(curve: &Curve, tolerance: f64) -> Result<Translation, GpkgError> {
    let _ = (curve, tolerance);
    Err(GpkgError::NotImplemented)
}

/// A finished export.
#[derive(Clone, Debug)]
pub struct GpkgExport {
    /// The file.
    pub bytes: Vec<u8>,
    /// One record per entity.
    pub ledger: FidelityLedger,
}

/// Writes the file.
pub fn export(entities: &[GpkgEntity], options: &GpkgOptions) -> Result<GpkgExport, GpkgError> {
    let _ = (entities, options);
    Err(GpkgError::NotImplemented)
}
