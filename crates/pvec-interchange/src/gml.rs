//! The GML export (issue P6.4). This is the API surface only: the export maps nothing yet, so the
//! tests that describe the behaviour start red.

use crate::{FidelityKind, FidelityLedger, Representation};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Transform2};

/// The GML version the export declares.
pub const GML_VERSION: &str = "3.2.1";
/// The GML namespace of that version.
pub const GML_NAMESPACE: &str = "http://www.opengis.net/gml/3.2";
/// The code space of the `gml:identifier` that carries a PVec entity id.
pub const ENTITY_CODE_SPACE: &str = "urn:pvec:entity";

/// Why an export was refused outright.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmlError {
    /// The CRS name is blank or contains a control character.
    BadSrsName,
    /// The deviation budget is not finite and strictly positive.
    BadBudget,
    /// Two entities share an id.
    DuplicateEntity(EntityId),
}

/// The target CRS and the transform into it, both supplied by the caller.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsTransform {
    srs_name: String,
    transform: Transform2,
}

impl CrsTransform {
    /// A CRS name and the source-to-CRS transform.
    pub fn new(srs_name: &str, transform: Transform2) -> Result<CrsTransform, GmlError> {
        Ok(CrsTransform { srs_name: srs_name.to_string(), transform })
    }
}

/// The largest deviation an approximated curve may have, in output CRS units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget(f64);

impl Budget {
    /// A budget.
    pub fn new(max_deviation: f64) -> Result<Budget, GmlError> {
        Ok(Budget(max_deviation))
    }
}

/// What an export produced.
#[derive(Clone, Debug, PartialEq)]
pub struct GmlExport {
    /// The document.
    pub xml: String,
    /// One record per emitted segment.
    pub ledger: FidelityLedger,
}

/// A way a PVec capability is mapped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// Placeholder.
    Point,
}

/// One row of the mapping table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingRow {
    /// The capability's registry name.
    pub capability: &'static str,
    /// The case within that capability.
    pub case: Case,
    /// The GML construct written, or `none`.
    pub construct: &'static str,
    /// Native or tessellated.
    pub representation: Representation,
    /// The fidelity value this case carries.
    pub fidelity: FidelityKind,
    /// A note.
    pub note: &'static str,
}

/// The mapping table (empty until the export exists).
pub const MAPPING: &[MappingRow] = &[];

/// The table as markdown.
pub fn mapping_table_markdown() -> String {
    String::new()
}

/// Exports `entities` to GML.
pub fn export(
    _entities: &[(EntityId, Curve)],
    _crs: &CrsTransform,
    _budget: Budget,
) -> Result<GmlExport, GmlError> {
    Ok(GmlExport { xml: String::new(), ledger: FidelityLedger::new() })
}
