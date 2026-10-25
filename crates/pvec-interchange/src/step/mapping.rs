//! The capability-to-AP242 mapping table.

use crate::{FidelityKind, Representation};

/// A way a PVec capability is mapped. Each variant is one row of the table in `docs/INTERCHANGE.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// A point of a curve.
    Point,
}

/// One row of the mapping table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingRow {
    /// The capability's registry name.
    pub capability: &'static str,
    /// The case within that capability.
    pub case: Case,
    /// The Part 21 entity written, or `none`.
    pub construct: &'static str,
    /// Native or tessellated.
    pub representation: Representation,
    /// The fidelity value this case carries.
    pub fidelity: FidelityKind,
    /// For `Unsupported`, the reason. For the others, how the value is obtained.
    pub note: &'static str,
}

/// The mapping table.
pub const MAPPING: &[MappingRow] = &[];

/// The table as the markdown `docs/INTERCHANGE.md` carries.
pub fn mapping_table_markdown() -> String {
    String::new()
}
