//! Exact nearest-point queries on curves (issue P7.5), with no tessellation anywhere.
//!
//! A curve is split into pieces. Lines, polyline segments and circular arcs have a closed form. Every
//! other piece (an ellipse, a Bezier curve, one knot span of a B-spline or a rational B-spline) is
//! refined adaptively: an interval `[a, b]` of speed bound `L` whose midpoint is `d` away from the query
//! point cannot hold a point nearer than `d - L (b - a) / 2`, so it is dropped when that is no better than
//! the best distance found, and split otherwise. The speed bound comes from the control points (the
//! derivative of a B-spline is a B-spline on the differences), so the pruning is a proof and not a
//! sampling guess. A Newton step on `(C(t) - p) . C'(t)` then polishes the parameter.

use pvec_geometry::{Curve, Curve2d, Placement2, Point2, Side};
use pvec_numeric::Policy;

/// The nearest point of a curve: its distance and its parameter under the curve's own domain.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nearest {
    pub(crate) distance: f64,
    pub(crate) param: f64,
}

/// The most midpoint evaluations one query spends on one curve, so a hostile curve cannot stall a pick.
const BUDGET: usize = 200_000;

/// The most leaves one smooth piece polishes.
const MAX_LEAVES: usize = 4096;

/// Maps a piece's own parameter to the entity curve's parameter: `top = offset + scale * own`.
#[derive(Clone, Copy)]
struct Map {
    offset: f64,
    scale: f64,
}

impl Map {
    const IDENTITY: Map = Map { offset: 0.0, scale: 1.0 };
    fn apply(self, t: f64) -> f64 {
        self.offset + self.scale * t
    }
    /// `self` after `inner`.
    fn after(self, inner: Map) -> Map {
        Map { offset: self.offset + self.scale * inner.offset, scale: self.scale * inner.scale }
    }
}

enum Piece<'a> {
    /// The straight segment `a..b`, own parameter `s0..s1`.
    Segment { a: Point2, b: Point2, s0: f64, s1: f64, map: Map },
    /// The arc `lo..hi` of a circle (own parameter is the angle).
    Arc { frame: &'a Placement2, radius: f64, lo: f64, hi: f64, map: Map },
    /// Anything refined adaptively.
    Smooth { curve: &'a Curve, lo: f64, hi: f64, speed: f64, map: Map },
}

fn dist(a: Point2, b: Point2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// Appends the pieces of `curve` restricted to its own parameter range `range` (its whole domain when
/// `None`).
fn pieces<'a>(curve: &'a Curve, range: Option<(f64, f64)>, map: Map, out: &mut Vec<Piece<'a>>) {
    let (dlo, dhi) = (curve.domain().lo(), curve.domain().hi());
    let (lo, hi) = range.map_or((dlo, dhi), |(a, b)| (a.max(dlo), b.min(dhi)));
    match curve {
        Curve::Line(l) => {
            let (o, d) = (l.origin(), l.direction());
            let at = |s: f64| Point2::new(o.x + s * d.x, o.y + s * d.y);
            out.push(Piece::Segment { a: at(lo), b: at(hi), s0: lo, s1: hi, map });
        }
        Curve::Circle(c) => {
            out.push(Piece::Arc { frame: c.placement(), radius: c.radius(), lo, hi, map });
        }
        Curve::Ellipse(e) => {
            let speed = e.semi_axis_1().max(e.semi_axis_2());
            out.push(Piece::Smooth { curve, lo, hi, speed, map });
        }
        Curve::Polyline(p) => {
            for (i, w) in p.vertices().windows(2).enumerate() {
                let s0 = i as f64;
                out.push(Piece::Segment { a: w[0], b: w[1], s0, s1: s0 + 1.0, map });
            }
        }
        Curve::Trimmed(t) => {
            let tr = t.trim();
            let inner = if tr.reversed {
                Map { offset: tr.start + tr.span, scale: -1.0 }
            } else {
                Map { offset: -tr.start, scale: 1.0 }
            };
            // `inner` takes the basis parameter to the trimmed parameter; the entity map sees that one.
            pieces(t.basis(), Some((tr.start, tr.start + tr.span)), map.after(inner), out);
        }
        Curve::Composite(c) => {
            for (k, seg) in c.segments().iter().enumerate() {
                let d = seg.domain();
                let w = d.width();
                // composite parameter u = k + (s - lo) / w
                let inner = Map { offset: k as f64 - d.lo() / w, scale: 1.0 / w };
                pieces(seg, None, map.after(inner), out);
            }
        }
        Curve::Bezier(b) => {
            let cps = b.control_points();
            let speed = b.degree() as f64 * cps.windows(2).map(|w| dist(w[0], w[1])).fold(0.0, f64::max);
            out.push(Piece::Smooth { curve, lo, hi, speed, map });
        }
        Curve::BSpline(s) => {
            spans(curve, s.degree(), s.knots(), s.control_points(), None, (lo, hi), map, out)
        }
        Curve::RationalBSpline(s) => {
            spans(curve, s.degree(), s.knots(), s.control_points(), Some(s.weights()), (lo, hi), map, out);
        }
        // The curve enum is non-exhaustive; a kind added later has no hit test until it gets one, and
        // is then never picked rather than picked wrongly.
        _ => {}
    }
}

/// One `Smooth` piece per non-empty knot span, each with the speed bound of its own control points.
#[allow(clippy::too_many_arguments)]
fn spans<'a>(
    curve: &'a Curve,
    p: usize,
    knots: &[f64],
    cps: &[Point2],
    weights: Option<&[f64]>,
    (lo, hi): (f64, f64),
    map: Map,
    out: &mut Vec<Piece<'a>>,
) {
    for k in p..cps.len() {
        let (a, b) = (knots[k].max(lo), knots[k + 1].min(hi));
        if knots[k] >= knots[k + 1] || a >= b {
            continue;
        }
        // The derivative control points of the span: i = k-p ..= k-1, over knot gaps U[i+p+1] - U[i+1].
        let (mut dp, mut dw, mut wmin, mut pmax) = (0.0f64, 0.0f64, f64::INFINITY, 0.0f64);
        for i in (k - p)..k {
            let gap = knots[i + p + 1] - knots[i + 1];
            if gap <= 0.0 {
                continue;
            }
            let f = p as f64 / gap;
            match weights {
                None => dp = dp.max(f * dist(cps[i], cps[i + 1])),
                Some(w) => {
                    let (dx, dy) = (
                        w[i + 1] * cps[i + 1].x - w[i] * cps[i].x,
                        w[i + 1] * cps[i + 1].y - w[i] * cps[i].y,
                    );
                    dp = dp.max(f * dx.hypot(dy));
                    dw = dw.max(f * (w[i + 1] - w[i]).abs());
                }
            }
        }
        let speed = match weights {
            None => dp,
            Some(w) => {
                for j in (k - p)..=k {
                    wmin = wmin.min(w[j]);
                    pmax = pmax.max(cps[j].x.hypot(cps[j].y));
                }
                (dp + pmax * dw) / wmin
            }
        };
        out.push(Piece::Smooth { curve, lo: a, hi: b, speed, map });
    }
}

struct Best {
    distance: f64,
    param: f64,
}

fn closest_on_segment(p: Point2, a: Point2, b: Point2) -> (f64, f64) {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let u = if len2 > 0.0 { (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (dist(p, Point2::new(a.x + u * dx, a.y + u * dy)), u)
}

/// The nearest point of `curve` to `p` at or under `cutoff`, or `None` when nothing is that close.
/// `scale` is a length of the order of the curve's size, which sets the absolute accuracy.
pub(crate) fn nearest(curve: &Curve, p: Point2, cutoff: f64, scale: f64) -> Option<Nearest> {
    let mut all = Vec::new();
    pieces(curve, None, Map::IDENTITY, &mut all);
    // Newton squares the error of a leaf, so a leaf of the square root of the policy's relative tolerance
    // is polished to the policy tolerance itself.
    let leaf_eps = Policy::V1.rel.sqrt() * (scale + dist(p, Point2::new(0.0, 0.0)));
    let mut best = Best { distance: cutoff, param: f64::NAN };
    let mut found = false;
    let mut budget = BUDGET;
    for piece in &all {
        match *piece {
            Piece::Segment { a, b, s0, s1, map } => {
                let (d, u) = closest_on_segment(p, a, b);
                if d <= best.distance && (!found || d < best.distance) {
                    best = Best { distance: d, param: map.apply(s0 + u * (s1 - s0)) };
                    found = true;
                }
            }
            Piece::Arc { frame, radius, lo, hi, map } => {
                let (d, s) = closest_on_arc(p, frame, radius, lo, hi);
                if d <= best.distance && (!found || d < best.distance) {
                    best = Best { distance: d, param: map.apply(s) };
                    found = true;
                }
            }
            Piece::Smooth { curve, lo, hi, speed, map } => {
                refine(curve, p, (lo, hi), speed, map, leaf_eps, &mut budget, &mut best, &mut found);
            }
        }
    }
    found.then_some(Nearest { distance: best.distance, param: best.param })
}

fn closest_on_arc(p: Point2, f: &Placement2, r: f64, lo: f64, hi: f64) -> (f64, f64) {
    use core::f64::consts::TAU;
    let (o, x, y) = (f.origin(), f.x_axis(), f.y_axis());
    let (vx, vy) = (p.x - o.x, p.y - o.y);
    let (lx, ly) = (vx * x.x + vy * x.y, vx * y.x + vy * y.y);
    let rho = lx.hypot(ly);
    let at = |s: f64| {
        let (sn, cs) = s.sin_cos();
        Point2::new(o.x + r * (cs * x.x + sn * y.x), o.y + r * (cs * x.y + sn * y.y))
    };
    if rho == 0.0 {
        return (r, lo);
    }
    let offset = (ly.atan2(lx) - lo).rem_euclid(TAU);
    if offset <= hi - lo {
        return ((rho - r).abs(), lo + offset);
    }
    let (d0, d1) = (dist(p, at(lo)), dist(p, at(hi)));
    if d0 <= d1 { (d0, lo) } else { (d1, hi) }
}

/// Branch and bound over `[lo, hi]` of one smooth piece.
///
/// An interval is dropped when its lower bound `d - speed * half` cannot beat the best distance. One whose
/// bound is already within `leaf_eps` is not split further (near a smooth minimum the linear bound would
/// keep splitting without end) but kept as a leaf. The leaves that can still win are then polished by
/// Newton steps, which converge in a few evaluations because a leaf is tiny.
#[allow(clippy::too_many_arguments)]
fn refine(
    curve: &Curve,
    p: Point2,
    (lo, hi): (f64, f64),
    speed: f64,
    map: Map,
    leaf_eps: f64,
    budget: &mut usize,
    best: &mut Best,
    found: &mut bool,
) {
    // (lower bound, interval start, interval end, midpoint, midpoint distance)
    let mut leaves: Vec<(f64, f64, f64, f64, f64)> = Vec::new();
    let mut stack = vec![(lo, hi)];
    while let Some((a, b)) = stack.pop() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let m = 0.5 * (a + b);
        let Ok(q) = curve.eval(m) else { continue };
        let d = dist(p, q);
        if d <= best.distance && (!*found || d < best.distance) {
            *best = Best { distance: d, param: map.apply(m) };
            *found = true;
        }
        let half = 0.5 * (b - a);
        let lower = d - speed * half;
        if lower >= best.distance {
            continue;
        }
        if speed * half <= leaf_eps || m <= a || m >= b {
            leaves.push((lower, a, b, m, d));
            continue;
        }
        stack.push((a, m));
        stack.push((m, b));
    }
    leaves.sort_by(|x, y| x.0.total_cmp(&y.0));
    for &(lower, a, b, m, d) in leaves.iter().take(MAX_LEAVES) {
        if lower >= best.distance {
            break;
        }
        let (t, nd) = polish(curve, p, m, a, b, d);
        if nd < best.distance || (!*found && nd <= best.distance) {
            *best = Best { distance: nd, param: map.apply(t) };
            *found = true;
        }
    }
}

/// A few Newton steps on `(C(t) - p) . C'(t) = 0`, kept inside `[lo, hi]` and only accepted when the
/// distance goes down.
fn polish(curve: &Curve, p: Point2, mut t: f64, lo: f64, hi: f64, mut d: f64) -> (f64, f64) {
    for _ in 0..8 {
        let (Ok(c), Ok(d1), Ok(d2)) =
            (curve.eval(t), curve.derivative(t, 1, Side::Both), curve.derivative(t, 2, Side::Both))
        else {
            break;
        };
        let (rx, ry) = (c.x - p.x, c.y - p.y);
        let g = rx * d1.x + ry * d1.y;
        let h = d1.x * d1.x + d1.y * d1.y + rx * d2.x + ry * d2.y;
        if h <= 0.0 || !g.is_finite() {
            break;
        }
        let next = (t - g / h).clamp(lo, hi);
        let Ok(q) = curve.eval(next) else { break };
        let nd = dist(p, q);
        if nd < d {
            t = next;
            d = nd;
        } else {
            break;
        }
    }
    (t, d)
}

/// The nearest point of the polyline `pts` (a Tier 0 preview) to `p`, at or under `cutoff`.
pub(crate) fn nearest_on_polyline(pts: &[[f64; 2]], p: Point2, cutoff: f64) -> Option<f64> {
    let q = |i: usize| Point2::new(pts[i][0], pts[i][1]);
    let d = if pts.len() == 1 {
        dist(p, q(0))
    } else {
        (0..pts.len() - 1).map(|i| closest_on_segment(p, q(i), q(i + 1)).0).fold(f64::INFINITY, f64::min)
    };
    (d <= cutoff).then_some(d)
}
