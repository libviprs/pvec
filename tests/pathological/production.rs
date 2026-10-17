//! The adapter that runs the evaluator probes against the production crate (`pvec-geometry`).
#![allow(missing_docs)]

use crate::probes::{Built, Evaluator, ProbeCurve};
use pvec_geometry::{
    BSpline, Circle, Curve, Curve2d, GeometryError, Line, Placement2, Point2, RationalBSpline, Side, Trimmed,
    Vector2,
};
use pvec_numeric::NumericError;

pub struct Production;
struct Wrapped(Curve);

/// The probes speak `NumericError`. Every refusal they expect is a numeric-policy refusal, so any
/// other kind of error is mapped to a variant that cannot equal an expectation.
fn lower(e: GeometryError) -> NumericError {
    e.as_numeric().unwrap_or(NumericError::Limit)
}

fn p2(a: [f64; 2]) -> Point2 {
    Point2::new(a[0], a[1])
}

fn build(c: &ProbeCurve) -> Result<Curve, GeometryError> {
    Ok(match c {
        ProbeCurve::Line { origin, dir } => {
            Curve::Line(Line::new(p2(*origin), Vector2::new(dir[0], dir[1]))?)
        }
        ProbeCurve::Circle { center, radius } => Curve::Circle(Circle::new(
            Placement2::new(p2(*center), Vector2::new(1.0, 0.0))?,
            *radius,
        )?),
        ProbeCurve::Trimmed { basis, t1, t2, sense } => {
            Curve::Trimmed(Trimmed::new(build(basis)?, *t1, *t2, *sense)?)
        }
        ProbeCurve::BSpline { degree, cps, weights: None, knots } => {
            Curve::BSpline(BSpline::new(*degree, cps.iter().map(|c| p2(*c)).collect(), knots.clone())?)
        }
        ProbeCurve::BSpline { degree, cps, weights: Some(w), knots } => Curve::RationalBSpline(
            RationalBSpline::new(*degree, cps.iter().map(|c| p2(*c)).collect(), knots.clone(), w.clone())?,
        ),
    })
}

impl Evaluator for Production {
    fn build(&self, c: &ProbeCurve) -> Result<Box<dyn Built>, NumericError> {
        build(c).map(|c| Box::new(Wrapped(c)) as Box<dyn Built>).map_err(lower)
    }
}

impl Built for Wrapped {
    fn domain(&self) -> (f64, f64) {
        let d = self.0.domain();
        (d.lo(), d.hi())
    }
    fn eval(&self, t: f64) -> Result<[f64; 2], NumericError> {
        self.0.eval(t).map(|p| [p.x, p.y]).map_err(lower)
    }
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<[f64; 2], NumericError> {
        self.0.derivative(t, order, side).map(|d| [d.x, d.y]).map_err(lower)
    }
}
