//! The round-trip comparison harness (issue P6.3, pvec#20).
//!
//! An independent evaluator (OpenCASCADE, through `tools/step-eval`, in a container) reads the exported
//! Part 21 file and reports what it found: one `EDGE` line per edge with the parameters of its curve, and
//! `SAMPLE` lines with the points and tangents it evaluates where it was asked to. This module compares that
//! report with the curves that were exported. It shares nothing with the writer: not its mapping table, not
//! its number formatting, not its reader.
//!
//! The comparison keeps three conformance classes apart and never merges them into one verdict:
//!
//! * structural: the parameters that are directly comparable (degree, control points, knots with their
//!   multiplicities, weights, radii, placements, trim domains);
//! * geometric: the locus, at deterministic samples across the domain, at every interior knot and a step either
//!   side of it, at the endpoints, with the tangent and, where there is a closed form, the arc length;
//! * display: only for an entity the export approximated, against the error bound recorded for it. A bounded
//!   approximation is never counted as an exactness pass.
//!
//! Every disagreement is a [`Disagreement`] with the entity, the PVec parameters, the evaluator's parameters,
//! the deviating sample, the tolerance that was applied and the class that failed. No tolerance is declared
//! here: every threshold is taken from `pvec_numeric::Policy::V1`.

use crate::Fidelity;
use core::f64::consts::TAU;
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, Side, Trim};
use pvec_numeric::Policy;
use std::collections::BTreeMap;

/// Which conformance class a disagreement is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Parameters that are directly comparable.
    Structural,
    /// The locus: samples, tangents and arc length.
    Geometric,
    /// Display fidelity against the recorded error bound.
    Display,
}

/// One disagreement between PVec and the evaluator.
#[derive(Clone, Debug, PartialEq)]
pub struct Disagreement {
    /// The entity.
    pub entity: EntityId,
    /// The class that failed.
    pub class: Class,
    /// PVec's parameters, as `name=values`.
    pub pvec: String,
    /// The evaluator's parameters, as `name=values`.
    pub step: String,
    /// The deviating sample (edge, parameter, both points), or the edge for a structural disagreement.
    pub sample: String,
    /// The tolerance that was applied.
    pub tolerance: f64,
}

/// An entity under test: what was exported, and what the export claimed for it.
#[derive(Clone, Debug)]
pub struct Subject<'a> {
    /// The entity id.
    pub id: EntityId,
    /// The curve that was written (after any transform).
    pub curve: &'a Curve,
    /// What the export claimed for it.
    pub fidelity: Fidelity,
}

struct Edge {
    kind: String,
    fields: BTreeMap<String, Vec<f64>>,
}

struct Sample {
    u: f64,
    values: [f64; 4],
}

/// What the evaluator printed.
pub struct Report {
    evaluator: String,
    edges: Vec<Edge>,
    samples: BTreeMap<usize, Vec<Sample>>,
}

fn floats(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|x| x.parse::<f64>().map_err(|e| format!("`{x}`: {e}"))).collect()
}

impl Report {
    /// Parses the evaluator's output. The first line must be the `EVALUATOR` line, and the number of `EDGE` lines
    /// must be the number it announces.
    pub fn parse(text: &str) -> Result<Report, String> {
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let head = lines.next().ok_or("the evaluator printed nothing")?;
        let evaluator = head.strip_prefix("EVALUATOR ").ok_or_else(|| format!("no EVALUATOR line: `{head}`"))?;
        let announced: usize = evaluator
            .split(' ')
            .find_map(|w| w.strip_prefix("edges="))
            .ok_or("the EVALUATOR line has no edges=")?
            .parse()
            .map_err(|e| format!("edges=: {e}"))?;
        let mut report = Report { evaluator: evaluator.to_string(), edges: Vec::new(), samples: BTreeMap::new() };
        for line in lines {
            let mut w = line.split(' ');
            match w.next() {
                Some("EDGE") => {
                    let index: usize = w.next().and_then(|x| x.parse().ok()).ok_or("EDGE without an index")?;
                    if index != report.edges.len() {
                        return Err(format!("EDGE {index} out of order"));
                    }
                    let kind = w.next().ok_or("EDGE without a kind")?.to_string();
                    let mut fields = BTreeMap::new();
                    for kv in w {
                        let (k, v) = kv.split_once('=').ok_or_else(|| format!("`{kv}` is not key=values"))?;
                        fields.insert(k.to_string(), floats(v)?);
                    }
                    report.edges.push(Edge { kind, fields });
                }
                Some("SAMPLE") => {
                    let e: usize = w.next().and_then(|x| x.parse().ok()).ok_or("SAMPLE without an edge")?;
                    let u: f64 = w.next().and_then(|x| x.parse().ok()).ok_or("SAMPLE without u")?;
                    let v = floats(w.next().ok_or("SAMPLE without values")?)?;
                    let values: [f64; 4] = v.try_into().map_err(|_| "a SAMPLE has four values")?;
                    report.samples.entry(e).or_default().push(Sample { u, values });
                }
                _ => return Err(format!("unexpected line `{line}`")),
            }
        }
        if report.edges.len() != announced {
            return Err(format!("the evaluator announced {announced} edges and printed {}", report.edges.len()));
        }
        Ok(report)
    }

    /// The evaluator line: its versions and the edge count.
    pub fn evaluator(&self) -> &str {
        &self.evaluator
    }

    /// The number of edges the evaluator found.
    pub fn edges(&self) -> usize {
        self.edges.len()
    }
}

/// One edge's worth of PVec curve: the piece the evaluator sees as a single edge.
struct Leaf<'a> {
    /// The piece, trim included, evaluated by PVec.
    curve: &'a Curve,
    /// The curve the piece is a trim of, or the piece itself.
    basis: &'a Curve,
    trim: Option<Trim>,
}

fn leaves<'a>(c: &'a Curve, out: &mut Vec<Leaf<'a>>) {
    match c {
        Curve::Composite(k) => k.segments().iter().for_each(|s| leaves(s, out)),
        Curve::Trimmed(t) => out.push(Leaf { curve: c, basis: t.basis(), trim: Some(t.trim()) }),
        _ => out.push(Leaf { curve: c, basis: c, trim: None }),
    }
}

fn claims_edges(f: &Fidelity) -> bool {
    !matches!(f, Fidelity::Unsupported(_))
}

fn plan<'a>(subjects: &'a [Subject<'a>]) -> Vec<(&'a Subject<'a>, Vec<Leaf<'a>>)> {
    subjects
        .iter()
        .map(|s| {
            let mut l = Vec::new();
            if claims_edges(&s.fidelity) {
                leaves(s.curve, &mut l);
            }
            (s, l)
        })
        .collect()
}

fn check_edges(report: &Report, planned: &[(&Subject<'_>, Vec<Leaf<'_>>)]) -> Result<(), String> {
    let expected: usize = planned.iter().map(|(_, l)| l.len()).sum();
    if expected != report.edges.len() {
        return Err(format!(
            "the entities written make {expected} edges and the evaluator found {}: an edge was merged, split or dropped",
            report.edges.len()
        ));
    }
    Ok(())
}

/// The fractions of the domain that are sampled: the eight spans, then each interior knot with a step either
/// side of it. The step is the numeric policy's tolerance at unit scale.
fn fractions(leaf: &Leaf<'_>) -> Vec<f64> {
    let step = Policy::V1.tol(1.0, 1.0);
    let mut f: Vec<f64> = (0..=8).map(|i| f64::from(i) / 8.0).collect();
    let d = leaf.curve.domain();
    for k in interior_knots(leaf) {
        let x = (k - d.lo()) / d.width();
        f.extend([x - step, x, x + step]);
    }
    f.retain(|x| (0.0..=1.0).contains(x));
    f
}

/// Interior knot positions in the parameter of `leaf.curve`: spline knots (shifted into the trim) and
/// polyline vertices.
fn interior_knots(leaf: &Leaf<'_>) -> Vec<f64> {
    let (lo, hi, shift) = match leaf.trim {
        Some(t) => (t.start, t.start + t.span, if t.reversed { None } else { Some(t.start) }),
        None => (leaf.curve.domain().lo(), leaf.curve.domain().hi(), Some(0.0)),
    };
    let Some(shift) = shift else { return Vec::new() };
    let all: Vec<f64> = match leaf.basis {
        Curve::BSpline(b) => b.knots().to_vec(),
        Curve::RationalBSpline(r) => r.knots().to_vec(),
        Curve::Bezier(_) => Vec::new(),
        Curve::Polyline(p) => (0..p.vertices().len()).map(|i| i as f64).collect(),
        _ => Vec::new(),
    };
    let mut out: Vec<f64> = all.into_iter().filter(|k| *k > lo && *k < hi).map(|k| k - shift).collect();
    out.dedup();
    out
}

/// The sample requests for the second evaluator run: `{"<edge>":[u,...]}`, with `u` in the evaluator's own
/// parameter range for that edge. An unbounded line is not sampled.
pub fn requests(subjects: &[Subject<'_>], report: &Report) -> Result<String, String> {
    let planned = plan(subjects);
    check_edges(report, &planned)?;
    let mut edge = 0;
    let mut parts = Vec::new();
    for (_, ls) in &planned {
        for leaf in ls {
            let e = &report.edges[edge];
            if sampled(leaf) {
                let (a, b) = (e.fields.get("first"), e.fields.get("last"));
                let (Some(a), Some(b)) = (a.and_then(|v| v.first()), b.and_then(|v| v.first())) else {
                    return Err(format!("edge {edge} has no parameter range"));
                };
                let us: Vec<String> = fractions(leaf).iter().map(|f| format!("{:?}", a + f * (b - a))).collect();
                parts.push(format!("\"{edge}\":[{}]", us.join(",")));
            }
            edge += 1;
        }
    }
    Ok(format!("{{{}}}", parts.join(",")))
}

fn sampled(leaf: &Leaf<'_>) -> bool {
    !(leaf.trim.is_none() && matches!(leaf.basis, Curve::Line(_)))
}

/// The outcome, with the structural, geometric and display results kept apart.
#[derive(Clone, Debug, Default)]
pub struct Verdict {
    /// Structural disagreements.
    pub structural: Vec<Disagreement>,
    /// Geometric disagreements.
    pub geometric: Vec<Disagreement>,
    /// Display disagreements, for entities the export approximated.
    pub display: Vec<Disagreement>,
    /// Entities claimed exact that agreed in every class. A bounded approximation is never counted here.
    pub exact_passes: usize,
    /// Structural parameter comparisons made.
    pub structural_checked: usize,
    /// Entities with an edge whose parameters are not directly comparable, with the reason.
    pub structural_not_comparable: Vec<(EntityId, &'static str)>,
    /// Geometric samples compared (points, tangents and lengths).
    pub geometric_checked: usize,
    /// Display samples compared.
    pub display_checked: usize,
}

impl Verdict {
    /// No disagreement in any class.
    pub fn passed(&self) -> bool {
        self.structural.is_empty() && self.geometric.is_empty() && self.display.is_empty()
    }
}

fn list(v: &[f64]) -> String {
    v.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(",")
}

struct Ctx<'v> {
    id: EntityId,
    edge: usize,
    scale: f64,
    verdict: &'v mut Verdict,
}

impl Ctx<'_> {
    /// Compares one named parameter list. A different length is a disagreement as well.
    fn param(&mut self, label: &str, pvec: &[f64], step: Option<&Vec<f64>>) {
        let policy = Policy::V1;
        self.verdict.structural_checked += 1;
        let mag = pvec.iter().chain(step.into_iter().flatten()).fold(0.0_f64, |m, x| m.max(x.abs()));
        let ok = step.is_some_and(|s| s.len() == pvec.len() && pvec.iter().zip(s).all(|(a, b)| policy.near(*a, *b, self.scale)));
        if !ok {
            self.verdict.structural.push(Disagreement {
                entity: self.id,
                class: Class::Structural,
                pvec: format!("{label}={}", list(pvec)),
                step: format!("{label}={}", step.map_or("missing".to_string(), |s| list(s))),
                sample: format!("edge={}", self.edge),
                tolerance: policy.tol(self.scale, mag),
            });
        }
    }
}

fn scale_of(c: &Curve) -> f64 {
    c.bounds().map_or(1.0, |b| b.diagonal().max(b.magnitude()))
}

/// Distinct knot values and their multiplicities, the form the evaluator reports.
fn distinct(knots: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let (mut values, mut mults): (Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new());
    for k in knots {
        if values.last().is_some_and(|v| v.to_bits() == k.to_bits()) {
            *mults.last_mut().expect("a multiplicity per value") += 1.0;
        } else {
            values.push(*k);
            mults.push(1.0);
        }
    }
    (values, mults)
}

fn xy(points: &[pvec_geometry::Point2]) -> Vec<f64> {
    points.iter().flat_map(|p| [p.x, p.y]).collect()
}

/// Structural comparison of one edge. Returns why it was skipped when the parameters are not directly comparable.
fn structural(cx: &mut Ctx<'_>, leaf: &Leaf<'_>, e: &Edge) -> Option<&'static str> {
    let reversed = leaf.trim.is_some_and(|t| t.reversed);
    if reversed {
        return Some("the evaluator re-parameterises a trim that runs backwards");
    }
    let want = |kind: &str, cx: &mut Ctx<'_>| {
        if e.kind != kind {
            cx.verdict.structural_checked += 1;
            cx.verdict.structural.push(Disagreement {
                entity: cx.id,
                class: Class::Structural,
                pvec: format!("kind={kind}"),
                step: format!("kind={}", e.kind),
                sample: format!("edge={}", cx.edge),
                tolerance: 0.0,
            });
            false
        } else {
            true
        }
    };
    let f = |k: &str| e.fields.get(k);
    // The range of the basis curve's parameter that the edge covers.
    let range = |natural: (f64, f64)| match leaf.trim {
        Some(t) => [t.start, t.start + t.span],
        None => [natural.0, natural.1],
    };
    match leaf.basis {
        Curve::Line(l) => {
            if !want("Line", cx) {
                return None;
            }
            cx.param("location", &[l.origin().x, l.origin().y], f("location"));
            cx.param("direction", &[l.direction().x, l.direction().y], f("direction"));
            if let Some(t) = leaf.trim {
                cx.param("range", &[t.start, t.start + t.span], range_of(e).as_ref());
            }
        }
        Curve::Circle(c) => {
            if !want("Circle", cx) {
                return None;
            }
            let p = c.placement();
            cx.param("center", &[p.origin().x, p.origin().y], f("center"));
            cx.param("radius", &[c.radius()], f("radius"));
            cx.param("xdir", &[p.x_axis().x, p.x_axis().y], f("xdir"));
            cx.param("range", &range((0.0, TAU)), range_of(e).as_ref());
        }
        Curve::Ellipse(el) => {
            if el.semi_axis_1() < el.semi_axis_2() {
                return Some("the evaluator names the longer semi-axis the major one");
            }
            if !want("Ellipse", cx) {
                return None;
            }
            let p = el.placement();
            cx.param("center", &[p.origin().x, p.origin().y], f("center"));
            cx.param("major", &[el.semi_axis_1()], f("major"));
            cx.param("minor", &[el.semi_axis_2()], f("minor"));
            cx.param("xdir", &[p.x_axis().x, p.x_axis().y], f("xdir"));
            cx.param("range", &range((0.0, TAU)), range_of(e).as_ref());
        }
        Curve::Polyline(p) => {
            if !want("BSplineCurve", cx) {
                return None;
            }
            let n = p.vertices().len();
            let knots: Vec<f64> = (0..n).map(|i| i as f64).collect();
            let mut mults = vec![1.0; n];
            mults[0] = 2.0;
            mults[n - 1] = 2.0;
            cx.param("degree", &[1.0], f("degree"));
            cx.param("poles", &xy(p.vertices()), f("poles"));
            cx.param("knots", &knots, f("knots"));
            cx.param("multiplicities", &mults, f("mults"));
        }
        Curve::Bezier(b) => {
            if !want("BSplineCurve", cx) {
                return None;
            }
            let d = b.degree();
            cx.param("degree", &[d as f64], f("degree"));
            cx.param("poles", &xy(b.control_points()), f("poles"));
            cx.param("knots", &[0.0, 1.0], f("knots"));
            cx.param("multiplicities", &[(d + 1) as f64; 2], f("mults"));
            cx.param("weights", &vec![1.0; b.control_points().len()], f("weights"));
            cx.param("range", &range((0.0, 1.0)), range_of(e).as_ref());
        }
        Curve::BSpline(b) => {
            if b.is_declared_closed() {
                return Some("the evaluator stores a closed spline as a periodic one");
            }
            if !want("BSplineCurve", cx) {
                return None;
            }
            let (values, mults) = distinct(b.knots());
            cx.param("degree", &[b.degree() as f64], f("degree"));
            cx.param("poles", &xy(b.control_points()), f("poles"));
            cx.param("knots", &values, f("knots"));
            cx.param("multiplicities", &mults, f("mults"));
            cx.param("weights", &vec![1.0; b.control_points().len()], f("weights"));
            cx.param("range", &range((values[0], values[values.len() - 1])), range_of(e).as_ref());
        }
        Curve::RationalBSpline(r) => {
            if r.is_declared_closed() {
                return Some("the evaluator stores a closed spline as a periodic one");
            }
            if !want("BSplineCurve", cx) {
                return None;
            }
            let (values, mults) = distinct(r.knots());
            cx.param("degree", &[r.degree() as f64], f("degree"));
            cx.param("poles", &xy(r.control_points()), f("poles"));
            cx.param("knots", &values, f("knots"));
            cx.param("multiplicities", &mults, f("mults"));
            cx.param("weights", r.weights(), f("weights"));
            cx.param("range", &range((values[0], values[values.len() - 1])), range_of(e).as_ref());
        }
        _ => return Some("a curve kind that has no edge of its own"),
    }
    None
}

fn range_of(e: &Edge) -> Option<Vec<f64>> {
    Some(vec![*e.fields.get("first")?.first()?, *e.fields.get("last")?.first()?])
}

/// Arc length where the curve has a closed form.
fn closed_form_length(leaf: &Leaf<'_>) -> Option<f64> {
    match (leaf.basis, leaf.trim) {
        (Curve::Circle(c), Some(t)) => Some(c.radius() * t.span),
        (Curve::Circle(c), None) => Some(c.radius() * TAU),
        (Curve::Line(l), Some(t)) => Some(t.span * l.direction().x.hypot(l.direction().y)),
        (Curve::Polyline(p), None) => Some(p.vertices().windows(2).map(|w| (w[1].x - w[0].x).hypot(w[1].y - w[0].y)).sum()),
        _ => None,
    }
}

fn unit(v: [f64; 2]) -> [f64; 2] {
    let n = v[0].hypot(v[1]);
    [v[0] / n, v[1] / n]
}

/// Compares the samples of one edge. `bound` is the recorded error bound of an approximated entity, which
/// turns the comparison into a display one.
fn geometric(cx: &mut Ctx<'_>, leaf: &Leaf<'_>, e: &Edge, samples: &[Sample], bound: Option<f64>) {
    let policy = Policy::V1;
    let class = if bound.is_some() { Class::Display } else { Class::Geometric };
    let d = leaf.curve.domain();
    let (Some(first), Some(last)) = (e.fields.get("first").and_then(|v| v.first()), e.fields.get("last").and_then(|v| v.first())) else {
        return;
    };
    let step = policy.tol(1.0, 1.0);
    let knots: Vec<f64> = interior_knots(leaf).iter().map(|k| (k - d.lo()) / d.width()).collect();
    let mut fail = |cx: &mut Ctx<'_>, what: String, pv: String, st: String, tol: f64| {
        let v = Disagreement { entity: cx.id, class, pvec: pv, step: st, sample: format!("edge={} {what}", cx.edge), tolerance: tol };
        match class {
            Class::Display => cx.verdict.display.push(v),
            _ => cx.verdict.geometric.push(v),
        }
    };
    let count = |cx: &mut Ctx<'_>| match class {
        Class::Display => cx.verdict.display_checked += 1,
        _ => cx.verdict.geometric_checked += 1,
    };
    for s in samples {
        let f = (s.u - first) / (last - first);
        let t = d.lo() + f * d.width();
        let Ok(p) = leaf.curve.eval(t) else {
            fail(cx, format!("u={:?}", s.u), "eval failed".into(), format!("{:?}", &s.values[..2]), 0.0);
            continue;
        };
        count(cx);
        let tol = bound.unwrap_or_else(|| policy.tol(cx.scale, p.x.abs().max(p.y.abs())));
        let off = (p.x - s.values[0]).hypot(p.y - s.values[1]);
        let near = match bound {
            Some(b) => off <= b,
            None => policy.near_point([p.x, p.y], [s.values[0], s.values[1]], cx.scale),
        };
        if !near {
            let tag = format!("u={:?} f={f:?} off={off:?}", s.u);
            fail(cx, tag, format!("point={:?},{:?}", p.x, p.y), format!("point={:?},{:?}", s.values[0], s.values[1]), tol);
        }
        if bound.is_none() && !knots.iter().any(|k| (f - k).abs() <= step / 2.0) {
            let side = if f < 1.0 { Side::Right } else { Side::Left };
            if let Ok(dv) = leaf.curve.derivative(t, 1, side) {
                let want = unit([dv.x, dv.y]);
                count(cx);
                if !policy.near_point(want, [s.values[2], s.values[3]], 1.0) {
                    fail(cx, format!("u={:?} f={f:?} tangent", s.u), format!("tangent={:?},{:?}", want[0], want[1]), format!("tangent={:?},{:?}", s.values[2], s.values[3]), policy.tol(1.0, 1.0));
                }
            }
        }
    }
    if bound.is_none() {
        if let (Some(len), Some(got)) = (closed_form_length(leaf), e.fields.get("length").and_then(|v| v.first())) {
            count(cx);
            if !policy.near(len, *got, len) {
                fail(cx, "arc length".into(), format!("length={len:?}"), format!("length={got:?}"), policy.tol(len, len));
            }
        }
    }
}

/// Compares the exported curves with what the evaluator found, edge by edge, in the order they were written.
/// The report must carry the samples requested by [`requests`] for the same subjects.
pub fn compare(subjects: &[Subject<'_>], report: &Report) -> Result<Verdict, String> {
    let planned = plan(subjects);
    check_edges(report, &planned)?;
    let mut verdict = Verdict::default();
    let mut edge = 0;
    for (subject, ls) in &planned {
        let before = (verdict.structural.len(), verdict.geometric.len(), verdict.display.len());
        let mut checked = 0;
        for leaf in ls {
            let e = &report.edges[edge];
            let mut cx = Ctx { id: subject.id, edge, scale: scale_of(leaf.curve), verdict: &mut verdict };
            match &subject.fidelity {
                Fidelity::Exact => {
                    if let Some(why) = structural(&mut cx, leaf, e) {
                        cx.verdict.structural_not_comparable.push((subject.id, why));
                    }
                    if sampled(leaf) {
                        let s = report.samples.get(&edge).ok_or_else(|| format!("edge {edge} was not sampled"))?;
                        geometric(&mut cx, leaf, e, s, None);
                    }
                    checked += 1;
                }
                Fidelity::BoundedApproximation(b) => {
                    let s = report.samples.get(&edge).ok_or_else(|| format!("edge {edge} was not sampled"))?;
                    geometric(&mut cx, leaf, e, s, Some(b.get()));
                }
                // Not claimed exact in the structural class: the locus is still held to the geometric class.
                _ => {
                    if sampled(leaf) {
                        let s = report.samples.get(&edge).ok_or_else(|| format!("edge {edge} was not sampled"))?;
                        geometric(&mut cx, leaf, e, s, None);
                    }
                }
            }
            edge += 1;
        }
        let clean = before == (verdict.structural.len(), verdict.geometric.len(), verdict.display.len());
        if clean && checked > 0 && matches!(subject.fidelity, Fidelity::Exact) {
            verdict.exact_passes += 1;
        }
    }
    Ok(verdict)
}
