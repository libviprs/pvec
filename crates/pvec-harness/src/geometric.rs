//! Geometric equivalence (issue P2.6): two representations describe the same locus when an
//! independent evaluation of the reference and this repository's evaluation of the candidate agree
//! at seeded, refined, knot-adjacent and endpoint samples.
//!
//! The reference side is the independent STEP evaluator of EPIC O (O1.3), reached through
//! [`Evaluator`]; comparing this repository's evaluator with itself is refused. Every tolerance comes
//! from [`pvec_numeric::Policy`]: this module writes none. Structural exactness (the parameters
//! themselves) is a different gate, and display fidelity is not asserted here at all.

use crate::error::{ConformanceError, Result};
use crate::evidence::meaning;
use pvec_capabilities::EvidenceClass;
use pvec_geometry::Curve2d;
use pvec_numeric::{Policy, Side, check_derivative_request};

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

/// The epsilon around a knot, from the numeric policy: the NP-14 parameter tolerance of the domain.
pub fn knot_epsilon(domain: [f64; 2], at: f64) -> f64 {
    let mag = domain[0].abs().max(domain[1].abs()).max(at.abs());
    Policy::V1.tol(domain[1] - domain[0], mag)
}

fn span_and_mag(points: impl Iterator<Item = [f64; 3]>) -> (f64, f64) {
    let (mut lo, mut hi, mut mag) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3], 0.0f64);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
            mag = mag.max(p[k].abs());
        }
    }
    ((0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max), mag)
}

/// The position tolerance for a reference: the policy tolerance at the size of its box, taken from the
/// closed-form bounds when it has them and from a uniform look at the curve when it does not.
pub fn position_tolerance(reference: &dyn Evaluator) -> f64 {
    let (extent, mag) = match reference.bounds() {
        Some((lo, hi)) => span_and_mag([lo, hi].into_iter()),
        None => {
            let [a, b] = reference.domain();
            span_and_mag((0..=64).map(|i| reference.point(a + (b - a) * f64::from(i) / 64.0)))
        }
    };
    Policy::V1.tol(extent, mag)
}

/// splitmix64, so the sample set is the same on every platform and every run.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl Run {
    /// The sample set for a reference: both endpoints, seeded samples, and for each break the
    /// parameter, one epsilon below it and one epsilon above it.
    pub fn new(reference: &dyn Evaluator, plan: Plan) -> Run {
        let [lo, hi] = reference.domain();
        let mut state = plan.seed;
        let mut parameters = vec![lo, hi];
        for _ in 0..plan.samples {
            let unit = (splitmix(&mut state) >> 11) as f64 / (1u64 << 53) as f64;
            parameters.push(lo + (hi - lo) * unit);
        }
        for b in reference.breaks() {
            let eps = knot_epsilon([lo, hi], b.parameter);
            parameters.extend([b.parameter - eps, b.parameter, b.parameter + eps]);
        }
        Run { seed: plan.seed, parameters }
    }

    /// Fails when another run's parameters differ in any bit.
    pub fn same_as(&self, other: &Run) -> std::result::Result<(), String> {
        if self.seed != other.seed || self.parameters.len() != other.parameters.len() {
            return Err(format!(
                "the runs differ in shape (seed {} vs {}, {} vs {} samples)",
                self.seed,
                other.seed,
                self.parameters.len(),
                other.parameters.len()
            ));
        }
        match self.parameters.iter().zip(&other.parameters).position(|(a, b)| a.to_bits() != b.to_bits()) {
            Some(i) => Err(format!(
                "sample {i} differs between runs: {:?} vs {:?}",
                self.parameters[i], other.parameters[i]
            )),
            None => Ok(()),
        }
    }
}

/// The candidate's parameter for a reference parameter: the unit map of NP-13, so a candidate on a
/// different (affinely related) domain is read at the same point of the locus.
fn mapped(reference: &dyn Evaluator, candidate: &dyn Evaluator, t: f64) -> f64 {
    let ([rl, rh], [cl, ch]) = (reference.domain(), candidate.domain());
    if rl == cl && rh == ch { t } else { cl + (ch - cl) * ((t - rl) / (rh - rl)) }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let d = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt();
    if d.is_finite() { d } else { f64::INFINITY }
}

fn position_deviation(reference: &dyn Evaluator, candidate: &dyn Evaluator, t: f64) -> f64 {
    distance(reference.point(t), candidate.point(mapped(reference, candidate, t)))
}

/// Keeps the sample that is furthest out of tolerance, which is the sample worth naming.
struct Worst {
    max: f64,
    at: Option<f64>,
    tolerance: f64,
    ratio: f64,
}

impl Worst {
    fn new(tolerance: f64) -> Worst {
        Worst { max: 0.0, at: None, tolerance, ratio: -1.0 }
    }
    fn update(&mut self, deviation: f64, tolerance: f64, at: Option<f64>) {
        let ratio = deviation / tolerance;
        if ratio > self.ratio {
            *self = Worst { max: deviation, at, tolerance, ratio };
        }
    }
    fn metric(self, name: &'static str) -> Metric {
        Metric { name, max: self.max, at: self.at, tolerance: self.tolerance, within: self.ratio <= 1.0 }
    }
}

/// Uniform sampling with `budget` evaluations.
pub fn fixed_step(reference: &dyn Evaluator, candidate: &dyn Evaluator, budget: usize) -> Deviation {
    let [lo, hi] = reference.domain();
    let n = budget.max(2);
    let mut worst = Deviation { max: 0.0, at: Some(lo) };
    for i in 0..n {
        let t = lo + (hi - lo) * i as f64 / (n - 1) as f64;
        let d = position_deviation(reference, candidate, t);
        if d > worst.max {
            worst = Deviation { max: d, at: Some(t) };
        }
    }
    worst
}

/// Interval refinement with at most `budget` evaluations: half the budget uniform, then bisect the
/// intervals whose ends deviate by at least half the tolerance, the pair with the largest combined
/// deviation first.
pub fn adaptive(
    reference: &dyn Evaluator,
    candidate: &dyn Evaluator,
    budget: usize,
    tolerance: f64,
) -> Deviation {
    let [lo, hi] = reference.domain();
    let start = budget.div_ceil(2).max(2);
    let mut worst = Deviation { max: 0.0, at: Some(lo) };
    let see = |t: f64, worst: &mut Deviation| {
        let d = position_deviation(reference, candidate, t);
        if d > worst.max {
            *worst = Deviation { max: d, at: Some(t) };
        }
        d
    };
    // Each interval is (start, end, deviation at start, deviation at end).
    let mut intervals = Vec::new();
    let mut previous: Option<(f64, f64)> = None;
    for i in 0..start {
        let t = lo + (hi - lo) * i as f64 / (start - 1) as f64;
        let d = see(t, &mut worst);
        if let Some((pt, pd)) = previous {
            intervals.push((pt, t, pd, d));
        }
        previous = Some((t, d));
    }
    let mut used = start;
    while used < budget {
        let pick = intervals
            .iter()
            .enumerate()
            .filter(|(_, iv)| iv.2.max(iv.3) * 2.0 >= tolerance && iv.1 > iv.0)
            .fold(None, |best: Option<(usize, f64)>, (i, iv)| {
                let score = iv.2 + iv.3;
                if best.is_none_or(|(_, s)| score > s) { Some((i, score)) } else { best }
            });
        let Some((i, _)) = pick else { break };
        let (a, b, da, db) = intervals[i];
        let mid = a + (b - a) / 2.0;
        if mid <= a || mid >= b {
            break;
        }
        let dm = see(mid, &mut worst);
        used += 1;
        intervals[i] = (a, mid, da, dm);
        intervals.insert(i + 1, (mid, b, dm, db));
    }
    worst
}

/// A derivative request at a break. A two-sided request above the break's order is refused (NP-16).
pub fn request(order: usize, at: &Break, side: Side) -> std::result::Result<(), String> {
    // check_derivative_request takes the degree and the multiplicity; a break's two-sided order is
    // degree minus multiplicity, so a degree of that order and a multiplicity of zero says the same.
    check_derivative_request(order, at.two_sided_order, 0, side).map_err(|_| {
        format!(
            "a two-sided derivative of order {order} at {:?} does not exist (two-sided up to order {})",
            at.parameter, at.two_sided_order
        )
    })
}

/// What to ask for at a sample: `(order, side)` pairs that are legal there, so a break is read from
/// each side and everywhere else from both.
fn sides(t: f64, domain: [f64; 2], breaks: &[Break], order: usize) -> Vec<Side> {
    if t == domain[0] {
        return vec![Side::Right];
    }
    if t == domain[1] {
        return vec![Side::Left];
    }
    match breaks.iter().find(|b| b.parameter == t) {
        Some(b) if request(order, b, Side::Both).is_err() => vec![Side::Left, Side::Right],
        _ => vec![Side::Both],
    }
}

/// Compares a candidate with the independent reference.
pub fn compare(
    fixture: &str,
    class: EvidenceClass,
    reference: &dyn Evaluator,
    candidate: &dyn Evaluator,
    plan: Plan,
) -> Result<Report> {
    if reference.source() != Source::Independent || candidate.source() != Source::Own {
        return Err(ConformanceError::Geometric(
            "the reference has to be the independent evaluator and the candidate this repository's; \
             this repository's evaluator is never compared with itself"
                .to_string(),
        ));
    }
    let policy = Policy::V1;
    let domain = reference.domain();
    let breaks = reference.breaks();
    let run = Run::new(reference, plan);
    let position_tol = position_tolerance(reference);
    let (mut position, mut endpoints) = (Worst::new(position_tol), Worst::new(position_tol));
    let (mut tangent, mut curvature) = (Worst::new(policy.tol(1.0, 1.0)), Worst::new(policy.tol(1.0, 1.0)));
    for &t in &run.parameters {
        let d = position_deviation(reference, candidate, t);
        position.update(d, position_tol, Some(t));
        if t == domain[0] || t == domain[1] {
            endpoints.update(d, position_tol, Some(t));
        }
        let tc = mapped(reference, candidate, t);
        for side in sides(t, domain, &breaks, 1) {
            let deviation = match (reference.tangent(t, side), candidate.tangent(tc, side)) {
                (Some(a), Some(b)) => distance(a, b),
                (None, None) => continue,
                _ => f64::INFINITY,
            };
            tangent.update(deviation, policy.tol(1.0, 1.0), Some(t));
        }
        for side in sides(t, domain, &breaks, 2) {
            let (a, b) = (reference.curvature(t, side), candidate.curvature(tc, side));
            let (deviation, scale) = match (a, b) {
                (Some(a), Some(b)) if a.is_finite() && b.is_finite() => ((a - b).abs(), a.abs().max(b.abs())),
                (None, None) => continue,
                _ => (f64::INFINITY, 1.0),
            };
            curvature.update(deviation, policy.tol(scale, scale), Some(t));
        }
    }

    let fixed = fixed_step(reference, candidate, plan.budget);
    let refined = adaptive(reference, candidate, plan.budget, position_tol);
    position.update(refined.max, position_tol, refined.at);

    let mut metrics = vec![
        endpoints.metric("endpoints"),
        position.metric("position"),
        tangent.metric("tangent"),
        curvature.metric("curvature"),
    ];
    if let (Some((rl, rh)), Some((cl, ch))) = (reference.bounds(), candidate.bounds()) {
        let d = (0..3).map(|k| (rl[k] - cl[k]).abs().max((rh[k] - ch[k]).abs())).fold(0.0, f64::max);
        let mut w = Worst::new(position_tol);
        w.update(if d.is_finite() { d } else { f64::INFINITY }, position_tol, None);
        metrics.push(w.metric("bounds"));
    }
    let (lr, lc) = (reference.arc_length(), candidate.arc_length());
    let mut w = Worst::new(policy.tol(lr, lr));
    w.update(if lc.is_finite() { (lr - lc).abs() } else { f64::INFINITY }, policy.tol(lr, lr), None);
    metrics.push(w.metric("arc_length"));

    let passed = metrics.iter().all(|m| m.within);
    Ok(Report {
        fixture: fixture.to_string(),
        class,
        run,
        metrics,
        refinement: Refinement { budget: plan.budget, tolerance: position_tol, fixed, adaptive: refined },
        passed,
    })
}

fn at_text(at: Option<f64>) -> String {
    at.map_or("n/a".to_string(), |t| format!("{t:?}"))
}

/// The report text, one line per quantity and one for the refinement comparison.
pub fn render(report: &Report) -> String {
    let (id, class, words) = (&report.fixture, report.class.name(), meaning(report.class));
    let mut lines: Vec<String> = report
        .metrics
        .iter()
        .map(|m| {
            format!(
                "{id} [{class}] geometric {}: {} max_deviation={:?} at={} tolerance={:?} ({words}, seed {})",
                m.name,
                if m.within { "PASS" } else { "FAIL" },
                m.max,
                at_text(m.at),
                m.tolerance,
                report.run.seed
            )
        })
        .collect();
    let r = &report.refinement;
    lines.push(format!(
        "{id} [{class}] geometric refinement with a budget of {}: fixed-step max_deviation={:?} at={}, adaptive max_deviation={:?} at={}, tolerance={:?}",
        r.budget,
        r.fixed.max,
        at_text(r.fixed.at),
        r.adaptive.max,
        at_text(r.adaptive.at),
        r.tolerance
    ));
    lines.join("\n")
}

/// Refuses text that words a geometric result as structural exactness or as display fidelity.
pub fn check_wording(text: &str) -> std::result::Result<(), String> {
    let lower = text.to_lowercase();
    for phrase in ["structurally exact", "structural exact", "display fidelity", "pixel"] {
        if lower.contains(phrase) {
            return Err(format!(
                "`{phrase}`: a geometric result is neither structural exactness nor display fidelity"
            ));
        }
    }
    Ok(())
}

/// Gauss-Legendre nodes (positive half) and weights, eight points.
const GAUSS: [(f64, f64); 4] = [
    (0.1834346424956498, 0.362683783378362),
    (0.525532409916329, 0.3137066458778873),
    (0.7966664774136267, 0.2223810344533745),
    (0.9602898564975363, 0.1012285362903763),
];

/// This repository's evaluator as the candidate side: [`pvec_geometry::Curve2d`] curves in the
/// z = 0 plane. The curve does not know where its breaks are, so the caller says.
pub struct Candidate<'a, C> {
    /// The curve under test.
    pub curve: &'a C,
    /// Its interior breaks.
    pub breaks: Vec<Break>,
}

impl<C: Curve2d> Candidate<'_, C> {
    fn velocity(&self, t: f64, order: usize, side: Side) -> Option<[f64; 2]> {
        self.curve.derivative(t, order, side).ok().map(|v| [v.x, v.y])
    }
}

impl<C: Curve2d> Evaluator for Candidate<'_, C> {
    fn source(&self) -> Source {
        Source::Own
    }
    fn domain(&self) -> [f64; 2] {
        let d = self.curve.domain();
        [d.lo(), d.hi()]
    }
    fn breaks(&self) -> Vec<Break> {
        self.breaks.clone()
    }
    fn point(&self, t: f64) -> [f64; 3] {
        self.curve.eval(t).map_or([f64::NAN; 3], |p| [p.x, p.y, 0.0])
    }
    fn tangent(&self, t: f64, side: Side) -> Option<[f64; 3]> {
        let v = self.velocity(t, 1, side)?;
        let n = v[0].hypot(v[1]);
        (n > 0.0 && n.is_finite()).then(|| [v[0] / n, v[1] / n, 0.0])
    }
    fn curvature(&self, t: f64, side: Side) -> Option<f64> {
        let (v, a) = (self.velocity(t, 1, side)?, self.velocity(t, 2, side)?);
        let speed = v[0].hypot(v[1]);
        (speed > 0.0).then(|| (v[0] * a[1] - v[1] * a[0]) / speed.powi(3))
    }
    fn arc_length(&self) -> f64 {
        let [lo, hi] = self.domain();
        let mut cuts = vec![lo];
        cuts.extend(self.breaks.iter().map(|b| b.parameter));
        cuts.push(hi);
        let mut total = 0.0;
        for span in cuts.windows(2) {
            let parts = 64;
            let h = (span[1] - span[0]) / f64::from(parts);
            for p in 0..parts {
                let mid = span[0] + h * (f64::from(p) + 0.5);
                for (x, w) in GAUSS {
                    for sign in [-1.0, 1.0] {
                        let t = mid + sign * x * h / 2.0;
                        let speed = self.velocity(t, 1, Side::Both).map_or(f64::NAN, |v| v[0].hypot(v[1]));
                        total += w * speed * h / 2.0;
                    }
                }
            }
        }
        total
    }
    fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        use pvec_geometry::CurveKind::{Circle, Ellipse, Polyline};
        if !matches!(self.curve.kind(), Circle | Ellipse | Polyline) {
            return None;
        }
        self.curve.bounds().ok().map(|b| ([b.min().x, b.min().y, 0.0], [b.max().x, b.max().y, 0.0]))
    }
}
