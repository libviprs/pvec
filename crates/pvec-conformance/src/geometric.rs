//! Geometric equivalence (issue P2.6): two representations describe the same locus when an
//! independent evaluation of the reference and this repository's evaluation of the candidate agree
//! at seeded, refined, knot-adjacent and endpoint samples.
//!
//! The reference side is the independent STEP evaluator of EPIC O (O1.3), reached through
//! [`Evaluator`]; comparing this repository's evaluator with itself is refused. Every tolerance comes
//! from [`pvec_numeric::Policy`]: this module writes none. Structural exactness (the parameters
//! themselves) is a different gate, and display fidelity is not asserted here at all.

use crate::error::Result;
use pvec_capabilities::EvidenceClass;
use pvec_geometry::Curve2d;
use pvec_numeric::Side;

/// Where an evaluator comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The independent evaluator of EPIC O.
    Independent,
    /// This repository's own evaluator.
    Own,
}

/// A parameter where the curve is less smooth than elsewhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Break {
    /// Where.
    pub parameter: f64,
    /// The highest derivative order that exists two-sided there (NP-16).
    pub two_sided_order: usize,
}

/// What the comparison needs from either side.
pub trait Evaluator {
    /// Which side of the comparison this is.
    fn source(&self) -> Source;
    /// The parameter domain, start then end.
    fn domain(&self) -> [f64; 2];
    /// The interior breaks.
    fn breaks(&self) -> Vec<Break>;
    /// The point at `t`.
    fn point(&self, t: f64) -> [f64; 3];
    /// The unit tangent at `t` from `side`, `None` where it is not defined.
    fn tangent(&self, t: f64, side: Side) -> Option<[f64; 3]>;
    /// The signed curvature at `t` from `side`, `None` where it is not defined.
    fn curvature(&self, t: f64, side: Side) -> Option<f64>;
    /// The arc length of the whole curve.
    fn arc_length(&self) -> f64;
    /// A closed-form bounding box, when there is one.
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])>;
}

/// How a run samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The seed of the sample set.
    pub seed: u64,
    /// How many seeded samples.
    pub samples: usize,
    /// How many evaluations the refinement comparison may spend.
    pub budget: usize,
}

/// The sample parameters of a run, recorded with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// The seed.
    pub seed: u64,
    /// The parameters, in order.
    pub parameters: Vec<f64>,
}

/// A largest deviation and where it happened.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Deviation {
    /// The largest deviation seen.
    pub max: f64,
    /// The parameter that produced it.
    pub at: Option<f64>,
}

/// One compared quantity.
#[derive(Clone, Debug, PartialEq)]
pub struct Metric {
    /// `endpoints`, `position`, `tangent`, `curvature`, `bounds` or `arc_length`.
    pub name: &'static str,
    /// The largest deviation.
    pub max: f64,
    /// The parameter that produced it, when the quantity has one.
    pub at: Option<f64>,
    /// The tolerance applied.
    pub tolerance: f64,
    /// Whether the deviation is within it.
    pub within: bool,
}

/// What fixed-step sampling and adaptive refinement each found with the same budget.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Refinement {
    /// The evaluations each was allowed.
    pub budget: usize,
    /// The position tolerance.
    pub tolerance: f64,
    /// Uniform sampling.
    pub fixed: Deviation,
    /// Interval refinement.
    pub adaptive: Deviation,
}

/// The outcome for one fixture.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The fixture id.
    pub fixture: String,
    /// Its evidence class, from the manifest.
    pub class: EvidenceClass,
    /// The seeded sample set, recorded.
    pub run: Run,
    /// The compared quantities.
    pub metrics: Vec<Metric>,
    /// The refinement comparison.
    pub refinement: Refinement,
    /// Whether every quantity is within its tolerance.
    pub passed: bool,
}

/// The epsilon around a knot, from the numeric policy.
pub fn knot_epsilon(_domain: [f64; 2], _at: f64) -> f64 {
    todo!("issue 39")
}

/// The position tolerance for a reference.
pub fn position_tolerance(_reference: &dyn Evaluator) -> f64 {
    todo!("issue 39")
}

impl Run {
    /// The sample set for a reference: both endpoints, seeded samples, and for each break the
    /// parameter, one epsilon below it and one epsilon above it.
    pub fn new(_reference: &dyn Evaluator, _plan: Plan) -> Run {
        todo!("issue 39")
    }

    /// Fails when another run's parameters differ in any bit.
    pub fn same_as(&self, _other: &Run) -> std::result::Result<(), String> {
        todo!("issue 39")
    }
}

/// Uniform sampling with `budget` evaluations.
pub fn fixed_step(_reference: &dyn Evaluator, _candidate: &dyn Evaluator, _budget: usize) -> Deviation {
    todo!("issue 39")
}

/// Interval refinement with at most `budget` evaluations: half the budget uniform, then bisect the
/// intervals whose ends deviate by at least half the tolerance.
pub fn adaptive(
    _reference: &dyn Evaluator,
    _candidate: &dyn Evaluator,
    _budget: usize,
    _tolerance: f64,
) -> Deviation {
    todo!("issue 39")
}

/// A derivative request at a break. A two-sided request above the break's order is refused (NP-16).
pub fn request(_order: usize, _at: &Break, _side: Side) -> std::result::Result<(), String> {
    todo!("issue 39")
}

/// Compares a candidate with the independent reference.
pub fn compare(
    _fixture: &str,
    _class: EvidenceClass,
    _reference: &dyn Evaluator,
    _candidate: &dyn Evaluator,
    _plan: Plan,
) -> Result<Report> {
    todo!("issue 39")
}

/// The report text, one line per quantity.
pub fn render(_report: &Report) -> String {
    todo!("issue 39")
}

/// Refuses text that words a geometric result as structural exactness or as display fidelity.
pub fn check_wording(_text: &str) -> std::result::Result<(), String> {
    todo!("issue 39")
}

/// This repository's evaluator as the candidate side: [`pvec_geometry::Curve2d`] curves in the
/// z = 0 plane. The curve does not know where its breaks are, so the caller says.
pub struct Candidate<'a, C> {
    /// The curve under test.
    pub curve: &'a C,
    /// Its interior breaks.
    pub breaks: Vec<Break>,
}

impl<C: Curve2d> Evaluator for Candidate<'_, C> {
    fn source(&self) -> Source {
        todo!("issue 39")
    }
    fn domain(&self) -> [f64; 2] {
        todo!("issue 39")
    }
    fn breaks(&self) -> Vec<Break> {
        todo!("issue 39")
    }
    fn point(&self, _t: f64) -> [f64; 3] {
        todo!("issue 39")
    }
    fn tangent(&self, _t: f64, _side: Side) -> Option<[f64; 3]> {
        todo!("issue 39")
    }
    fn curvature(&self, _t: f64, _side: Side) -> Option<f64> {
        todo!("issue 39")
    }
    fn arc_length(&self) -> f64 {
        todo!("issue 39")
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        todo!("issue 39")
    }
}
