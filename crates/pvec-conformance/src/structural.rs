//! Structural exactness (issue P2.5): parameters compared field by field, but only where two
//! representations are meaningfully equivalent.
//!
//! Comparing a reparameterised spline field by field manufactures false failures, so an equivalence
//! precondition runs first. When it fails the fixture is routed to the geometric gate
//! ([`crate::geometric`]) as reparameterisation-eligible, and it is neither exact nor failed here.
//! Every comparison goes through [`pvec_numeric::Policy`]; this module writes no tolerance.

use crate::evidence::Assertion;
use pvec_capabilities::EvidenceClass;
use pvec_corpus::expected::Node;

/// The spline representation families structural exactness covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// A Bezier curve (no knot vector).
    Bezier,
    /// A non-rational B-spline.
    BSpline,
    /// A rational B-spline.
    Nurbs,
}

/// One distinct knot value with its multiplicity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Knot {
    /// The knot value.
    pub value: f64,
    /// How often it repeats.
    pub multiplicity: usize,
}

/// The parameters structural exactness compares.
#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    /// The representation family.
    pub family: Family,
    /// The degree.
    pub degree: usize,
    /// The control points (poles).
    pub control_points: Vec<[f64; 3]>,
    /// The knots with multiplicities, empty for a Bezier curve.
    pub knots: Vec<Knot>,
    /// The weights of a rational curve.
    pub weights: Option<Vec<f64>>,
    /// Whether the knot vector wraps.
    pub periodic: bool,
    /// Whether the first and last control point coincide.
    pub closed_by_endpoint: bool,
    /// Whether the curve is closed by its knots.
    pub closed_by_knots: bool,
    /// The trim: start and end of the parameter domain.
    pub domain: [f64; 2],
    /// Whether the parameter runs against the source direction.
    pub reversed: bool,
}

/// What the comparison decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Structural {
    /// Every field is equal under the equivalence precondition.
    Exact,
    /// The precondition held and at least one field differs.
    Differs,
    /// The precondition failed: not comparable field by field, sent to the geometric gate.
    RoutedToGeometric,
}

/// One compared field.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The field, for example `knots[1].multiplicity`.
    pub field: String,
    /// The reference value as text.
    pub expected: String,
    /// The compared value as text.
    pub actual: String,
    /// The numeric policy rule applied.
    pub rule: &'static str,
    /// Whether the field is equal.
    pub equal: bool,
    /// The degree of the reference curve.
    pub degree: usize,
    /// The knot count of the reference curve (with multiplicities).
    pub knot_count: usize,
    /// The control point count of the reference curve.
    pub control_point_count: usize,
}

/// The outcome for one fixture.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    /// The fixture id.
    pub fixture: String,
    /// Its evidence class, from the manifest.
    pub class: EvidenceClass,
    /// The assertion a pass supports for that class. `None` for a quarantined fixture.
    pub assertion: Option<Assertion>,
    /// The decision.
    pub result: Structural,
    /// Why the precondition failed, when it did.
    pub reasons: Vec<String>,
    /// The per-field rows, empty when the fixture was routed.
    pub rows: Vec<Row>,
}

impl Structure {
    /// Reads a spline entity of an authored record or of the evaluator's record.
    pub fn from_entity(_entity: &Node) -> Result<Structure, String> {
        todo!("issue 38")
    }
}

/// Compares `actual` with the reference `expected` for a fixture of the given class.
pub fn compare(_fixture: &str, _class: EvidenceClass, _expected: &Structure, _actual: &Structure) -> Report {
    todo!("issue 38")
}

/// The report text, one line per row, or one line saying where the fixture was routed.
pub fn render(_report: &Report) -> String {
    todo!("issue 38")
}
