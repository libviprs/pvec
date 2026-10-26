use crate::{Curve, PvecError, Transform2};

/// What a tessellation claims to be. There is one value: a tessellation is an approximation for
/// display, never the curve itself. A type with a second value would let a consumer present a polyline
/// as exact geometry, so this one is closed on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conformance {
    /// Within the stated budget on screen, for drawing. Not mathematically exact.
    DisplayFidelity,
}

/// The whole transform chain and the budget a tessellation is held to.
///
/// A point `p` of the curve is at `map_to_screen(source_to_map(p))` on screen, in CSS pixels, and
/// `pixel_ratio` device pixels make one CSS pixel. The deviation of the polyline from the exact curve is
/// measured after the chain, on screen, so a coordinate offset of 10^6 in the source costs nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TessellationContext {
    /// Source space to map (world) space.
    pub source_to_map: Transform2,
    /// Map space to screen space, in CSS pixels.
    pub map_to_screen: Transform2,
    /// Device pixels per CSS pixel. Positive.
    pub pixel_ratio: f64,
    /// The largest deviation allowed on screen, in device pixels. Positive.
    pub max_screen_error_px: f64,
    /// The largest deviation allowed in map units. Positive; `f64::INFINITY` means no world budget.
    pub max_world_error: f64,
    /// The most segments the result may have. At least 1.
    pub max_segments: usize,
}

/// A polyline standing in for a curve, for display.
#[derive(Clone, Debug, PartialEq)]
pub struct Tessellation {
    /// The vertices, in source space, in curve order. Break points of the curve are vertices.
    pub vertices: Vec<[f64; 2]>,
    /// The largest deviation found on screen, in device pixels.
    pub achieved_screen_error_px: f64,
    /// The largest deviation found in map units.
    pub achieved_world_error: f64,
    /// Always [`Conformance::DisplayFidelity`].
    pub conformance: Conformance,
}

/// Adaptive tessellation of a curve under an explicit error budget.
pub trait Tessellate {
    /// Splits the curve into segments until the deviation is inside the budget of `ctx`, at the knots
    /// and continuity breaks first. A budget that `ctx.max_segments` cannot reach is refused with
    /// [`PvecError::ErrorBudgetUnmet`], never quietly exceeded.
    fn tessellate(&self, ctx: &TessellationContext) -> Result<Tessellation, PvecError>;
}

pub(crate) fn tessellate_with(
    _curve: &Curve,
    _ctx: &TessellationContext,
    _split_at_breaks: bool,
) -> Result<Tessellation, PvecError> {
    Err(PvecError::Geometry { rule: "unimplemented".to_string() })
}

impl Tessellate for Curve {
    fn tessellate(&self, ctx: &TessellationContext) -> Result<Tessellation, PvecError> {
        tessellate_with(self, ctx, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvec_geometry::{BSpline, Point2};

    /// A degree 1 B-spline with a corner at t = 0.3, which no bisection of [0, 1] lands on.
    fn corner() -> Curve {
        let p = |x: f64, y: f64| Point2 { x, y };
        Curve::BSpline(
            BSpline::new(1, vec![p(0.0, 0.0), p(300.0, 400.0), p(600.0, 0.0)], vec![0.0, 0.0, 0.3, 1.0, 1.0])
                .unwrap(),
        )
    }

    fn context() -> TessellationContext {
        TessellationContext {
            source_to_map: Transform2::IDENTITY,
            map_to_screen: Transform2::IDENTITY,
            pixel_ratio: 1.0,
            max_screen_error_px: 0.01,
            max_world_error: f64::INFINITY,
            max_segments: 8,
        }
    }

    #[test]
    fn issue_53_the_knot_break_is_a_vertex_and_two_segments_are_exact() {
        let t = tessellate_with(&corner(), &context(), true).unwrap();
        assert_eq!(t.vertices.len(), 3);
        assert_eq!(t.vertices[1], [300.0, 400.0]);
    }

    /// Mutation check: with the knot-break insertion switched off, the same fixture cannot be held to
    /// its budget in the same number of segments, so the test above is the one that depends on it.
    #[test]
    fn issue_53_without_knot_break_insertion_the_c1_discontinuity_exceeds_its_budget() {
        assert!(matches!(
            tessellate_with(&corner(), &context(), false),
            Err(PvecError::ErrorBudgetUnmet { .. })
        ));
    }
}
