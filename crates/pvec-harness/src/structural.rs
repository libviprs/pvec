//! Structural exactness (issue P2.5): parameters compared field by field, but only where two
//! representations are meaningfully equivalent.
//!
//! Comparing a reparameterised spline field by field manufactures false failures, so an equivalence
//! precondition runs first. When it fails the fixture is routed to the geometric gate
//! ([`crate::geometric`]) as reparameterisation-eligible, and it is neither exact nor failed here.
//! Every comparison goes through [`pvec_numeric::Policy`]; this module writes no tolerance.

use crate::evidence::{Assertion, may_assert};
use pvec_capabilities::EvidenceClass;
use pvec_corpus::expected::Node;
use pvec_numeric::Policy;

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

fn scalar(node: Option<&Node>, what: &str) -> Result<f64, String> {
    node.and_then(Node::as_scalar).map(|s| s.value()).ok_or_else(|| format!("{what} is not a scalar"))
}

fn flag(node: &Node, key: &str) -> Result<bool, String> {
    match node.get("closure").and_then(|c| c.get(key)) {
        Some(Node::Bool(b)) => Ok(*b),
        _ => Err(format!("closure.{key} is missing")),
    }
}

impl Structure {
    /// Reads a spline entity (a Bezier curve, a B-spline or a NURBS) of an authored record or of the
    /// evaluator's record, which share their field names.
    pub fn from_entity(entity: &Node) -> Result<Structure, String> {
        let family = match entity.get("class") {
            Some(Node::Str(c)) if c == "bezier" => Family::Bezier,
            Some(Node::Str(c)) if c == "bspline" => Family::BSpline,
            Some(Node::Str(c)) if c == "nurbs" => Family::Nurbs,
            other => return Err(format!("{other:?} is not a spline class")),
        };
        let geometry = entity.get("geometry").ok_or("geometry is missing")?;
        let degree = match geometry.get("degree") {
            Some(Node::Int(d)) if *d >= 1 => *d as usize,
            _ => return Err("degree is missing".to_string()),
        };
        let mut control_points = Vec::new();
        for (i, p) in geometry
            .get("control_points")
            .and_then(Node::as_list)
            .ok_or("control_points is missing")?
            .iter()
            .enumerate()
        {
            let c = p
                .as_list()
                .filter(|c| c.len() == 3)
                .ok_or_else(|| format!("control_points[{i}] is not a point"))?;
            let mut xyz = [0.0; 3];
            for (k, v) in c.iter().enumerate() {
                xyz[k] = scalar(Some(v), &format!("control_points[{i}][{k}]"))?;
            }
            control_points.push(xyz);
        }
        let mut knots = Vec::new();
        if family != Family::Bezier {
            for (i, k) in
                geometry.get("knots").and_then(Node::as_list).ok_or("knots is missing")?.iter().enumerate()
            {
                let multiplicity = match k.get("multiplicity") {
                    Some(Node::Int(m)) if *m >= 1 => *m as usize,
                    _ => return Err(format!("knots[{i}].multiplicity is missing")),
                };
                knots.push(Knot {
                    value: scalar(k.get("value"), &format!("knots[{i}].value"))?,
                    multiplicity,
                });
            }
        }
        let weights = if family == Family::Nurbs {
            let list = geometry.get("weights").and_then(Node::as_list).ok_or("weights is missing")?;
            Some(
                list.iter()
                    .enumerate()
                    .map(|(i, w)| scalar(Some(w), &format!("weights[{i}]")))
                    .collect::<Result<Vec<_>, _>>()?,
            )
        } else {
            None
        };
        let domain = entity.get("parameter_domain").ok_or("parameter_domain is missing")?;
        let reversed = matches!(
            entity.get("orientation").and_then(|o| o.get("parameter_direction")),
            Some(Node::Str(d)) if d == "REVERSED_AGAINST_SOURCE"
        );
        Ok(Structure {
            family,
            degree,
            control_points,
            knots,
            weights,
            periodic: flag(entity, "periodic")?,
            closed_by_endpoint: flag(entity, "closed_by_endpoint")?,
            closed_by_knots: flag(entity, "closed_by_knots")?,
            domain: [
                scalar(domain.get("start"), "parameter_domain.start")?,
                scalar(domain.get("end"), "parameter_domain.end")?,
            ],
            reversed,
        })
    }

    fn expanded_knots(&self) -> usize {
        self.knots.iter().map(|k| k.multiplicity).sum()
    }

    /// The size of the feature: the longest side of the control polygon's box.
    fn extent(&self) -> f64 {
        (0..3)
            .map(|axis| {
                let (lo, hi) =
                    self.control_points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                        (lo.min(p[axis]), hi.max(p[axis]))
                    });
                if hi >= lo { hi - lo } else { 0.0 }
            })
            .fold(0.0, f64::max)
    }

    /// The weights divided by the largest one, so a common factor drops out.
    fn normalised_weights(&self) -> Option<Vec<f64>> {
        self.weights.as_ref().map(|w| {
            let max = w.iter().copied().fold(0.0, f64::max);
            w.iter().map(|x| x / max).collect()
        })
    }
}

fn assertion_for(class: EvidenceClass) -> Option<Assertion> {
    [Assertion::SourceIntent, Assertion::ConverterAgreement, Assertion::RegressionStability]
        .into_iter()
        .find(|a| may_assert(class, *a))
}

/// Why the two cannot be compared field by field, evaluated before any field is looked at.
fn precondition(expected: &Structure, actual: &Structure) -> Vec<String> {
    let policy = Policy::V1;
    let mut reasons = Vec::new();
    if expected.family != actual.family {
        reasons.push(format!("representation family differs ({:?} vs {:?})", expected.family, actual.family));
    }
    let width = expected.domain[1] - expected.domain[0];
    if !policy.near(expected.domain[0], actual.domain[0], width)
        || !policy.near(expected.domain[1], actual.domain[1], width)
    {
        reasons.push(format!(
            "knot-vector normalisation differs (domain {:?} vs {:?}), so the knots are not the same numbers",
            expected.domain, actual.domain
        ));
    }
    let scalable = |s: &Structure| {
        s.family != Family::Nurbs || s.weights.as_ref().is_some_and(|w| policy.validate_weights(w).is_ok())
    };
    if !scalable(expected) || !scalable(actual) {
        reasons.push(
            "weight scaling cannot be normalised (a weight is missing, not positive, or below the floor)"
                .to_string(),
        );
    }
    if expected.reversed != actual.reversed {
        reasons.push("orientation differs (one parameter runs against the source direction)".to_string());
    }
    reasons
}

/// Compares `actual` with the reference `expected` for a fixture of the given class. The precondition
/// runs first; a fixture that fails it is routed, not failed.
pub fn compare(fixture: &str, class: EvidenceClass, expected: &Structure, actual: &Structure) -> Report {
    let mut report = Report {
        fixture: fixture.to_string(),
        class,
        assertion: assertion_for(class),
        result: Structural::Exact,
        reasons: precondition(expected, actual),
        rows: Vec::new(),
    };
    if !report.reasons.is_empty() {
        report.result = Structural::RoutedToGeometric;
        return report;
    }
    let policy = Policy::V1;
    let size = expected.extent();
    let width = expected.domain[1] - expected.domain[0];
    let mut row = |field: String, e: String, a: String, rule: &'static str, equal: bool| {
        report.rows.push(Row {
            field,
            expected: e,
            actual: a,
            rule,
            equal,
            degree: expected.degree,
            knot_count: expected.expanded_knots(),
            control_point_count: expected.control_points.len(),
        });
    };
    row(
        "degree".into(),
        expected.degree.to_string(),
        actual.degree.to_string(),
        "exact",
        expected.degree == actual.degree,
    );

    let (ne, na) = (expected.control_points.len(), actual.control_points.len());
    row("control_points.count".into(), ne.to_string(), na.to_string(), "exact", ne == na);
    for i in 0..ne.max(na) {
        let (e, a) = (expected.control_points.get(i), actual.control_points.get(i));
        let equal = matches!((e, a), (Some(e), Some(a)) if (0..3).all(|k| policy.near(e[k], a[k], size)));
        row(format!("control_points[{i}]"), format!("{e:?}"), format!("{a:?}"), "NP-05", equal);
    }

    let (ne, na) = (expected.knots.len(), actual.knots.len());
    row("knots.count".into(), ne.to_string(), na.to_string(), "NP-14", ne == na);
    for i in 0..ne.max(na) {
        let (e, a) = (expected.knots.get(i), actual.knots.get(i));
        row(
            format!("knots[{i}].value"),
            format!("{:?}", e.map(|k| k.value)),
            format!("{:?}", a.map(|k| k.value)),
            "NP-14",
            matches!((e, a), (Some(e), Some(a)) if policy.near(e.value, a.value, width)),
        );
        row(
            format!("knots[{i}].multiplicity"),
            format!("{:?}", e.map(|k| k.multiplicity)),
            format!("{:?}", a.map(|k| k.multiplicity)),
            "NP-14",
            matches!((e, a), (Some(e), Some(a)) if e.multiplicity == a.multiplicity),
        );
    }

    if let (Some(we), Some(wa)) = (expected.normalised_weights(), actual.normalised_weights()) {
        row(
            "weights.count".into(),
            we.len().to_string(),
            wa.len().to_string(),
            "NP-15",
            we.len() == wa.len(),
        );
        for i in 0..we.len().max(wa.len()) {
            let (e, a) = (we.get(i), wa.get(i));
            row(
                format!("weights[{i}]"),
                format!("{e:?}"),
                format!("{a:?}"),
                "NP-15",
                matches!((e, a), (Some(e), Some(a)) if policy.near(*e, *a, 1.0)),
            );
        }
    }

    for (name, e, a, rule) in [
        ("periodic", expected.periodic, actual.periodic, "exact"),
        ("closed_by_endpoint", expected.closed_by_endpoint, actual.closed_by_endpoint, "NP-20"),
        ("closed_by_knots", expected.closed_by_knots, actual.closed_by_knots, "NP-20"),
    ] {
        row(name.into(), e.to_string(), a.to_string(), rule, e == a);
    }
    for (name, i) in [("domain.start", 0), ("domain.end", 1)] {
        row(
            name.into(),
            expected.domain[i].to_string(),
            actual.domain[i].to_string(),
            "NP-13",
            policy.near(expected.domain[i], actual.domain[i], width),
        );
    }
    report.result = if report.rows.iter().all(|r| r.equal) { Structural::Exact } else { Structural::Differs };
    report
}

/// The report text, one line per row, or one line saying where the fixture was routed.
pub fn render(report: &Report) -> String {
    let class = report.class.name();
    if report.result == Structural::RoutedToGeometric {
        return format!(
            "{} [{class}] structural: routed to the geometric gate as reparameterisation-eligible, not a structural failure ({})",
            report.fixture,
            report.reasons.join("; ")
        );
    }
    let meaning = match report.assertion {
        Some(Assertion::SourceIntent) => "source intent",
        Some(Assertion::ConverterAgreement) => "converter agreement",
        Some(Assertion::RegressionStability) => "regression stability",
        None => "not counted",
    };
    report
        .rows
        .iter()
        .map(|r| {
            format!(
                "{} [{class}] structural {}: {} expected={} actual={} rule={} degree={} knots={} control_points={} ({meaning})",
                report.fixture,
                r.field,
                if r.equal { "PASS" } else { "FAIL" },
                r.expected,
                r.actual,
                r.rule,
                r.degree,
                r.knot_count,
                r.control_point_count
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
