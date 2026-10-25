//! The round-trip comparison harness (issue P6.3). Stub: the tests in `tests/step_roundtrip.rs` come first.

use crate::Fidelity;
use pvec_entity::EntityId;
use pvec_geometry::Curve;

/// Which conformance class a disagreement is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Parameters that are directly comparable.
    Structural,
    /// The locus.
    Geometric,
    /// Display fidelity against a recorded bound.
    Display,
}

/// One disagreement between PVec and the evaluator.
#[derive(Clone, Debug, PartialEq)]
pub struct Disagreement {
    /// The entity.
    pub entity: EntityId,
    /// The class that failed.
    pub class: Class,
    /// PVec's parameters.
    pub pvec: String,
    /// The evaluator's parameters.
    pub step: String,
    /// The deviating sample.
    pub sample: String,
    /// The tolerance applied.
    pub tolerance: f64,
}

/// An entity under test.
#[derive(Clone, Debug)]
pub struct Subject<'a> {
    /// The entity id.
    pub id: EntityId,
    /// The curve that was exported.
    pub curve: &'a Curve,
    /// What the export claimed.
    pub fidelity: Fidelity,
}

/// What the evaluator printed.
#[derive(Clone, Debug)]
pub struct Report;

impl Report {
    /// Parses the evaluator's output.
    pub fn parse(_text: &str) -> Result<Report, String> {
        todo!()
    }
    /// The evaluator line.
    pub fn evaluator(&self) -> &str {
        todo!()
    }
    /// Edge count.
    pub fn edges(&self) -> usize {
        todo!()
    }
}

/// The sample requests for the second evaluator run.
pub fn requests(_subjects: &[Subject<'_>], _report: &Report) -> Result<String, String> {
    todo!()
}

/// The outcome, structural and geometric results kept apart.
#[derive(Clone, Debug, Default)]
pub struct Verdict {
    /// Structural disagreements.
    pub structural: Vec<Disagreement>,
    /// Geometric disagreements.
    pub geometric: Vec<Disagreement>,
    /// Display disagreements.
    pub display: Vec<Disagreement>,
    /// Entities claimed exact that agreed in every class.
    pub exact_passes: usize,
    /// Structural parameter comparisons made.
    pub structural_checked: usize,
    /// Entities whose parameters are not directly comparable, with the reason.
    pub structural_not_comparable: Vec<(EntityId, &'static str)>,
    /// Geometric samples compared.
    pub geometric_checked: usize,
    /// Display samples compared.
    pub display_checked: usize,
}

impl Verdict {
    /// No disagreement in any class.
    pub fn passed(&self) -> bool {
        todo!()
    }
}

/// Compares.
pub fn compare(_subjects: &[Subject<'_>], _report: &Report) -> Result<Verdict, String> {
    todo!()
}
