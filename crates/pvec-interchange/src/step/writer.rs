//! The Part 21 writer.

use crate::FidelityLedger;
use pvec_entity::{EntityId, EntityTable};
use pvec_geometry::{Curve, Transform2};

/// Why an export was refused outright.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    /// The file name is blank or contains a control character.
    BadFileName,
    /// The timestamp is not `YYYY-MM-DDThh:mm:ss`.
    BadTimestamp,
    /// Two entities share an id.
    DuplicateEntity(EntityId),
    /// The entity is a preview, which is not geometry.
    Preview(EntityId),
    /// The table has no such entity.
    UnknownEntity(EntityId),
}

/// An entity the writer accepts: an exact curve under an id.
#[derive(Clone, Debug, PartialEq)]
pub struct StepEntity {
    id: EntityId,
    curve: Curve,
}

impl StepEntity {
    /// An exact curve.
    pub fn exact(id: EntityId, curve: Curve) -> StepEntity {
        StepEntity { id, curve }
    }

    /// The entity `id` of `table`, refused when the table says it is a preview.
    pub fn from_table(_table: &EntityTable, _id: EntityId, _curve: Curve) -> Result<StepEntity, StepError> {
        unimplemented!("P6.2")
    }
}

/// What the caller supplies.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOptions {
    file_name: String,
    timestamp: String,
    transform: Transform2,
}

impl StepOptions {
    /// The file name and the timestamp that go in the header.
    pub fn new(_file_name: &str, _timestamp: &str) -> Result<StepOptions, StepError> {
        unimplemented!("P6.2")
    }

    /// Applies `transform` to every curve before it is written.
    pub fn with_transform(mut self, transform: Transform2) -> StepOptions {
        self.transform = transform;
        self
    }
}

/// What an export produced.
#[derive(Clone, Debug, PartialEq)]
pub struct StepExport {
    /// The file, ASCII, byte-identical for identical input.
    pub part21: String,
    /// One record per entity.
    pub ledger: FidelityLedger,
}

/// Exports `entities`.
///
/// The input type is the guard: only a [`StepEntity`] goes in, and a `StepEntity` is an exact curve.
/// A bare `(EntityId, Curve)` pair, a list of sampled points, or anything a display export produced
/// does not type-check:
///
/// ```compile_fail
/// use pvec_interchange::step::{StepOptions, export};
/// use pvec_geometry::Point2;
/// let sampled: Vec<Point2> = vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let _ = export(&sampled, &options);
/// ```
///
/// ```compile_fail
/// use pvec_interchange::step::{StepOptions, export};
/// use pvec_entity::EntityId;
/// use pvec_geometry::{Curve, Polyline, Point2};
/// let preview = Curve::Polyline(Polyline::new(vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)]).unwrap());
/// let pairs: Vec<(EntityId, Curve)> = vec![(EntityId::new(0).unwrap(), preview)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let _ = export(&pairs, &options);
/// ```
///
/// The same call with the right input type compiles and runs:
///
/// ```
/// use pvec_interchange::step::{StepEntity, StepOptions, export};
/// use pvec_entity::EntityId;
/// use pvec_geometry::{Curve, Polyline, Point2};
/// let line = Curve::Polyline(Polyline::new(vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)]).unwrap());
/// let entities = vec![StepEntity::exact(EntityId::new(0).unwrap(), line)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let out = export(&entities, &options).unwrap();
/// assert!(out.part21.starts_with("ISO-10303-21;"));
/// ```
pub fn export(_entities: &[StepEntity], _options: &StepOptions) -> Result<StepExport, StepError> {
    unimplemented!("P6.2")
}
