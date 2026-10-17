//! Pathological probes that need an evaluator. They are data plus known truth, written before any
//! evaluator exists. `run_probe` runs one against any [`Evaluator`], so the same file is used by
//! the red suite here and by the geometry spike (P1.3), and later by the production crate (P3.1).
#![allow(missing_docs, dead_code)]

use core::f64::consts::PI;
use pvec_numeric::*;

/// A curve described as plain data. No serialization type appears here, on purpose.
#[derive(Clone, Debug)]
pub enum ProbeCurve {
    Line { origin: [f64; 2], dir: [f64; 2] },
    Circle { center: [f64; 2], radius: f64 },
    Trimmed { basis: Box<ProbeCurve>, t1: f64, t2: f64, sense: bool },
    BSpline { degree: usize, cps: Vec<[f64; 2]>, weights: Option<Vec<f64>>, knots: Vec<f64> },
}

/// What an evaluator builds from a [`ProbeCurve`].
pub trait Built {
    fn domain(&self) -> (f64, f64);
    fn eval(&self, t: f64) -> Result<[f64; 2], NumericError>;
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<[f64; 2], NumericError>;
}

/// Builds curves, validating them under the numeric policy.
pub trait Evaluator {
    fn build(&self, c: &ProbeCurve) -> Result<Box<dyn Built>, NumericError>;
}

pub enum Expect {
    /// The curve is refused at construction with this error.
    BuildRejects(NumericError),
    /// Evaluating at `t` gives `xy`, equal at feature scale `scale`.
    Point { t: f64, xy: [f64; 2], scale: f64 },
    /// Evaluation at `t` is refused with this error.
    EvalRejects { t: f64, err: NumericError },
    /// The derivative of `order` from `side` at `t` is `xy` at feature scale `scale`.
    Deriv { t: f64, order: usize, side: Side, xy: [f64; 2], scale: f64 },
    /// The derivative request is refused.
    DerivRejects { t: f64, order: usize, side: Side, err: NumericError },
    /// Every sample over the domain is finite and inside the control-hull box.
    InsideHull { samples: usize },
}

pub struct Probe {
    pub id: &'static str,
    pub rule: &'static str,
    pub curve: ProbeCurve,
    pub expect: Vec<Expect>,
}

const P: Policy = Policy::V1;

fn clamped_cubic_with_c0_break() -> ProbeCurve {
    ProbeCurve::BSpline {
        degree: 3,
        cps: vec![[0., 0.], [1., 2.], [2., 3.], [3., 0.], [4., -1.], [5., -2.], [6., 0.]],
        weights: None,
        knots: vec![0., 0., 0., 0., 0.5, 0.5, 0.5, 1., 1., 1., 1.],
    }
}

pub fn probes() -> Vec<Probe> {
    let r = P.abs * 1000.0;
    let m = MAX_ABS_COORD / 2.0;
    let rational_hull = vec![[0., 0.], [1., 1.], [3., 3.]];
    vec![
        Probe {
            id: "near-degenerate-arc",
            rule: "NP-07",
            curve: ProbeCurve::Trimmed {
                basis: Box::new(ProbeCurve::Circle { center: [0., 0.], radius: r }),
                t1: 0.0,
                t2: PI,
                sense: true,
            },
            expect: vec![Expect::Point { t: PI / 2.0, xy: [0.0, r], scale: r }],
        },
        Probe {
            id: "degenerate-radius-rejected",
            rule: "NP-07",
            curve: ProbeCurve::Circle { center: [0., 0.], radius: P.abs * 0.5 },
            expect: vec![Expect::BuildRejects(NumericError::DegenerateRadius)],
        },
        Probe {
            id: "collinear-nurbs-hull",
            rule: "NP-08",
            curve: ProbeCurve::BSpline {
                degree: 2,
                cps: rational_hull,
                weights: Some(vec![1., 2., 1.]),
                knots: vec![0., 0., 0., 1., 1., 1.],
            },
            // Rational quadratic at t = 1/2: weights (1,2,1), basis (1/4,1/2,1/4).
            expect: vec![
                Expect::Point { t: 0.5, xy: [7.0 / 6.0, 7.0 / 6.0], scale: 3.0 },
                Expect::InsideHull { samples: 64 },
            ],
        },
        Probe {
            id: "coincident-control-points-rejected",
            rule: "NP-08",
            curve: ProbeCurve::BSpline {
                degree: 2,
                cps: vec![[1., 1.], [1., 1.], [1., 1.]],
                weights: None,
                knots: vec![0., 0., 0., 1., 1., 1.],
            },
            expect: vec![Expect::BuildRejects(NumericError::DegenerateHull)],
        },
        Probe {
            id: "full-multiplicity-interior-knot-value",
            rule: "NP-14",
            curve: clamped_cubic_with_c0_break(),
            // The curve interpolates the control point at the break.
            expect: vec![Expect::Point { t: 0.5, xy: [3.0, 0.0], scale: 6.0 }],
        },
        Probe {
            id: "full-multiplicity-interior-knot-one-sided",
            rule: "NP-16",
            curve: clamped_cubic_with_c0_break(),
            // Each side is a cubic Bezier span of width 1/2: derivative at the shared end is
            // 3 (P3 - P2) / (1/2) = (6,-18) from the left and 3 (P4 - P3) / (1/2) = (6,-6) from the right,
            // and the two differ, which is what makes the break a break.
            expect: vec![
                Expect::Deriv { t: 0.5, order: 1, side: Side::Left, xy: [6.0, -18.0], scale: 18.0 },
                Expect::Deriv { t: 0.5, order: 1, side: Side::Right, xy: [6.0, -6.0], scale: 18.0 },
            ],
        },
        Probe {
            id: "full-multiplicity-interior-knot-two-sided",
            rule: "NP-16",
            curve: clamped_cubic_with_c0_break(),
            expect: vec![Expect::DerivRejects {
                t: 0.5,
                order: 1,
                side: Side::Both,
                err: NumericError::TwoSidedAtBreak,
            }],
        },
        Probe {
            id: "huge-coordinate-line",
            rule: "NP-02",
            curve: ProbeCurve::Line { origin: [m, m], dir: [1., 0.] },
            expect: vec![Expect::Point { t: 1.0, xy: [m + 1.0, m], scale: 1.0 }],
        },
        Probe {
            id: "mixed-magnitude-circle",
            rule: "NP-05",
            curve: ProbeCurve::Circle { center: [m, m], radius: 1.0 },
            expect: vec![Expect::Point { t: 0.0, xy: [m + 1.0, m], scale: 1.0 }],
        },
        Probe {
            id: "weight-near-zero-evaluates",
            rule: "NP-15",
            curve: ProbeCurve::BSpline {
                degree: 2,
                cps: vec![[0., 0.], [1., 5.], [2., 0.]],
                weights: Some(vec![1., P.weight_floor * 2.0, 1.]),
                knots: vec![0., 0., 0., 1., 1., 1.],
            },
            expect: vec![Expect::InsideHull { samples: 64 }],
        },
        Probe {
            id: "weight-below-floor-rejected",
            rule: "NP-15",
            curve: ProbeCurve::BSpline {
                degree: 2,
                cps: vec![[0., 0.], [1., 5.], [2., 0.]],
                weights: Some(vec![1., P.weight_floor * 0.5, 1.]),
                knots: vec![0., 0., 0., 1., 1., 1.],
            },
            expect: vec![Expect::BuildRejects(NumericError::BadWeight)],
        },
        Probe {
            id: "denormal-knot-rejected",
            rule: "NP-01",
            curve: ProbeCurve::BSpline {
                degree: 1,
                cps: vec![[0., 0.], [1., 1.]],
                weights: None,
                knots: vec![f64::MIN_POSITIVE / 2.0, f64::MIN_POSITIVE / 2.0, 1., 1.],
            },
            expect: vec![Expect::BuildRejects(NumericError::Subnormal)],
        },
        Probe {
            id: "denormal-parameter-rejected",
            rule: "NP-01",
            curve: ProbeCurve::Line { origin: [0., 0.], dir: [1., 0.] },
            expect: vec![Expect::EvalRejects { t: f64::MIN_POSITIVE / 2.0, err: NumericError::Subnormal }],
        },
        Probe {
            id: "zero-length-trim-rejected",
            rule: "NP-12",
            curve: ProbeCurve::Trimmed {
                basis: Box::new(ProbeCurve::Circle { center: [0., 0.], radius: 1.0 }),
                t1: 1.0,
                t2: 1.0,
                sense: true,
            },
            expect: vec![Expect::BuildRejects(NumericError::CollapsedTrim)],
        },
        Probe {
            id: "inverted-trim-rejected",
            rule: "NP-12",
            curve: ProbeCurve::Trimmed {
                basis: Box::new(ProbeCurve::Line { origin: [0., 0.], dir: [1., 0.] }),
                t1: 2.0,
                t2: 1.0,
                sense: true,
            },
            expect: vec![Expect::BuildRejects(NumericError::InconsistentSense)],
        },
    ]
}

/// Runs one probe against an evaluator. `Err` carries a message naming the probe and rule.
pub fn run_probe(ev: &dyn Evaluator, p: &Probe) -> Result<(), String> {
    let fail = |m: String| Err(format!("{} [{}]: {m}", p.id, p.rule));
    let built = match ev.build(&p.curve) {
        Ok(b) => Some(b),
        Err(e) => {
            return match p.expect.first() {
                Some(Expect::BuildRejects(want)) if *want == e => Ok(()),
                _ => fail(format!("build refused with {e}")),
            };
        }
    };
    let b = built.unwrap();
    for e in &p.expect {
        match e {
            Expect::BuildRejects(want) => return fail(format!("built, expected refusal {want}")),
            Expect::Point { t, xy, scale } => match b.eval(*t) {
                Ok(got) if P.near_point(got, *xy, *scale) => {}
                other => return fail(format!("eval({t}) = {other:?}, expected {xy:?}")),
            },
            Expect::EvalRejects { t, err } => match b.eval(*t) {
                Err(got) if got == *err => {}
                other => return fail(format!("eval({t}) = {other:?}, expected {err}")),
            },
            Expect::Deriv { t, order, side, xy, scale } => match b.derivative(*t, *order, *side) {
                Ok(got) if P.near_point(got, *xy, *scale) => {}
                other => {
                    return fail(format!("derivative({t},{order},{side:?}) = {other:?}, expected {xy:?}"));
                }
            },
            Expect::DerivRejects { t, order, side, err } => match b.derivative(*t, *order, *side) {
                Err(got) if got == *err => {}
                other => {
                    return fail(format!("derivative({t},{order},{side:?}) = {other:?}, expected {err}"));
                }
            },
            Expect::InsideHull { samples } => {
                let ProbeCurve::BSpline { cps, .. } = &p.curve else {
                    return fail("InsideHull needs a B-spline".into());
                };
                let (lo, hi) = b.domain();
                let bound = |k: usize| {
                    let v: Vec<f64> = cps.iter().map(|c| c[k]).collect();
                    (
                        v.iter().cloned().fold(f64::INFINITY, f64::min),
                        v.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                    )
                };
                let (bx, by) = (bound(0), bound(1));
                for i in 0..=*samples {
                    let t = lo + (hi - lo) * (i as f64) / (*samples as f64);
                    let Ok(q) = b.eval(t) else { return fail(format!("eval({t}) refused")) };
                    let slack = P.tol(bx.1 - bx.0 + by.1 - by.0, 0.0);
                    if !(q[0] >= bx.0 - slack
                        && q[0] <= bx.1 + slack
                        && q[1] >= by.0 - slack
                        && q[1] <= by.1 + slack)
                    {
                        return fail(format!("eval({t}) = {q:?} outside the hull box"));
                    }
                }
            }
        }
    }
    Ok(())
}
