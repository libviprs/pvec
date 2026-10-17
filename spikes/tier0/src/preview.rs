//! Preview forms and their measured deviation.
//!
//! A preview is a polyline (or a closed rectangle) that stands in for one deferred entity until the
//! exact entity arrives. It is display fidelity only. Its error is *measured* against the P1.3
//! reference evaluator as a two-sided Hausdorff distance, never judged by eye.

use geometry_api_spike::{Curve, Curve2d};
use pvec_fixtures::{Geom, Pt};

/// Upper bound on vertices in one preview. Bounds the allocation a hostile stream can ask for.
pub const MAX_PREVIEW_VERTS: usize = 4096;
/// Depth limit of the adaptive subdivision.
const MAX_DEPTH: u32 = 24;

/// Which form a preview takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Form {
    /// A single vertex at the centre of the bounds: the stand-in for an entity below the budget.
    PointStub,
    /// Axis-aligned bounding rectangle outline.
    BboxStub,
    /// The control polygon (control points for splines, tangent-intersection points for conics).
    ControlPolygon,
    /// A polyline refined until its chord sagitta is within the budget.
    SagittaPolyline,
}

impl Form {
    /// Stable on-wire code.
    pub fn code(self) -> u8 {
        match self {
            Form::BboxStub => 0,
            Form::ControlPolygon => 1,
            Form::SagittaPolyline => 2,
            Form::PointStub => 3,
        }
    }
    /// Inverse of [`Form::code`].
    pub fn from_code(c: u8) -> Option<Form> {
        [Form::BboxStub, Form::ControlPolygon, Form::SagittaPolyline, Form::PointStub]
            .into_iter()
            .find(|f| f.code() == c)
    }
}

/// A preview of one deferred entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    /// The file-local id of the exact entity that supersedes this preview. This is the whole
    /// replacement identity: preview and exact resolve to the same id by construction.
    pub replaces: u32,
    /// The form.
    pub form: Form,
    /// True when the last vertex connects back to the first (the bounding stub).
    pub closed: bool,
    /// Offset all vertices are stored relative to.
    pub origin: Pt,
    /// Vertices as stored: `f32` offsets from `origin`.
    pub verts: Vec<[f32; 2]>,
}

impl Preview {
    /// Packed size: id (4) + form (1) + flags (1) + count (2) + origin (16) + 8 per vertex.
    pub fn bytes(&self) -> usize {
        24 + 8 * self.verts.len()
    }
    /// The vertices as they will be drawn, after the `f32` quantisation.
    pub fn points(&self) -> Vec<Pt> {
        self.verts.iter().map(|v| [self.origin[0] + v[0] as f64, self.origin[1] + v[1] as f64]).collect()
    }
}

/// Quantises world vertices into the stored form.
pub fn quantise(replaces: u32, form: Form, closed: bool, pts: &[Pt]) -> Preview {
    let mut lo = [f64::INFINITY; 2];
    for p in pts {
        lo[0] = lo[0].min(p[0]);
        lo[1] = lo[1].min(p[1]);
    }
    let verts = pts.iter().map(|p| [(p[0] - lo[0]) as f32, (p[1] - lo[1]) as f32]).collect();
    Preview { replaces, form, closed, origin: lo, verts }
}

fn seg_dist(p: Pt, a: Pt, b: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

fn dist_to_poly(p: Pt, v: &[Pt], closed: bool) -> f64 {
    if v.len() == 1 {
        return (p[0] - v[0][0]).hypot(p[1] - v[0][1]);
    }
    let mut d = f64::INFINITY;
    for w in v.windows(2) {
        d = d.min(seg_dist(p, w[0], w[1]));
    }
    if closed {
        d = d.min(seg_dist(p, v[v.len() - 1], v[0]));
    }
    d
}

/// Dense samples of the reference curve, used as ground truth for deviation.
pub fn dense_samples(c: &Curve, n: usize) -> Vec<Pt> {
    let (lo, hi) = c.domain();
    (0..=n).filter_map(|i| c.eval(lo + (hi - lo) * i as f64 / n as f64).ok()).collect()
}

/// Two-sided Hausdorff distance, in world units, between the reference curve and a preview.
///
/// Exact side: `exact_dense` points against the preview polyline. Preview side: 6 points per preview
/// segment (and each vertex) against the polyline through `exact_dense`. The dense polyline carries
/// its own chord error, which is far below the budgets measured here and is not subtracted.
pub fn deviation(exact_dense: &[Pt], prev: &[Pt], closed: bool) -> f64 {
    let mut worst = 0.0f64;
    for &p in exact_dense {
        worst = worst.max(dist_to_poly(p, prev, closed));
    }
    let mut segs: Vec<[Pt; 2]> = prev.windows(2).map(|w| [w[0], w[1]]).collect();
    if closed && prev.len() > 1 {
        segs.push([prev[prev.len() - 1], prev[0]]);
    }
    let mut check = |p: Pt| worst = worst.max(dist_to_poly(p, exact_dense, false));
    for w in &segs {
        for k in 0..=6 {
            let t = k as f64 / 6.0;
            check([w[0][0] + t * (w[1][0] - w[0][0]), w[0][1] + t * (w[1][1] - w[0][1])]);
        }
    }
    if prev.len() == 1 {
        check(prev[0]);
    }
    worst
}

/// A polyline whose chord sagitta is within `tol` (world units), by adaptive bisection on the
/// reference curve. Returns `None` when the vertex cap is reached or the evaluator refuses.
pub fn sagitta_polyline(c: &Curve, tol: f64, init_hint: usize) -> Option<Vec<Pt>> {
    let (lo, hi) = c.domain();
    // Start from enough pieces that a chord cannot straddle a whole symmetric curve, and cut a
    // composite at its joints.
    let init = if matches!(c, Curve::Composite(_)) { (hi - lo) as usize * 4 } else { init_hint.max(8) };
    let mut out = vec![c.eval(lo).ok()?];
    for i in 0..init {
        let t0 = lo + (hi - lo) * i as f64 / init as f64;
        let t1 = lo + (hi - lo) * (i + 1) as f64 / init as f64;
        split(c, t0, t1, tol, 0, &mut out)?;
    }
    Some(out)
}

fn split(c: &Curve, t0: f64, t1: f64, tol: f64, depth: u32, out: &mut Vec<Pt>) -> Option<()> {
    let p0 = *out.last()?;
    let p1 = c.eval(t1).ok()?;
    let mut bad = false;
    for f in [0.25, 0.5, 0.75] {
        let q = c.eval(t0 + (t1 - t0) * f).ok()?;
        if seg_dist(q, p0, p1) > 0.9 * tol {
            bad = true;
        }
    }
    if bad && depth < MAX_DEPTH {
        let m = 0.5 * (t0 + t1);
        split(c, t0, m, tol, depth + 1, out)?;
        split(c, m, t1, tol, depth + 1, out)
    } else {
        if out.len() >= MAX_PREVIEW_VERTS {
            return None;
        }
        out.push(p1);
        Some(())
    }
}

/// Control polygon of an elliptic arc (rotation `rot`, eccentric angles `t0..t1`): the arc's end
/// points and the intersections of its end tangents, one span per at most a quarter turn.
fn conic_polygon(c: Pt, a: f64, b: f64, rot: f64, t0: f64, t1: f64) -> Vec<Pt> {
    let (s, co) = rot.sin_cos();
    let at = |x: f64, y: f64| [c[0] + co * x - s * y, c[1] + s * x + co * y];
    let n = (((t1 - t0) / core::f64::consts::FRAC_PI_2).ceil() as usize).max(1);
    let mut v = vec![at(a * t0.cos(), b * t0.sin())];
    for k in 0..n {
        let (ta, tb) = (t0 + (t1 - t0) * k as f64 / n as f64, t0 + (t1 - t0) * (k + 1) as f64 / n as f64);
        let (tm, h) = (0.5 * (ta + tb), 0.5 * (tb - ta));
        v.push(at(a * tm.cos() / h.cos(), b * tm.sin() / h.cos()));
        v.push(at(a * tb.cos(), b * tb.sin()));
    }
    v
}

/// The control polygon of a deferred geometry, or `None` where the form does not apply.
pub fn control_polygon(g: &Geom) -> Option<Vec<Pt>> {
    match g {
        Geom::Spline { cps, .. } => Some(cps.clone()),
        Geom::Ellipse { c, a, b, rot, t0, t1 } => Some(conic_polygon(*c, *a, *b, *rot, *t0, *t1)),
        Geom::Composite { parts } => {
            let mut v: Vec<Pt> = Vec::new();
            for p in parts {
                let piece = match p {
                    Geom::Line { a, b } => vec![*a, *b],
                    Geom::Arc { c, r, a0, a1 } => conic_polygon(*c, *r, *r, 0.0, *a0, *a1),
                    _ => return None,
                };
                v.extend(piece);
            }
            Some(v)
        }
        _ => None,
    }
}

/// The point stub: the centre of the bounds.
pub fn point_stub(g: &Geom) -> Vec<Pt> {
    let b = g.bbox();
    vec![[0.5 * (b[0] + b[2]), 0.5 * (b[1] + b[3])]]
}

/// How many initial pieces the adaptive polyline starts from: four per knot span of a spline, so a
/// long spline is not judged from three samples across its whole length.
pub fn init_pieces(g: &Geom) -> usize {
    match g {
        Geom::Spline { degree, cps, .. } => 4 * (cps.len() - *degree as usize),
        _ => 8,
    }
}

/// The bounding stub: the closed outline of the entity's bounds.
pub fn bbox_stub(g: &Geom) -> Vec<Pt> {
    let b = g.bbox();
    vec![[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]]
}
