//! The five conformance gates and the runner that records them (issue P2.8).
//!
//! `docs/CONFORMANCE.md` defines what each gate requires; this module is the same table as code. The
//! gates are independent: a fixture can pass the geometric gate and fail the structural one, and the
//! runner keeps both rows. Nothing here writes a tolerance (they come from `docs/NUMERIC_POLICY.md`
//! through the P2.5 and P2.6 modules) and nothing describes a gate as ISO conformance (the only claim
//! is the one in `docs/ISO_PROFILE.md`).

use crate::error::Result;
use crate::{geometric, structural};
use pvec_capabilities::EvidenceClass;

/// The five gates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Every structural field equal under the equivalence precondition.
    StructuralExact,
    /// Deviation within the policy tolerance across the whole sampling strategy.
    GeometricExact,
    /// Bounded screen-space and world-space error, with the budget stated.
    DisplayApproximate,
    /// Header, directory, integrity and limits accepted by the safe reader.
    FormatValid,
    /// Unknown optional blocks skipped without parsing their payload.
    ForwardCompatible,
}

/// What a gate decided for one fixture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    /// The gate holds.
    Pass,
    /// It does not.
    Fail,
    /// The gate does not apply and the fixture went to another one (structural to geometric).
    Routed,
}

/// One row of the report.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The gate.
    pub gate: Gate,
    /// The fixture id.
    pub fixture: String,
    /// Its evidence class, from the manifest.
    pub class: EvidenceClass,
    /// The verdict.
    pub verdict: GateVerdict,
    /// The numbers behind it, by name.
    pub numbers: Vec<(String, f64)>,
}

/// Every claim made in a run, in order.
#[derive(Clone, Debug, Default)]
pub struct Runner {
    rows: Vec<Row>,
}

impl Gate {
    /// Every gate, in the order of `docs/CONFORMANCE.md`.
    pub const ALL: [Gate; 5] = [
        Gate::StructuralExact,
        Gate::GeometricExact,
        Gate::DisplayApproximate,
        Gate::FormatValid,
        Gate::ForwardCompatible,
    ];

    /// The name the report and the document use.
    pub fn name(self) -> &'static str {
        todo!("issue 41")
    }

    /// The evidence classes that may satisfy the gate. A quarantined fixture satisfies none.
    pub fn classes(self) -> &'static [EvidenceClass] {
        todo!("issue 41")
    }

    /// The word a verdict is reported with. The display gate never has a word for exactness.
    pub fn word(self, _verdict: GateVerdict) -> &'static str {
        todo!("issue 41")
    }
}

impl Runner {
    /// An empty run.
    pub fn new() -> Runner {
        Runner::default()
    }

    /// The rows so far.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Records a claim, refusing one the fixture's class cannot satisfy.
    pub fn claim(
        &mut self,
        _gate: Gate,
        _fixture: &str,
        _class: EvidenceClass,
        _verdict: GateVerdict,
        _numbers: &[(&str, f64)],
    ) -> Result<()> {
        todo!("issue 41")
    }

    /// Records a structural comparison (P2.5) under the structural gate.
    pub fn structural(&mut self, _report: &structural::Report) -> Result<()> {
        todo!("issue 41")
    }

    /// Records a geometric comparison (P2.6) under the geometric gate.
    pub fn geometric(&mut self, _report: &geometric::Report) -> Result<()> {
        todo!("issue 41")
    }

    /// Records a display result: each error with the budget it is held to, screen then world.
    pub fn display(
        &mut self,
        _fixture: &str,
        _class: EvidenceClass,
        _screen: (f64, f64),
        _world: (f64, f64),
    ) -> Result<()> {
        todo!("issue 41")
    }

    /// Every gate's verdict for one fixture, side by side, never collapsed into one.
    pub fn verdicts(&self, _fixture: &str) -> Vec<(Gate, GateVerdict)> {
        todo!("issue 41")
    }

    /// The report text, one line per row.
    pub fn render(&self) -> String {
        todo!("issue 41")
    }
}
