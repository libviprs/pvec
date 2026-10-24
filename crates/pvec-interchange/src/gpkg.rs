//! The GeoPackage export (issue P6.5, `docs/INTERCHANGE.md` chapter 4).
//!
//! [`export`] turns PVec curves into a GeoPackage file (as bytes: this crate does no I/O) and a
//! [`FidelityLedger`] saying what it did to each one. A GIS desktop opens the file without repair:
//! the core tables and their required rows, the feature table, the extension rows for every nonlinear
//! geometry type the file uses, and an R-tree spatial index with its seven triggers are all written.
//!
//! * A circular arc becomes a `CIRCULARSTRING` that carries its exact circle, a composite curve becomes
//!   a `COMPOUNDCURVE`, a polyline a `LINESTRING`: the classes the geometry types can say exactly.
//! * A rational spline whose spans are circular arcs becomes those arcs, as an equivalent compound,
//!   after exact evaluation confirms every span follows its circle.
//! * Everything else, ellipses, Bezier curves and the remaining splines, is approximated by a
//!   `LINESTRING` within a world-space tolerance the caller supplies. The ledger and the `error_bound`
//!   column hold the deviation measured against the exact evaluator, not the tolerance asked for.
//!
//! The SQLite file is written by hand ([`sqlite`]), so the crate has no SQLite library and no C source
//! behind it, and the bytes are a pure function of the input: no clock, no random numbers.

mod rtree;
mod sqlite;
mod translate;
mod wkb;

use crate::{
    EmptyText, ErrorBound, Fidelity, FidelityLedger, FidelityRecord, Reason, Representation, TargetConstruct,
};
use pvec_capabilities::{CapabilityId, ids};
use pvec_entity::EntityId;
use pvec_geometry::Curve;
use sqlite::{Value, Writer};
use std::collections::BTreeSet;
use std::fmt;
use translate::{Outcome, translate_curve};
use wkb::{Piece, gpkg_blob, pieces_box, union, wkb};

/// The feature table the export writes.
pub const TABLE_NAME: &str = "pvec_features";
/// Its geometry column.
pub const GEOMETRY_COLUMN: &str = "geom";
/// The `last_change` stamp used when the caller gives none: a fixed value, so the file is reproducible.
pub const DEFAULT_LAST_CHANGE: &str = "1970-01-01T00:00:00.000Z";
/// The spatial reference of the features: the specification's "undefined Cartesian" system, which is
/// what a drawing in its own units is. Reprojection is out of scope.
const SRS_ID: i32 = -1;

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
    /// The library version number stamped in the header. The file is not written by that library; the
    /// number is fixed so the header, and so the digest, never depends on the build.
    pub sqlite_version_number: u32,
}

/// The settings every file is written with: a 4096 byte page, a rollback journal (not WAL), schema
/// format 4, UTF-8, no free pages, no auto-vacuum.
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
    /// The tolerance is not a finite, strictly positive number.
    BadTolerance,
    /// The `last_change` stamp is not an ISO-8601 timestamp of the form `YYYY-MM-DDTHH:MM:SS.SSSZ`.
    BadTimestamp,
    /// The coordinates cannot resolve the tolerance: it is below what the numbers can tell apart, or
    /// meeting it would need more vertices than the export allows.
    ToleranceUnreachable {
        /// What was asked.
        tolerance: f64,
    },
    /// Two entities share an id.
    DuplicateEntity(EntityId),
    /// The curve model refused an evaluation, with the rule it names.
    Evaluation(&'static str),
    /// A value the file format or the ledger could not hold; a bug in the export.
    Internal(&'static str),
}

impl fmt::Display for GpkgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpkgError::BadTolerance => write!(f, "the tolerance must be finite and greater than zero"),
            GpkgError::BadTimestamp => write!(f, "the timestamp must look like 2026-10-24T12:00:00.000Z"),
            GpkgError::ToleranceUnreachable { tolerance } => {
                write!(f, "the coordinates cannot resolve a tolerance of {tolerance}")
            }
            GpkgError::DuplicateEntity(id) => write!(f, "entity {} appears twice", id.index()),
            GpkgError::Evaluation(rule) => write!(f, "the curve model refused an evaluation ({rule})"),
            GpkgError::Internal(what) => write!(f, "internal error: {what}"),
        }
    }
}

impl std::error::Error for GpkgError {}

/// What a caller chooses.
#[derive(Clone, Debug, PartialEq)]
pub struct GpkgOptions {
    tolerance: f64,
    last_change: String,
}

fn valid_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 24 {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        b[r].iter().try_fold(0u32, |acc, c| c.is_ascii_digit().then(|| acc * 10 + u32::from(c - b'0')))
    };
    let shape = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':'), (19, b'.'), (23, b'Z')];
    if shape.iter().any(|&(i, c)| b[i] != c) {
        return false;
    }
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se), Some(_)) = (
        digits(0..4),
        digits(5..7),
        digits(8..10),
        digits(11..13),
        digits(14..16),
        digits(17..19),
        digits(20..23),
    ) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 28 + u32::from(leap),
        _ => return false,
    };
    (1..=days).contains(&d) && h < 24 && mi < 60 && se < 60
}

impl GpkgOptions {
    /// Options with the chord tolerance, in drawing units: the most an approximated curve may deviate
    /// from its exact form. It must be finite and greater than zero.
    pub fn new(tolerance: f64) -> Result<GpkgOptions, GpkgError> {
        if !(tolerance.is_finite() && tolerance > 0.0) {
            return Err(GpkgError::BadTolerance);
        }
        Ok(GpkgOptions { tolerance, last_change: DEFAULT_LAST_CHANGE.to_string() })
    }

    /// The same options with a `last_change` stamp (`YYYY-MM-DDTHH:MM:SS.SSSZ`). The export never reads
    /// a clock: a file carries the stamp it is given, and the same stamp gives the same bytes.
    pub fn with_last_change(mut self, stamp: &str) -> Result<GpkgOptions, GpkgError> {
        if !valid_timestamp(stamp) {
            return Err(GpkgError::BadTimestamp);
        }
        self.last_change = stamp.to_string();
        Ok(self)
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
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
    /// `[min x, max x, min y, max y]` of the geometry as written, arcs included.
    pub envelope: [f64; 4],
    /// Whether any circular arc is inside it, which is what makes the `CIRCULARSTRING` extension
    /// necessary even when the type of the whole is `COMPOUNDCURVE`.
    pub contains_arcs: bool,
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
    /// For a bounded approximation, the recorded bound (the measured value, inflated by a thousandth
    /// to err high). For an exact class, the largest residual found when the written points were
    /// compared with the exact curve at sample points; zero for a copy. `None` when nothing was
    /// written.
    pub measured_deviation: Option<f64>,
    /// A sentence for the ledger.
    pub note: String,
}

/// How far a result is from the source, in the order the claims weaken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Rank {
    Exact,
    Repar,
    Equivalent,
    Bounded,
}

fn capability_of(curve: &Curve) -> CapabilityId {
    match curve {
        Curve::Line(_) => ids::LINE,
        Curve::Circle(_) => ids::CIRCLE,
        Curve::Ellipse(_) => ids::ELLIPSE,
        Curve::Polyline(_) => ids::POLYLINE,
        Curve::Trimmed(_) => ids::TRIMMED_CURVE,
        Curve::Composite(_) => ids::COMPOSITE_CURVE,
        Curve::Bezier(_) => ids::BEZIER_CURVE,
        Curve::BSpline(_) => ids::B_SPLINE_CURVE,
        Curve::RationalBSpline(_) => ids::RATIONAL_B_SPLINE_CURVE,
        _ => ids::CARTESIAN_POINT,
    }
}

/// Translates one curve at a chord tolerance: the geometry, the claim and the measured deviation.
pub fn translate(curve: &Curve, tolerance: f64) -> Result<Translation, GpkgError> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err(GpkgError::BadTolerance);
    }
    let source_capability = capability_of(curve);
    match translate_curve(curve, tolerance)? {
        Outcome::Unsupported(reason) => Ok(Translation {
            fidelity: Fidelity::Unsupported(
                Reason::new(reason).map_err(|EmptyText| GpkgError::Internal("an empty reason"))?,
            ),
            source_capability,
            geometry: None,
            measured_deviation: None,
            note: String::new(),
        }),
        Outcome::Done(done) => {
            let (kind, bytes) = wkb(&done.pieces, done.compound);
            let fidelity = match done.rank {
                Rank::Exact => Fidelity::Exact,
                Rank::Repar => Fidelity::ExactReparameterized,
                Rank::Equivalent => Fidelity::EquivalentCompound,
                Rank::Bounded => {
                    let bound =
                        done.deviation.ok_or(GpkgError::Internal("an approximation without a measure"))?;
                    Fidelity::BoundedApproximation(
                        ErrorBound::new(bound)
                            .map_err(|_| GpkgError::Internal("a deviation that is not a bound"))?,
                    )
                }
            };
            let note = if done.rank == Rank::Bounded {
                format!("tolerance {tolerance}, measured {}; {}", done.deviation.unwrap_or(0.0), done.note)
            } else {
                done.note
            };
            Ok(Translation {
                fidelity,
                source_capability,
                geometry: Some(Geometry {
                    kind,
                    wkb: bytes,
                    envelope: pieces_box(&done.pieces),
                    contains_arcs: done.pieces.iter().any(|p| matches!(p, Piece::Arcs(_))),
                }),
                measured_deviation: done.deviation,
                note,
            })
        }
    }
}

/// A finished export.
#[derive(Clone, Debug)]
pub struct GpkgExport {
    /// The file.
    pub bytes: Vec<u8>,
    /// One record per entity, in the order the entities were given.
    pub ledger: FidelityLedger,
}

// ---- schema text ----------------------------------------------------------------------------------

const SRS_SQL: &str = "CREATE TABLE gpkg_spatial_ref_sys (srs_name TEXT NOT NULL, srs_id INTEGER PRIMARY KEY, organization TEXT NOT NULL, organization_coordsys_id INTEGER NOT NULL, definition TEXT NOT NULL, description TEXT)";
const CONTENTS_SQL: &str = "CREATE TABLE gpkg_contents (table_name TEXT NOT NULL PRIMARY KEY, data_type TEXT NOT NULL, identifier TEXT UNIQUE, description TEXT DEFAULT '', last_change DATETIME NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')), min_x DOUBLE, min_y DOUBLE, max_x DOUBLE, max_y DOUBLE, srs_id INTEGER, CONSTRAINT fk_gc_r_srs_id FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys(srs_id))";
const GEOMETRY_COLUMNS_SQL: &str = "CREATE TABLE gpkg_geometry_columns (table_name TEXT NOT NULL, column_name TEXT NOT NULL, geometry_type_name TEXT NOT NULL, srs_id INTEGER NOT NULL, z TINYINT NOT NULL, m TINYINT NOT NULL, CONSTRAINT pk_geom_cols PRIMARY KEY (table_name, column_name), CONSTRAINT uk_gc_table_name UNIQUE (table_name), CONSTRAINT fk_gc_tn FOREIGN KEY (table_name) REFERENCES gpkg_contents(table_name), CONSTRAINT fk_gc_srs FOREIGN KEY (srs_id) REFERENCES gpkg_spatial_ref_sys (srs_id))";
const EXTENSIONS_SQL: &str = "CREATE TABLE gpkg_extensions (table_name TEXT, column_name TEXT, extension_name TEXT NOT NULL, definition TEXT NOT NULL, scope TEXT NOT NULL, CONSTRAINT ge_tce UNIQUE (table_name, column_name, extension_name))";
const FEATURES_SQL: &str = "CREATE TABLE \"pvec_features\" (\"fid\" INTEGER PRIMARY KEY NOT NULL, \"geom\" GEOMETRY, \"entity_id\" INTEGER NOT NULL, \"fidelity\" TEXT NOT NULL, \"error_bound\" REAL, \"source_capability\" INTEGER NOT NULL)";

const WGS84: &str = "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563,AUTHORITY[\"EPSG\",\"7030\"]],AUTHORITY[\"EPSG\",\"6326\"]],PRIMEM[\"Greenwich\",0,AUTHORITY[\"EPSG\",\"8901\"]],UNIT[\"degree\",0.0174532925199433,AUTHORITY[\"EPSG\",\"9122\"]],AUTHORITY[\"EPSG\",\"4326\"]]";

const GEOM_TYPES_DEFINITION: &str = "http://www.geopackage.org/spec/#extension_geometry_types";
const RTREE_DEFINITION: &str = "http://www.geopackage.org/spec120/#extension_rtree";

fn rtree_name() -> String {
    format!("rtree_{TABLE_NAME}_{GEOMETRY_COLUMN}")
}

/// The seven triggers the GeoPackage 1.4 specification gives the R-tree index (`insert`, `update2`,
/// `update4` to `update7` and `delete`; the numbering skips the two of earlier versions), with the
/// statements as the reference implementation writes them, so the validator that checks them by
/// meaning and the readers that run them find what they expect.
fn triggers() -> Vec<(&'static str, String)> {
    let (t, c, r) = (TABLE_NAME, GEOMETRY_COLUMN, rtree_name());
    let live = format!("(NEW.\"{c}\" NOTNULL AND NOT ST_IsEmpty(NEW.\"{c}\"))");
    let dead = format!("(NEW.\"{c}\" ISNULL OR ST_IsEmpty(NEW.\"{c}\"))");
    let put = format!(
        "INSERT OR REPLACE INTO \"{r}\" VALUES (NEW.\"fid\",ST_MinX(NEW.\"{c}\"), ST_MaxX(NEW.\"{c}\"),ST_MinY(NEW.\"{c}\"), ST_MaxY(NEW.\"{c}\"));"
    );
    vec![
        (
            "insert",
            format!(
                "CREATE TRIGGER \"{r}_insert\" AFTER INSERT ON \"{t}\" WHEN (new.\"{c}\" NOT NULL AND NOT ST_IsEmpty(NEW.\"{c}\")) BEGIN {put} END"
            ),
        ),
        (
            "update6",
            format!(
                "CREATE TRIGGER \"{r}_update6\" AFTER UPDATE OF \"{c}\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND {live} AND (OLD.\"{c}\" NOTNULL AND NOT ST_IsEmpty(OLD.\"{c}\")) BEGIN UPDATE \"{r}\" SET minx = ST_MinX(NEW.\"{c}\"), maxx = ST_MaxX(NEW.\"{c}\"),miny = ST_MinY(NEW.\"{c}\"), maxy = ST_MaxY(NEW.\"{c}\") WHERE id = NEW.\"fid\";END"
            ),
        ),
        (
            "update7",
            format!(
                "CREATE TRIGGER \"{r}_update7\" AFTER UPDATE OF \"{c}\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND {live} AND (OLD.\"{c}\" ISNULL OR ST_IsEmpty(OLD.\"{c}\")) BEGIN INSERT INTO \"{r}\" VALUES (NEW.\"fid\",ST_MinX(NEW.\"{c}\"), ST_MaxX(NEW.\"{c}\"),ST_MinY(NEW.\"{c}\"), ST_MaxY(NEW.\"{c}\")); END"
            ),
        ),
        (
            "update2",
            format!(
                "CREATE TRIGGER \"{r}_update2\" AFTER UPDATE OF \"{c}\" ON \"{t}\" WHEN OLD.\"fid\" = NEW.\"fid\" AND {dead} BEGIN DELETE FROM \"{r}\" WHERE id = OLD.\"fid\"; END"
            ),
        ),
        (
            "update5",
            format!(
                "CREATE TRIGGER \"{r}_update5\" AFTER UPDATE ON \"{t}\" WHEN OLD.\"fid\" != NEW.\"fid\" AND {live} BEGIN DELETE FROM \"{r}\" WHERE id = OLD.\"fid\"; {put} END"
            ),
        ),
        (
            "update4",
            format!(
                "CREATE TRIGGER \"{r}_update4\" AFTER UPDATE ON \"{t}\" WHEN OLD.\"fid\" != NEW.\"fid\" AND {dead} BEGIN DELETE FROM \"{r}\" WHERE id IN (OLD.\"fid\", NEW.\"fid\"); END"
            ),
        ),
        (
            "delete",
            format!(
                "CREATE TRIGGER \"{r}_delete\" AFTER DELETE ON \"{t}\" WHEN old.\"{c}\" NOT NULL BEGIN DELETE FROM \"{r}\" WHERE id = OLD.\"fid\"; END"
            ),
        ),
    ]
}

fn layout<T>(r: Result<T, sqlite::LayoutError>) -> Result<T, GpkgError> {
    r.map_err(|sqlite::LayoutError::DoesNotFit(what)| GpkgError::Internal(what))
}

/// Writes the file: translates every entity, records what was done in the ledger and lays out the
/// GeoPackage. Rows are in the order the entities were given.
pub fn export(entities: &[GpkgEntity], options: &GpkgOptions) -> Result<GpkgExport, GpkgError> {
    let mut seen = BTreeSet::new();
    for e in entities {
        if !seen.insert(e.entity_id.index()) {
            return Err(GpkgError::DuplicateEntity(e.entity_id));
        }
    }
    let mut ledger = FidelityLedger::new();
    let mut rows: Vec<(i64, Vec<Value>)> = Vec::new();
    let mut boxes: Vec<(i64, [f64; 4])> = Vec::new();
    let mut total: Option<[f64; 4]> = None;
    let (mut circular, mut compound) = (false, false);
    for e in entities {
        let t = translate(&e.curve, options.tolerance())?;
        let (name, repr) = match &t.geometry {
            Some(g) => (
                g.kind.name(),
                if matches!(t.fidelity, Fidelity::BoundedApproximation(_)) {
                    Representation::Tessellated
                } else {
                    Representation::Native
                },
            ),
            None => ("none", Representation::Native),
        };
        let target =
            TargetConstruct::new(name, repr).map_err(|EmptyText| GpkgError::Internal("an empty target"))?;
        let note = match &t.fidelity {
            Fidelity::Unsupported(r) => r.as_str().to_string(),
            _ => t.note.clone(),
        };
        let record = FidelityRecord::new(e.entity_id, t.source_capability, target, t.fidelity.clone(), &note)
            .map_err(|_| GpkgError::Internal("a record the taxonomy refuses"))?;
        ledger.record(record);
        let Some(g) = &t.geometry else { continue };
        circular |= g.kind == GeometryType::CircularString || g.contains_arcs;
        compound |= g.kind == GeometryType::CompoundCurve;
        let fid = rows.len() as i64 + 1;
        total = Some(match total {
            Some(mut b) => {
                union(&mut b, g.envelope);
                b
            }
            None => g.envelope,
        });
        boxes.push((fid, g.envelope));
        rows.push((
            fid,
            vec![
                Value::Null,
                Value::Blob(gpkg_blob(SRS_ID, g.envelope, &g.wkb)),
                Value::Int(i64::from(e.entity_id.index())),
                Value::text(t.fidelity.kind().name()),
                match &t.fidelity {
                    Fidelity::BoundedApproximation(b) => Value::Real(b.get()),
                    _ => Value::Null,
                },
                Value::Int(i64::from(t.source_capability.get())),
            ],
        ));
    }

    let mut w = Writer::new();
    let text = Value::text;
    let srs = vec![
        (
            -1,
            vec![
                text("Undefined Cartesian SRS"),
                Value::Null,
                text("NONE"),
                Value::Int(-1),
                text("undefined"),
                text("undefined Cartesian coordinate reference system"),
            ],
        ),
        (
            0,
            vec![
                text("Undefined geographic SRS"),
                Value::Null,
                text("NONE"),
                Value::Int(0),
                text("undefined"),
                text("undefined geographic coordinate reference system"),
            ],
        ),
        (
            4326,
            vec![
                text("WGS 84"),
                Value::Null,
                text("EPSG"),
                Value::Int(4326),
                text(WGS84),
                text("longitude/latitude coordinates in decimal degrees on the WGS 84 spheroid"),
            ],
        ),
    ];
    layout(w.table("gpkg_spatial_ref_sys", SRS_SQL, &srs))?;

    // the envelope is [min x, max x, min y, max y]; the table wants min x, min y, max x, max y
    let bound = |i: usize| total.map_or(Value::Null, |b| Value::Real(b[i]));
    let identifier = "PVec features";
    let contents = vec![(
        1,
        vec![
            text(TABLE_NAME),
            text("features"),
            text(identifier),
            text("Curves exported from a PVec file; the fidelity column says how each was translated"),
            text(options.last_change()),
            bound(0),
            bound(2),
            bound(1),
            bound(3),
            Value::Int(i64::from(SRS_ID)),
        ],
    )];
    layout(w.table("gpkg_contents", CONTENTS_SQL, &contents))?;
    layout(w.autoindex("gpkg_contents", 1, vec![vec![text(TABLE_NAME), Value::Int(1)]]))?;
    layout(w.autoindex("gpkg_contents", 2, vec![vec![text(identifier), Value::Int(1)]]))?;

    let geometry_columns = vec![(
        1,
        vec![
            text(TABLE_NAME),
            text(GEOMETRY_COLUMN),
            text("GEOMETRY"),
            Value::Int(i64::from(SRS_ID)),
            Value::Int(0),
            Value::Int(0),
        ],
    )];
    layout(w.table("gpkg_geometry_columns", GEOMETRY_COLUMNS_SQL, &geometry_columns))?;
    layout(w.autoindex(
        "gpkg_geometry_columns",
        1,
        vec![vec![text(TABLE_NAME), text(GEOMETRY_COLUMN), Value::Int(1)]],
    ))?;
    layout(w.autoindex("gpkg_geometry_columns", 2, vec![vec![text(TABLE_NAME), Value::Int(1)]]))?;

    let mut extensions: Vec<(String, &str, &str)> = Vec::new();
    if circular {
        extensions.push(("gpkg_geom_CIRCULARSTRING".into(), GEOM_TYPES_DEFINITION, "read-write"));
    }
    if compound {
        extensions.push(("gpkg_geom_COMPOUNDCURVE".into(), GEOM_TYPES_DEFINITION, "read-write"));
    }
    extensions.push(("gpkg_rtree_index".into(), RTREE_DEFINITION, "write-only"));
    let ext_rows: Vec<(i64, Vec<Value>)> = extensions
        .iter()
        .enumerate()
        .map(|(i, (name, def, scope))| {
            (i as i64 + 1, vec![text(TABLE_NAME), text(GEOMETRY_COLUMN), text(name), text(def), text(scope)])
        })
        .collect();
    layout(w.table("gpkg_extensions", EXTENSIONS_SQL, &ext_rows))?;
    let ext_index = extensions
        .iter()
        .enumerate()
        .map(|(i, (name, _, _))| {
            vec![text(TABLE_NAME), text(GEOMETRY_COLUMN), text(name), Value::Int(i as i64 + 1)]
        })
        .collect();
    layout(w.autoindex("gpkg_extensions", 1, ext_index))?;

    layout(w.table(TABLE_NAME, FEATURES_SQL, &rows))?;

    let rt = rtree_name();
    let shadow = rtree::build(&boxes, w.page_size() - 64);
    w.virtual_table(&rt, &format!("CREATE VIRTUAL TABLE \"{rt}\" USING rtree(id, minx, maxx, miny, maxy)"));
    // the shadow tables as SQLite's R-tree module itself declares them, in the order it creates them
    layout(w.table(
        &format!("{rt}_rowid"),
        &format!("CREATE TABLE \"{rt}_rowid\"(rowid INTEGER PRIMARY KEY,nodeno)"),
        &shadow.rowid,
    ))?;
    layout(w.table(
        &format!("{rt}_node"),
        &format!("CREATE TABLE \"{rt}_node\"(nodeno INTEGER PRIMARY KEY,data)"),
        &shadow.node,
    ))?;
    layout(w.table(
        &format!("{rt}_parent"),
        &format!("CREATE TABLE \"{rt}_parent\"(nodeno INTEGER PRIMARY KEY,parentnode)"),
        &shadow.parent,
    ))?;
    for (suffix, sql) in triggers() {
        w.trigger(&format!("{rt}_{suffix}"), TABLE_NAME, &sql);
    }
    let bytes = layout(w.finish())?;
    Ok(GpkgExport { bytes, ledger })
}
