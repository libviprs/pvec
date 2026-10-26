//! The five conformance gates and the runner that records them (issue P2.8).
//!
//! `docs/CONFORMANCE.md` defines what each gate requires; this module is the same table as code. The
//! gates are independent: a fixture can pass the geometric gate and fail the structural one, and the
//! runner keeps both rows. Nothing here writes a tolerance (they come from `docs/NUMERIC_POLICY.md`
//! through the P2.5 and P2.6 modules) and nothing describes a gate as ISO conformance (the only claim
//! is the one in `docs/ISO_PROFILE.md`).

use crate::error::{ConformanceError, Result};
use crate::evidence::meaning;
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
        match self {
            Gate::StructuralExact => "structural exact",
            Gate::GeometricExact => "geometric exact",
            Gate::DisplayApproximate => "display approximate",
            Gate::FormatValid => "format valid",
            Gate::ForwardCompatible => "forward compatible",
        }
    }

    /// The evidence classes that may satisfy the gate. A quarantined fixture satisfies none.
    pub fn classes(self) -> &'static [EvidenceClass] {
        match self {
            Gate::StructuralExact | Gate::GeometricExact => {
                &[EvidenceClass::KnownTruthExact, EvidenceClass::DualOracleEquivalent]
            }
            Gate::DisplayApproximate | Gate::FormatValid | Gate::ForwardCompatible => &[
                EvidenceClass::KnownTruthExact,
                EvidenceClass::DualOracleEquivalent,
                EvidenceClass::RealWorldDifferential,
            ],
        }
    }

    /// The word a verdict is reported with. The display gate never has a word for exactness.
    pub fn word(self, verdict: GateVerdict) -> &'static str {
        match (self, verdict) {
            (Gate::StructuralExact, GateVerdict::Pass) => "exact",
            (Gate::StructuralExact, GateVerdict::Fail) => "differs",
            (Gate::GeometricExact, GateVerdict::Pass) => "same locus within the policy tolerance",
            (Gate::GeometricExact, GateVerdict::Fail) => "locus deviates",
            (Gate::DisplayApproximate, GateVerdict::Pass) => "approximate, within budget",
            (Gate::DisplayApproximate, GateVerdict::Fail) => "approximate, over budget",
            (Gate::DisplayApproximate, GateVerdict::Routed) => "approximate, no budget to hold it to",
            (Gate::FormatValid, GateVerdict::Pass) => "valid",
            (Gate::FormatValid, GateVerdict::Fail) => "invalid",
            (Gate::ForwardCompatible, GateVerdict::Pass) => "compatible",
            (Gate::ForwardCompatible, GateVerdict::Fail) => "incompatible",
            (_, GateVerdict::Routed) => "routed to the geometric gate",
        }
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
        gate: Gate,
        fixture: &str,
        class: EvidenceClass,
        verdict: GateVerdict,
        numbers: &[(&str, f64)],
    ) -> Result<()> {
        if !gate.classes().contains(&class) {
            return Err(ConformanceError::GateRefused {
                gate: gate.name().to_string(),
                fixture: fixture.to_string(),
                class: class.name().to_string(),
            });
        }
        self.rows.push(Row {
            gate,
            fixture: fixture.to_string(),
            class,
            verdict,
            numbers: numbers.iter().map(|(n, v)| ((*n).to_string(), *v)).collect(),
        });
        Ok(())
    }

    /// Records a structural comparison (P2.5) under the structural gate.
    pub fn structural(&mut self, report: &structural::Report) -> Result<()> {
        let verdict = match report.result {
            structural::Structural::Exact => GateVerdict::Pass,
            structural::Structural::Differs => GateVerdict::Fail,
            structural::Structural::RoutedToGeometric => GateVerdict::Routed,
        };
        let failing = report.rows.iter().filter(|r| !r.equal).count() as f64;
        let mut numbers = Vec::new();
        if let Some(first) = report.rows.first() {
            numbers.extend([
                ("degree", first.degree as f64),
                ("knot_count", first.knot_count as f64),
                ("control_point_count", first.control_point_count as f64),
            ]);
        }
        numbers.push(("failing_fields", failing));
        self.claim(Gate::StructuralExact, &report.fixture, report.class, verdict, &numbers)
    }

    /// Records a geometric comparison (P2.6) under the geometric gate.
    pub fn geometric(&mut self, report: &geometric::Report) -> Result<()> {
        let metric = |name: &str| report.metrics.iter().find(|m| m.name == name);
        let mut numbers = Vec::new();
        if let Some(m) = metric("position") {
            numbers.push(("max_deviation", m.max));
            if let Some(at) = m.at {
                numbers.push(("worst_parameter", at));
            }
            numbers.push(("tolerance", m.tolerance));
        }
        for (name, key) in [
            ("tangent", "tangent_deviation"),
            ("curvature", "curvature_deviation"),
            ("arc_length", "arc_length_deviation"),
        ] {
            if let Some(m) = metric(name) {
                numbers.push((key, m.max));
            }
        }
        let verdict = if report.passed { GateVerdict::Pass } else { GateVerdict::Fail };
        self.claim(Gate::GeometricExact, &report.fixture, report.class, verdict, &numbers)
    }

    /// Records a display result: each error with the budget it is held to, screen then world.
    pub fn display(
        &mut self,
        fixture: &str,
        class: EvidenceClass,
        screen: (f64, f64),
        world: (f64, f64),
    ) -> Result<()> {
        let within = |(error, budget): (f64, f64)| error.is_finite() && error <= budget;
        let verdict = if within(screen) && within(world) { GateVerdict::Pass } else { GateVerdict::Fail };
        let numbers = [
            ("screen_error", screen.0),
            ("screen_budget", screen.1),
            ("world_error", world.0),
            ("world_budget", world.1),
        ];
        self.claim(Gate::DisplayApproximate, fixture, class, verdict, &numbers)
    }

    /// Every gate's verdict for one fixture, side by side, never collapsed into one.
    pub fn verdicts(&self, fixture: &str) -> Vec<(Gate, GateVerdict)> {
        self.rows.iter().filter(|r| r.fixture == fixture).map(|r| (r.gate, r.verdict)).collect()
    }

    /// The report text, one line per row.
    pub fn render(&self) -> String {
        self.rows
            .iter()
            .map(|r| {
                let verdict = match r.verdict {
                    GateVerdict::Pass => "PASS",
                    GateVerdict::Fail => "FAIL",
                    GateVerdict::Routed => "ROUTED",
                };
                let numbers: Vec<String> = r.numbers.iter().map(|(n, v)| format!("{n}={v:?}")).collect();
                format!(
                    "{} [{}] {}: {verdict} ({}) {} ({})",
                    r.fixture,
                    r.class.name(),
                    r.gate.name(),
                    r.gate.word(r.verdict),
                    numbers.join(" "),
                    meaning(r.class)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
