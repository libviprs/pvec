//! Which geometry each curve becomes, and with what claim (`docs/INTERCHANGE.md`, chapter 4).
//!
//! The curve model is evaluated only through `pvec-geometry`'s `Curve2d`; this file never evaluates a
//! curve itself. It picks the target construct, writes the points, and where it has to approximate,
//! subdivides until the deviation it measures against that exact evaluation is within the caller's
//! tolerance and records the measured value.

use super::wkb::{Piece, Pt, circumcircle, piece_distance, segment_distance};
use super::{GpkgError, Rank};
use core::f64::consts::PI;
use pvec_geometry::{Curve, Curve2d, Point2};
use pvec_numeric::Policy;

const POLICY: Policy = Policy::V1;

/// Samples inside one chord when its deviation is measured, before the peak is refined.
const CHORD_SAMPLES: usize = 24;
/// Segments of the local reference polyline a chord point is measured against.
const REFERENCE_SEGMENTS: usize = 64;
/// The measured deviation is inflated by this factor before it is recorded or compared with the
/// tolerance, so a recorded bound errs high.
const SAFETY: f64 = 1.001;
/// A bound on the vertices one curve may produce.
const MAX_VERTICES: usize = 1_000_000;
/// Equal pieces every approximated curve starts from, so a closed or periodic curve is never one chord.
const INITIAL_PIECES: usize = 8;

/// What a curve became.
pub(super) struct Done {
    pub pieces: Vec<Piece>,
    pub compound: bool,
    pub rank: Rank,
    /// For an approximation, the recorded bound; for an exact class the residual of the written points
    /// against the exact curve where that was measured.
    pub deviation: Option<f64>,
    pub note: String,
}

pub(super) enum Outcome {
    Done(Done),
    Unsupported(&'static str),
}

fn pt(p: Point2) -> Pt {
    [p.x, p.y]
}

fn ev(c: &Curve, t: f64) -> Result<Pt, GpkgError> {
    c.eval(t).map(pt).map_err(|e| GpkgError::Evaluation(e.rule()))
}

fn geom<T>(r: Result<T, pvec_geometry::GeometryError>) -> Result<T, GpkgError> {
    r.map_err(|e| GpkgError::Evaluation(e.rule()))
}

/// The magnitude term NP-04 wants: the largest coordinate the curve reaches.
fn magnitude(c: &Curve) -> Result<f64, GpkgError> {
    Ok(geom(c.bounds())?.magnitude())
}

pub(super) fn translate_curve(curve: &Curve, tol: f64) -> Result<Outcome, GpkgError> {
    match curve {
        Curve::Line(_) => {
            Ok(Outcome::Unsupported("an unbounded line has no finite geometry to write; trim it first"))
        }
        Curve::Composite(c) => composite(c.segments(), tol),
        other => Ok(Outcome::Done(member(other, tol)?)),
    }
}

fn exact(piece: Piece, rank: Rank, note: &str) -> Done {
    Done { pieces: vec![piece], compound: false, rank, deviation: None, note: note.to_string() }
}

/// A curve that is not a composite.
fn member(curve: &Curve, tol: f64) -> Result<Done, GpkgError> {
    match curve {
        Curve::Polyline(p) => {
            let pts = p.vertices().iter().map(|v| pt(*v)).collect();
            let mut d = exact(Piece::Line(pts), Rank::Exact, "the vertices carried over bit for bit");
            d.deviation = Some(0.0);
            Ok(d)
        }
        Curve::Circle(_) => arc_of(curve, true),
        Curve::Trimmed(t) => match t.basis() {
            Curve::Line(_) => {
                let d = curve.domain();
                let piece = Piece::Line(vec![ev(curve, d.lo())?, ev(curve, d.hi())?]);
                residual(
                    curve,
                    exact(piece, Rank::Repar, "origin, direction and trim became the two end points"),
                )
            }
            Curve::Circle(_) => arc_of(curve, t.is_full_period()),
            _ => spline_or_approximate(curve, tol),
        },
        Curve::Ellipse(_) | Curve::Bezier(_) | Curve::BSpline(_) | Curve::RationalBSpline(_) => {
            spline_or_approximate(curve, tol)
        }
        _ => Err(GpkgError::Evaluation("a curve kind this export does not know")),
    }
}

/// The residual of the written points against the exact curve, sampled, for the classes that claim
/// exactness. It is evidence, not an upper bound: the points are exact by construction.
fn residual(curve: &Curve, mut done: Done) -> Result<Done, GpkgError> {
    let d = curve.domain();
    let n = 64;
    let mut worst: f64 = 0.0;
    for k in 0..=n {
        let q = ev(curve, d.lo() + d.width() * k as f64 / n as f64)?;
        let dist = done.pieces.iter().map(|p| piece_distance(p, q)).fold(f64::INFINITY, f64::min);
        worst = worst.max(dist);
    }
    done.deviation = Some(worst);
    Ok(done)
}

/// A circular arc, or a whole circle, as a `CIRCULARSTRING`.
fn arc_of(curve: &Curve, whole: bool) -> Result<Done, GpkgError> {
    let d = curve.domain();
    let sweep = if whole { 2.0 * PI } else { d.width() };
    let count = if whole || sweep > PI { 5 } else { 3 };
    let mut pts = Vec::with_capacity(count);
    for k in 0..count {
        let t = if k == count - 1 { d.hi() } else { d.lo() + d.width() * k as f64 / (count - 1) as f64 };
        pts.push(ev(curve, t)?);
    }
    if whole {
        // the first and last evaluations differ by rounding; a closed arc ends exactly where it began
        pts[count - 1] = pts[0];
    }
    let note = "centre, radius and trim became start, middle and end points of the same circle";
    residual(curve, exact(Piece::Arcs(pts), Rank::Repar, note))
}

/// Whether the curve is piecewise straight (degree one) and so exactly a polyline through its knots.
fn piecewise_linear(curve: &Curve) -> bool {
    match curve {
        Curve::Bezier(b) => b.degree() == 1,
        Curve::BSpline(b) => b.degree() == 1,
        Curve::RationalBSpline(b) => b.degree() == 1,
        Curve::Trimmed(t) => piecewise_linear(t.basis()),
        _ => false,
    }
}

/// Parameters of the curve's own domain where the curve may change character: its distinct knots,
/// clipped to the domain, mapped through a trim.
fn breakpoints(curve: &Curve) -> Vec<f64> {
    let d = curve.domain();
    let mut out = vec![d.lo(), d.hi()];
    let mut add = |knots: &[f64], map: &dyn Fn(f64) -> Option<f64>| {
        for k in knots {
            if let Some(t) = map(*k) {
                if t > d.lo() && t < d.hi() {
                    out.push(t);
                }
            }
        }
    };
    match curve {
        Curve::BSpline(b) => add(b.knots(), &Some),
        Curve::RationalBSpline(b) => add(b.knots(), &Some),
        Curve::Trimmed(t) => {
            let trim = t.trim();
            let map = |s: f64| {
                (s > trim.start && s < trim.start + trim.span).then_some(if trim.reversed {
                    trim.start + trim.span - s
                } else {
                    s - trim.start
                })
            };
            match t.basis() {
                Curve::BSpline(b) => add(b.knots(), &map),
                Curve::RationalBSpline(b) => add(b.knots(), &map),
                _ => {}
            }
        }
        _ => {}
    }
    out.sort_by(f64::total_cmp);
    out.dedup();
    out
}

fn spline_or_approximate(curve: &Curve, tol: f64) -> Result<Done, GpkgError> {
    if piecewise_linear(curve) {
        let mut pts = Vec::new();
        for t in breakpoints(curve) {
            pts.push(ev(curve, t)?);
        }
        let note = "degree one: the polyline through the curve at its knots is the same locus";
        let done = exact(Piece::Line(pts), Rank::Equivalent, note);
        return residual(curve, done);
    }
    if let Curve::RationalBSpline(r) = curve {
        if let Some(done) = rational_arcs(curve, r.degree(), r.knots(), r.control_points(), r.weights())? {
            return Ok(done);
        }
    }
    approximate(curve, tol)
}

/// A clamped quadratic rational spline whose spans are circular arcs, as the circular arcs. Each span
/// is a conic; it is accepted only when the circle through its end points and its middle point is
/// followed by the exact evaluation across the span, to the numeric policy's tolerance. Anything else
/// is left to the approximation.
fn rational_arcs(
    curve: &Curve,
    degree: usize,
    knots: &[f64],
    cps: &[Point2],
    weights: &[f64],
) -> Result<Option<Done>, GpkgError> {
    if degree != 2 || knots.len() != cps.len() + 3 {
        return Ok(None);
    }
    let n = knots.len();
    if !(knots[0] == knots[1]
        && knots[1] == knots[2]
        && knots[n - 1] == knots[n - 2]
        && knots[n - 2] == knots[n - 3])
    {
        return Ok(None);
    }
    // interior knots must each appear exactly twice, so the spans are Bezier arcs sharing end points
    let interior = &knots[3..n - 3];
    if interior.len() % 2 != 0 || interior.chunks(2).any(|c| c[0] != c[1]) {
        return Ok(None);
    }
    let mut edges = vec![knots[0]];
    edges.extend(interior.chunks(2).map(|c| c[0]));
    edges.push(knots[n - 1]);
    if edges.windows(2).any(|w| w[0] >= w[1]) || cps.len() != 2 * (edges.len() - 1) + 1 {
        return Ok(None);
    }
    let mag = magnitude(curve)?;
    let mut pts: Vec<Pt> = vec![pt(cps[0])];
    let mut worst: f64 = 0.0;
    for (k, span) in edges.windows(2).enumerate() {
        let (p0, p1, p2) = (pt(cps[2 * k]), pt(cps[2 * k + 1]), pt(cps[2 * k + 2]));
        let (w0, w1, w2) = (weights[2 * k], weights[2 * k + 1], weights[2 * k + 2]);
        // normalise to end weights of one: the same locus, reparameterised
        let w = w1 / (w0 * w2).sqrt();
        let mid = [
            (p0[0] + 2.0 * w * p1[0] + p2[0]) / (2.0 + 2.0 * w),
            (p0[1] + 2.0 * w * p1[1] + p2[1]) / (2.0 + 2.0 * w),
        ];
        let Some((centre, r)) = circumcircle(p0, mid, p2) else { return Ok(None) };
        let tol = POLICY.tol(r, mag.max(r));
        for s in 0..=64 {
            let t = span[0] + (span[1] - span[0]) * s as f64 / 64.0;
            let q = ev(curve, t)?;
            let off = ((q[0] - centre[0]).hypot(q[1] - centre[1]) - r).abs();
            if off > tol {
                return Ok(None);
            }
            worst = worst.max(off);
        }
        pts.push(mid);
        pts.push(p2);
    }
    Ok(Some(Done {
        pieces: vec![Piece::Arcs(pts)],
        compound: false,
        rank: Rank::Equivalent,
        deviation: Some(worst),
        note: "every span of the rational spline is a circular arc, written as that arc".into(),
    }))
}

// ---- approximation ------------------------------------------------------------------------------

/// The largest of `f` over `[lo, hi]`, from a coarse scan refined around the best sample.
fn peak(mut f: impl FnMut(f64) -> Result<f64, GpkgError>, lo: f64, hi: f64) -> Result<f64, GpkgError> {
    let n = CHORD_SAMPLES;
    let (mut best, mut at) = (f64::NEG_INFINITY, 0);
    for i in 0..=n {
        let v = f(lo + (hi - lo) * i as f64 / n as f64)?;
        if v > best {
            best = v;
            at = i;
        }
    }
    let step = (hi - lo) / n as f64;
    let (mut a, mut b) = ((lo + step * at as f64 - step).max(lo), (lo + step * at as f64 + step).min(hi));
    for _ in 0..3 {
        let mut local = (f64::NEG_INFINITY, a);
        for i in 0..=8 {
            let t = a + (b - a) * i as f64 / 8.0;
            let v = f(t)?;
            if v > local.0 {
                local = (v, t);
            }
        }
        best = best.max(local.0);
        let h = (b - a) / 8.0;
        (a, b) = ((local.1 - h).max(lo), (local.1 + h).min(hi));
    }
    Ok(best)
}

/// The deviation of the chord `pa pb` from the curve over `[a, b]`, in both directions.
fn chord_deviation(curve: &Curve, a: f64, b: f64, pa: Pt, pb: Pt) -> Result<f64, GpkgError> {
    let forward = peak(|t| Ok(segment_distance(pa, pb, ev(curve, t)?)), a, b)?;
    // chord points back to the curve, against a fine local reference polyline of the curve
    let mut reference = Vec::with_capacity(REFERENCE_SEGMENTS + 1);
    for i in 0..=REFERENCE_SEGMENTS {
        reference.push(ev(curve, a + (b - a) * i as f64 / REFERENCE_SEGMENTS as f64)?);
    }
    let mut back: f64 = 0.0;
    for i in 0..=CHORD_SAMPLES {
        let f = i as f64 / CHORD_SAMPLES as f64;
        let q = [pa[0] + f * (pb[0] - pa[0]), pa[1] + f * (pb[1] - pa[1])];
        let d = reference.windows(2).map(|w| segment_distance(w[0], w[1], q)).fold(f64::INFINITY, f64::min);
        back = back.max(d);
    }
    Ok(forward.max(back))
}

fn approximate(curve: &Curve, tol: f64) -> Result<Done, GpkgError> {
    let mag = magnitude(curve)?;
    // below the numeric policy's own tolerance for these coordinates (NP-04) a deviation cannot be told
    // from rounding in the points
    if tol < POLICY.tol(0.0, mag) {
        return Err(GpkgError::ToleranceUnreachable { tolerance: tol });
    }
    let d = curve.domain();
    // the narrowest parameter span worth splitting: the policy's tolerance for a feature of this size
    let narrowest = POLICY.tol(d.width(), d.lo().abs().max(d.hi().abs()));
    let mut params = breakpoints(curve);
    // top up to the minimum number of equal pieces
    for i in 1..INITIAL_PIECES {
        params.push(d.lo() + d.width() * i as f64 / INITIAL_PIECES as f64);
    }
    params.sort_by(f64::total_cmp);
    params.dedup();
    let mut pts: Vec<Pt> = vec![ev(curve, params[0])?];
    let mut worst: f64 = 0.0;
    for w in params.windows(2) {
        let end = ev(curve, w[1])?;
        refine(curve, w[0], w[1], *pts.last().unwrap(), end, tol, narrowest, 0, &mut pts, &mut worst)?;
    }
    if pts.len() > MAX_VERTICES {
        return Err(GpkgError::ToleranceUnreachable { tolerance: tol });
    }
    let bound = worst.max(f64::MIN_POSITIVE);
    Ok(Done {
        pieces: vec![Piece::Line(pts.clone())],
        compound: false,
        rank: Rank::Bounded,
        deviation: Some(bound),
        note: format!("sampled at {} vertices", pts.len()),
    })
}

#[allow(clippy::too_many_arguments)]
fn refine(
    curve: &Curve,
    a: f64,
    b: f64,
    pa: Pt,
    pb: Pt,
    tol: f64,
    narrowest: f64,
    depth: usize,
    pts: &mut Vec<Pt>,
    worst: &mut f64,
) -> Result<(), GpkgError> {
    let dev = chord_deviation(curve, a, b, pa, pb)? * SAFETY;
    let mid = a + 0.5 * (b - a);
    let narrow = !(mid > a && mid < b) || (b - a) <= narrowest || depth >= 60;
    if dev <= tol {
        *worst = worst.max(dev);
        pts.push(pb);
    } else if narrow || pts.len() > MAX_VERTICES {
        return Err(GpkgError::ToleranceUnreachable { tolerance: tol });
    } else {
        let pm = ev(curve, mid)?;
        refine(curve, a, mid, pa, pm, tol, narrowest, depth + 1, pts, worst)?;
        refine(curve, mid, b, pm, pb, tol, narrowest, depth + 1, pts, worst)?;
    }
    Ok(())
}

// ---- composite curves -----------------------------------------------------------------------------

fn composite(segments: &[Curve], tol: f64) -> Result<Outcome, GpkgError> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut rank = Rank::Exact;
    let mut bound: Option<f64> = None;
    let mut residual_max: f64 = 0.0;
    let mut snapped = false;
    for seg in segments {
        let mut d = member(seg, tol)?;
        rank = rank.max(d.rank);
        match (d.rank, d.deviation) {
            (Rank::Bounded, Some(v)) => bound = Some(bound.map_or(v, |b: f64| b.max(v))),
            (_, Some(v)) => residual_max = residual_max.max(v),
            _ => {}
        }
        // pieces of a compound curve have to be connected exactly: the joint is written once
        if let Some(prev) = pieces.last() {
            let end = prev.last();
            let first = &mut d.pieces[0].points_mut()[0];
            if *first != end {
                snapped = true;
                *first = end;
            }
        }
        pieces.append(&mut d.pieces);
    }
    if snapped && rank == Rank::Exact {
        rank = Rank::Repar;
    }
    let deviation = if rank == Rank::Bounded { bound } else { Some(residual_max) };
    Ok(Outcome::Done(Done {
        pieces,
        compound: true,
        rank,
        deviation,
        note: format!("{} segments; the weakest segment decides the claim", segments.len()),
    }))
}
