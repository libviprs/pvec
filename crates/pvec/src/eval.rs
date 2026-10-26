use crate::{Bounds2, Curve, Domain, PvecError, Transform2};

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
    fn derivative(&self, t: f64, order: usize) -> Result<[f64; 2], PvecError>;
    /// As [`CurveEval::derivative`], taken from `side`.
    fn derivative_from(&self, t: f64, order: usize, side: crate::Side) -> Result<[f64; 2], PvecError>;
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
        unimplemented!("P7.2")
    }
    fn point_at(&self, _t: f64) -> Result<[f64; 2], PvecError> {
        Err(PvecError::Geometry { rule: "unimplemented".to_string() })
    }
    fn derivative(&self, _t: f64, _order: usize) -> Result<[f64; 2], PvecError> {
        Err(PvecError::Geometry { rule: "unimplemented".to_string() })
    }
    fn derivative_from(&self, _t: f64, _order: usize, _side: crate::Side) -> Result<[f64; 2], PvecError> {
        Err(PvecError::Geometry { rule: "unimplemented".to_string() })
    }
    fn bounds(&self) -> Result<Bounds2, PvecError> {
        Err(PvecError::Geometry { rule: "unimplemented".to_string() })
    }
    fn is_closed(&self) -> bool {
        false
    }
    fn transformed(&self, _m: &Transform2) -> Result<Curve, PvecError> {
        Err(PvecError::Geometry { rule: "unimplemented".to_string() })
    }
}
