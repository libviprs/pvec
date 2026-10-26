use crate::error::geometry;
use crate::{Bounds2, Curve, Domain, PvecError, Side, Transform2};
use pvec_geometry::Curve2d;

/// Exact evaluation of a curve, under the numeric policy of `docs/NUMERIC_POLICY.md`.
///
/// Nothing here approximates and nothing is cached from a tessellation: `point_at` of the same
/// parameter gives the same bits before and after any call to [`crate::Tessellate::tessellate`].
pub trait CurveEval {
    /// The parameter interval.
    fn domain(&self) -> Domain;
    /// The point at parameter `t`, as `[x, y]`.
    fn point_at(&self, t: f64) -> Result<[f64; 2], PvecError>;
    /// The derivative of `order` at `t`, as `[dx, dy]`. Where only one-sided limits exist (a knot, a
    /// polyline vertex, a composite joint) the request is refused rather than answered from a side;
    /// [`CurveEval::derivative_from`] names the side.
    fn derivative_at(&self, t: f64, order: usize) -> Result<[f64; 2], PvecError>;
    /// As [`CurveEval::derivative_at`], taken from `side`.
    fn derivative_from(&self, t: f64, order: usize, side: Side) -> Result<[f64; 2], PvecError>;
    /// A box that contains the curve. Closed form for arcs, polylines and cubics at most; the control
    /// hull for splines. An unbounded line has none and is refused.
    fn bounds(&self) -> Result<Bounds2, PvecError>;
    /// Whether the curve ends where it starts.
    fn is_closed(&self) -> bool;
    /// The exact image under an affine map. A kind that cannot represent its image exactly is refused.
    fn transformed(&self, m: &Transform2) -> Result<Curve, PvecError>;
}

impl CurveEval for Curve {
    fn domain(&self) -> Domain {
        Curve2d::domain(self)
    }
    fn point_at(&self, t: f64) -> Result<[f64; 2], PvecError> {
        self.eval(t).map(|p| [p.x, p.y]).map_err(geometry)
    }
    fn derivative_at(&self, t: f64, order: usize) -> Result<[f64; 2], PvecError> {
        self.derivative_from(t, order, Side::Both)
    }
    fn derivative_from(&self, t: f64, order: usize, side: Side) -> Result<[f64; 2], PvecError> {
        Curve2d::derivative(self, t, order, side).map(|v| [v.x, v.y]).map_err(geometry)
    }
    fn bounds(&self) -> Result<Bounds2, PvecError> {
        let b = Curve2d::bounds(self).map_err(geometry)?;
        Ok(Bounds2 { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] })
    }
    fn is_closed(&self) -> bool {
        match self {
            Curve::Circle(_) | Curve::Ellipse(_) => true,
            Curve::Polyline(p) => p.vertices().first() == p.vertices().last(),
            Curve::Trimmed(t) => t.is_full_period(),
            Curve::Composite(c) => c.is_declared_closed(),
            Curve::Bezier(b) => b.is_declared_closed(),
            Curve::BSpline(b) => b.is_declared_closed(),
            Curve::RationalBSpline(b) => b.is_declared_closed(),
            _ => false,
        }
    }
    fn transformed(&self, m: &Transform2) -> Result<Curve, PvecError> {
        self.transform(m).map(|(c, _)| c).map_err(geometry)
    }
}
