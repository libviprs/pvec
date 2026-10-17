//! Bridges fixture geometry to the P1.3 reference evaluator, and the packed-size model.

use geometry_api_spike::{
    BSpline, Circle, Composite, Curve, Ellipse, Error, Line, Polyline, RationalBSpline, Trimmed,
};
use pvec_fixtures::Geom;

fn line(a: [f64; 2], b: [f64; 2]) -> Result<Curve, Error> {
    let l = Line::new(a, [b[0] - a[0], b[1] - a[1]])?;
    let len = (b[0] - a[0]).hypot(b[1] - a[1]);
    Ok(Curve::Trimmed(Trimmed::new(Curve::Line(l), 0.0, len, true)?))
}

/// Builds the reference curve for a fixture geometry. An error means the P1.3 evaluator refuses the
/// entity under the numeric policy, which makes it infeasible for Tier 0 (neither exact nor preview).
pub fn reference_curve(g: &Geom) -> Result<Curve, Error> {
    Ok(match g {
        Geom::Line { a, b } => line(*a, *b)?,
        Geom::Circle { c, r } => Curve::Circle(Circle::new(*c, [1.0, 0.0], *r)?),
        Geom::Arc { c, r, a0, a1 } => {
            Curve::Trimmed(Trimmed::new(Curve::Circle(Circle::new(*c, [1.0, 0.0], *r)?), *a0, *a1, true)?)
        }
        Geom::Ellipse { c, a, b, rot, t0, t1 } => {
            let e = Ellipse::new(*c, [rot.cos(), rot.sin()], *a, *b)?;
            Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), *t0, *t1, true)?)
        }
        Geom::Polyline { pts } => Curve::Polyline(Polyline::new(pts.clone())?),
        Geom::Spline { degree, cps, knots, weights } => match weights {
            None => Curve::BSpline(BSpline::new(*degree as usize, cps.clone(), knots.clone())?),
            Some(w) => Curve::RationalBSpline(RationalBSpline::new(
                *degree as usize,
                cps.clone(),
                knots.clone(),
                w.clone(),
            )?),
        },
        Geom::Composite { parts } => {
            let segs = parts.iter().map(reference_curve).collect::<Result<Vec<_>, _>>()?;
            Curve::Composite(Composite::new(segs)?)
        }
    })
}

/// Bytes of the exact packed record under the size model of `docs/spikes/01-7-tier0.md`:
/// id (4) + tag (1) + payload, all coordinates `f64`. It is a model for budgeting, not the layout
/// P1.5 will recommend.
pub fn exact_bytes(g: &Geom) -> usize {
    5 + payload_bytes(g)
}

fn payload_bytes(g: &Geom) -> usize {
    match g {
        Geom::Line { .. } => 32,
        Geom::Circle { .. } => 24,
        Geom::Arc { .. } => 40,
        Geom::Ellipse { .. } => 56,
        Geom::Polyline { pts } => 4 + 16 * pts.len(),
        Geom::Spline { degree, cps, weights, .. } => {
            let n = cps.len();
            1 + 1 + 4 + 16 * n + 8 * (n + *degree as usize + 1) + if weights.is_some() { 8 * n } else { 0 }
        }
        Geom::Composite { parts } => 2 + parts.iter().map(|p| 1 + payload_bytes(p)).sum::<usize>(),
    }
}
