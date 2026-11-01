use crate::{Curve, CurveEval, PvecError, Transform2};
use pvec_geometry::{Curve2d, GeometryError, Point2};

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

/// The sampling margin: the deviation of a segment is found at seven interior points, and the claimed
/// bound is that measure times this, so a curve that bulges between two samples is still inside it.
const MARGIN: f64 = 1.05;

/// Where a curve is not smooth, as parameters strictly inside its domain: the interior knots of a
/// spline, the vertices of a polyline, the joints of a composite and whichever of those a trim keeps.
fn breaks(curve: &Curve) -> Vec<f64> {
    let mut out: Vec<f64> = match curve {
        Curve::Polyline(p) => (1..p.vertices().len() - 1).map(|i| i as f64).collect(),
        Curve::BSpline(b) => b.knots().to_vec(),
        Curve::RationalBSpline(b) => b.knots().to_vec(),
        Curve::Composite(c) => c
            .segments()
            .iter()
            .enumerate()
            .flat_map(|(k, seg)| {
                let d = Curve2d::domain(seg);
                let own = breaks(seg).into_iter().map(move |u| k as f64 + (u - d.lo()) / d.width());
                (k > 0).then_some(k as f64).into_iter().chain(own)
            })
            .collect(),
        Curve::Trimmed(t) => {
            let trim = t.trim();
            breaks(t.basis())
                .into_iter()
                .map(|b| if trim.reversed { trim.start + trim.span - b } else { b - trim.start })
                .collect()
        }
        _ => Vec::new(),
    };
    let d = Curve2d::domain(curve);
    out.retain(|t| *t > d.lo() && *t < d.hi());
    out.sort_by(f64::total_cmp);
    out.dedup();
    out
}

/// One stretch of the curve between two parameters, with the exact points at its ends.
struct Span {
    a: f64,
    b: f64,
    pa: [f64; 2],
    pb: [f64; 2],
    /// The exact points at `a + (b - a) * j / 8` for `j` in 1 to 7 that are already known. A half of a split
    /// span shares three of these with its parent, and the parent's middle is the split point itself.
    known: [Option<[f64; 2]>; 7],
}

pub(crate) fn tessellate_with(
    curve: &Curve,
    ctx: &TessellationContext,
    split_at_breaks: bool,
    evals: &std::cell::Cell<usize>,
) -> Result<Tessellation, PvecError> {
    let bad = |reason: &str| Err(PvecError::BadContext { reason: reason.to_string() });
    if ctx.max_screen_error_px.is_nan() || ctx.max_screen_error_px <= 0.0 {
        return bad("max_screen_error_px must be positive");
    }
    if ctx.max_world_error.is_nan() || ctx.max_world_error <= 0.0 {
        return bad("max_world_error must be positive");
    }
    if ctx.pixel_ratio.is_nan() || ctx.pixel_ratio <= 0.0 || ctx.pixel_ratio.is_infinite() {
        return bad("pixel_ratio must be positive and finite");
    }
    if ctx.max_segments == 0 {
        return bad("max_segments must be at least 1");
    }
    if matches!(curve, Curve::Line(_)) {
        return Err(crate::error::geometry(GeometryError::Unbounded));
    }
    let at = |t: f64| {
        evals.set(evals.get() + 1);
        curve.point_at(t)
    };
    let to_world = |q: [f64; 2]| ctx.source_to_map.apply_point(Point2 { x: q[0], y: q[1] });
    let to_device = |w: Point2| {
        let s = ctx.map_to_screen.apply_point(w);
        [s.x * ctx.pixel_ratio, s.y * ctx.pixel_ratio]
    };
    let from_chord = |q: [f64; 2], a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        let t = if len2 == 0.0 {
            0.0
        } else {
            (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
        };
        (q[0] - a[0] - t * dx).hypot(q[1] - a[1] - t * dy)
    };
    // The deviation of the chord of a span from the curve over it: on screen in device pixels, and in
    // map units, each times the sampling margin.
    // It also returns the seven sample points, so a split can hand them on instead of evaluating them again.
    let measure = |s: &Span| -> Result<(f64, f64, [[f64; 2]; 7]), PvecError> {
        let (wa, wb) = (to_world(s.pa), to_world(s.pb));
        let (da, db) = (to_device(wa), to_device(wb));
        let (mut px, mut world) = (0.0f64, 0.0f64);
        let mut samples = [[0.0f64; 2]; 7];
        for (i, slot) in samples.iter_mut().enumerate() {
            *slot = match s.known[i] {
                Some(p) => p,
                None => at(s.a + (s.b - s.a) * ((i + 1) as f64 / 8.0))?,
            };
            let q = to_world(*slot);
            world = world.max(from_chord([q.x, q.y], [wa.x, wa.y], [wb.x, wb.y]));
            px = px.max(from_chord(to_device(q), da, db));
        }
        Ok((px * MARGIN, world * MARGIN, samples))
    };
    let within = |(px, world): (f64, f64)| px <= ctx.max_screen_error_px && world <= ctx.max_world_error;
    let unmet = |(px, world): (f64, f64)| {
        if px > ctx.max_screen_error_px {
            PvecError::ErrorBudgetUnmet { achieved: px, requested: ctx.max_screen_error_px }
        } else {
            PvecError::ErrorBudgetUnmet { achieved: world, requested: ctx.max_world_error }
        }
    };

    let d = Curve2d::domain(curve);
    let mut cuts = vec![d.lo()];
    if split_at_breaks {
        cuts.extend(breaks(curve));
    }
    cuts.push(d.hi());
    let points = cuts.iter().map(|t| at(*t)).collect::<Result<Vec<_>, _>>()?;
    let spans: Vec<Span> = cuts
        .windows(2)
        .zip(points.windows(2))
        .map(|(t, p)| Span { a: t[0], b: t[1], pa: p[0], pb: p[1], known: [None; 7] })
        .collect();
    if spans.len() > ctx.max_segments {
        // Not even the breaks fit. What the same limit gives with the spans spread evenly says how far off.
        let n = ctx.max_segments;
        let even = (0..n)
            .map(|i| {
                let (a, b) = (
                    d.lo() + d.width() * i as f64 / n as f64,
                    d.lo() + d.width() * (i + 1) as f64 / n as f64,
                );
                Ok(Span { a, b, pa: at(a)?, pb: at(b)?, known: [None; 7] })
            })
            .collect::<Result<Vec<_>, PvecError>>()?;
        let mut worst = (0.0f64, 0.0f64);
        for s in &even {
            let m = measure(s).map(|r| (r.0, r.1))?;
            worst = (worst.0.max(m.0), worst.1.max(m.1));
        }
        return Err(unmet(worst));
    }

    let mut vertices = vec![points[0]];
    let mut worst = (0.0f64, 0.0f64);
    let mut stack: Vec<Span> = spans.into_iter().rev().collect();
    while let Some(span) = stack.pop() {
        let (px, world, samples) = measure(&span)?;
        let m = (px, world);
        if within(m) {
            vertices.push(span.pb);
            worst = (worst.0.max(m.0), worst.1.max(m.1));
            continue;
        }
        let mid = span.a + (span.b - span.a) * 0.5;
        let room = vertices.len() - 1 + stack.len() + 2 <= ctx.max_segments;
        if !room || !(mid > span.a && mid < span.b) {
            // Nothing more can be split: the answer is the worst of what is left.
            let mut left = m;
            for s in &stack {
                let o = measure(s).map(|r| (r.0, r.1))?;
                left = (left.0.max(o.0), left.1.max(o.1));
            }
            return Err(unmet(left));
        }
        let pm = samples[3];
        let half = |from: usize| {
            let mut known = [None; 7];
            for k in 0..3 {
                known[2 * k + 1] = Some(samples[from + k]);
            }
            known
        };
        stack.push(Span { a: mid, b: span.b, pa: pm, pb: span.pb, known: half(4) });
        stack.push(Span { a: span.a, b: mid, pa: span.pa, pb: pm, known: half(0) });
    }
    Ok(Tessellation {
        vertices,
        achieved_screen_error_px: worst.0,
        achieved_world_error: worst.1,
        conformance: Conformance::DisplayFidelity,
    })
}

impl Tessellate for Curve {
    fn tessellate(&self, ctx: &TessellationContext) -> Result<Tessellation, PvecError> {
        tessellate_with(self, ctx, true, &std::cell::Cell::new(0))
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
        let t = tessellate_with(&corner(), &context(), true, &Default::default()).unwrap();
        assert_eq!(t.vertices.len(), 3);
        assert_eq!(t.vertices[1], [300.0, 400.0]);
    }

    /// Mutation check: with the knot-break insertion switched off, the same fixture cannot be held to
    /// its budget in the same number of segments, so the test above is the one that depends on it.
    #[test]
    fn issue_53_without_knot_break_insertion_the_c1_discontinuity_exceeds_its_budget() {
        assert!(matches!(
            tessellate_with(&corner(), &context(), false, &Default::default()),
            Err(PvecError::ErrorBudgetUnmet { .. })
        ));
    }

    /// Issue 240: a span that fails its budget is split, and its seven samples are shared with the halves, so each
    /// measured span after the first costs four evaluations and the split point costs none. The count is exact.
    #[test]
    fn issue_240_a_split_reuses_the_samples_of_its_parent() {
        let p = |x: f64, y: f64| Point2 { x, y };
        let curve = Curve::BSpline(
            BSpline::new(
                3,
                vec![p(0.0, 0.0), p(900.0, 1200.0), p(-300.0, 1200.0), p(600.0, 0.0)],
                vec![0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0],
            )
            .unwrap(),
        );
        let ctx = TessellationContext { max_segments: 4096, ..context() };
        let evals = std::cell::Cell::new(0);
        let t = tessellate_with(&curve, &ctx, true, &evals).unwrap();
        let used = evals.get();
        let n = t.vertices.len() - 1;
        assert!(n >= 16, "the fixture has to need real subdivision, got {n} segments");
        // two end points, seven samples for the root, four new samples for each of the n - 1 other measured spans
        // on the way down (a binary tree with n leaves has 2n - 1 spans, n - 1 of them split)
        assert_eq!(used, 2 + 7 + 4 * (2 * n - 2), "evaluations for {n} segments");
    }

    /// A clamped cubic through `controls` zigzagging points, evaluated with the count of `tessellate_with`'s evaluations.
    /// Returns (segments, evaluations).
    fn evaluations_for_zigzag(controls: usize) -> (usize, usize) {
        let p = |x: f64, y: f64| Point2 { x, y };
        let points: Vec<Point2> =
            (0..controls).map(|i| p(i as f64 * 3.0, if i % 2 == 0 { 0.0 } else { 40.0 })).collect();
        let spans = controls - 3;
        let mut knots = vec![0.0; 4];
        knots.extend((1..spans).map(|i| i as f64 / spans as f64));
        knots.extend([1.0; 4]);
        let curve = Curve::BSpline(BSpline::new(3, points, knots).unwrap());
        let ctx = TessellationContext { max_segments: 1 << 20, max_screen_error_px: 1.0, ..context() };
        let evals = std::cell::Cell::new(0);
        let t = tessellate_with(&curve, &ctx, true, &evals).unwrap();
        (t.vertices.len() - 1, evals.get())
    }

    /// Issue 245: a curve of 10^4 control points costs 11 to 14 ms in one call, with about eight evaluations for each
    /// segment it returns. Deciding that a span is flat from the control polygon of its knot span needs about one.
    /// The bound is two evaluations for each segment, plus the end points and a small constant.
    #[test]
    fn issue_245_a_large_spline_costs_at_most_two_evaluations_for_each_segment() {
        let (n, used) = evaluations_for_zigzag(10_000);
        assert!(n >= 5_000, "the fixture has to be as heavy as the pathological one, got {n} segments");
        assert!(used <= 2 * n + 16, "{used} evaluations for {n} segments");
    }

    /// The same bound for a small curve, so it is a property of the tessellator and not of one big fixture.
    #[test]
    fn issue_245_a_small_spline_costs_at_most_two_evaluations_for_each_segment() {
        let (n, used) = evaluations_for_zigzag(40);
        assert!(n >= 16, "the fixture has to need real subdivision, got {n} segments");
        assert!(used <= 2 * n + 16, "{used} evaluations for {n} segments");
    }
}
