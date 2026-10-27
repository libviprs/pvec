//! The Tier 0 complete-coverage stream (issue P5.4, pvec#13).
//!
//! Tier 0 is the first thing a reader paints. Every entity of the artifact is in it, either `Exact`
//! (cheap geometry, which lives in its hot block and is named here by id) or as a `Preview`: a
//! bounded-error stand-in that carries the `EntityId` of the entity it stands for. The error bound is
//! measured by the writer against the exact curve (through `pvec-geometry`, the one evaluator) and stored
//! beside the preview. A reader never infers it. Previews are display fidelity only and are never
//! reported as exact. The design, the stream layout and the infeasibility list are in
//! `docs/STREAMING.md` chapter 4.

use crate::generation::{Applied, DropCounts, DropReason, Generation, Refinement, RefinementResult, Update};
use pvec_entity::{EntityError, EntityId, check_dense};
use pvec_geometry::{Curve, Curve2d, GeometryError, Point2};
use pvec_numeric::MAX_ENTITIES;
use std::collections::BTreeMap;

/// The most vertices one preview may hold.
pub const MAX_PREVIEW_VERTICES: usize = 4096;

/// Most vertices of a polyline that still counts as cheap exact geometry.
pub const EXACT_POLYLINE_VERTICES: usize = 64;

/// The magic of a Tier 0 stream.
const MAGIC: [u8; 4] = *b"PVT0";
/// The stream version of a whole-artifact plan.
const VERSION: u8 = 1;
/// The stream version of the plan of one leaf: the same layout, covering some of the artifact's entities.
const LEAF_VERSION: u8 = 2;
/// Header: magic, version, three zero bytes, then four little-endian `u32` counts.
const HEADER_LEN: usize = 24;
/// Bytes of a preview record before its vertices: id, form, count, bound, origin.
const PREVIEW_FIXED: usize = 4 + 1 + 2 + 4 + 16;
/// Bytes of a vertex.
const VERTEX_LEN: usize = 8;
/// Bytes of an infeasible record: id and reason.
const INFEASIBLE_LEN: usize = 5;
/// Bytes of an exact run: start and length.
const RUN_LEN: usize = 8;

/// The margin kept over a measured deviation, because the measurement samples the curve.
const BOUND_MARGIN: f64 = 1.1;
/// What the stored bound adds for floating point rounding, per unit of the preview's own extent (an `f32`
/// carries about 0.00000006 of its value, and the vertices are stored as offsets from an `f64` origin, so
/// the value that is rounded is the offset and not the absolute coordinate). It is a rounding allowance,
/// not a geometric tolerance.
const ROUNDING_SLACK: f64 = 0.0000001;
/// The share of the budget the chord polyline is built to, before measuring.
const CHORD_TARGETS: [f64; 3] = [0.5, 0.25, 0.1];
/// The depth the chord polyline gives up at.
const MAX_DEPTH: u32 = 24;
/// Fewest and most samples of the exact curve one measurement takes.
const MIN_SAMPLES: usize = 512;
const MAX_SAMPLES: usize = 8192;

/// The shape a preview takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreviewForm {
    /// The control polygon of the curve.
    ControlPolygon,
    /// A polyline whose chords stay within the error bound of the curve.
    ChordPolyline,
}

impl PreviewForm {
    fn code(self) -> u8 {
        match self {
            PreviewForm::ControlPolygon => 0,
            PreviewForm::ChordPolyline => 1,
        }
    }
    fn from_code(code: u8) -> Option<PreviewForm> {
        match code {
            0 => Some(PreviewForm::ControlPolygon),
            1 => Some(PreviewForm::ChordPolyline),
            _ => None,
        }
    }
}

/// How faithful a Tier 0 entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// The entity's own geometry.
    Exact,
    /// A bounded-error stand-in, never reported as exact.
    Display,
}

impl Fidelity {
    /// The label conformance output prints.
    pub fn label(self) -> &'static str {
        match self {
            Fidelity::Exact => "exact",
            Fidelity::Display => "display-fidelity",
        }
    }
}

/// Why no bounded preview could be made for an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InfeasibleReason {
    /// The exact evaluator refused the curve, so there is nothing to measure a preview against.
    EvaluatorRefused,
    /// No form met the error bound within `MAX_PREVIEW_VERTICES` vertices.
    VertexCap,
    /// A preview could be drawn, but its measured deviation, after quantisation, is over the budget.
    BoundNotMet,
}

impl InfeasibleReason {
    fn code(self) -> u8 {
        match self {
            InfeasibleReason::EvaluatorRefused => 0,
            InfeasibleReason::VertexCap => 1,
            InfeasibleReason::BoundNotMet => 2,
        }
    }
    fn from_code(code: u8) -> Option<InfeasibleReason> {
        match code {
            0 => Some(InfeasibleReason::EvaluatorRefused),
            1 => Some(InfeasibleReason::VertexCap),
            2 => Some(InfeasibleReason::BoundNotMet),
            _ => None,
        }
    }
}

/// An entity declared infeasible for a bounded preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Infeasible {
    /// The entity.
    pub id: EntityId,
    /// Why.
    pub reason: InfeasibleReason,
}

/// Why a Tier 0 plan was refused or could not be read.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Tier0Error {
    /// This entity of the artifact is not in Tier 0.
    Gap(EntityId),
    /// This entity is in Tier 0 twice.
    Duplicate(EntityId),
    /// This id is not an entity of the artifact.
    OutOfRange(EntityId),
    /// A bound or parameter is not usable.
    BadBound,
    /// The stream is malformed.
    Format {
        /// What is wrong.
        reason: &'static str,
    },
    /// A count or length is over a limit.
    Limit,
    /// The exact geometry does not match the preview it is meant to replace.
    Mismatch(EntityId),
    /// The entity is already exact with different geometry.
    Conflict(EntityId),
    /// The scene has no such entity.
    Unknown(EntityId),
    /// The exact evaluator refused.
    Geometry(GeometryError),
    /// A Tier 0 block is in a cell the spatial directory does not have.
    NoLeaf(u32),
    /// A Tier 0 block is in cell 0 or in a straddler payload cell.
    BadCell(u32),
    /// More than one Tier 0 block for one leaf.
    DuplicateLeaf(u32),
    /// A Tier 0 block's entry disagrees with its leaf: the entity count or the bounds.
    LeafMismatch(u32),
    /// The container refused to read a Tier 0 block.
    Read(String),
}

impl Tier0Error {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            Tier0Error::Gap(_) => "TIER0-01",
            Tier0Error::Duplicate(_) => "TIER0-02",
            Tier0Error::OutOfRange(_) => "TIER0-03",
            Tier0Error::BadBound => "TIER0-04",
            Tier0Error::Format { .. } => "TIER0-05",
            Tier0Error::Limit => "TIER0-06",
            Tier0Error::Mismatch(_) => "TIER0-07",
            Tier0Error::Conflict(_) => "TIER0-08",
            Tier0Error::Unknown(_) => "TIER0-09",
            Tier0Error::NoLeaf(_) => "TIER0-10",
            Tier0Error::BadCell(_) => "TIER0-11",
            Tier0Error::DuplicateLeaf(_) => "TIER0-12",
            Tier0Error::LeafMismatch(_) => "TIER0-13",
            Tier0Error::Read(_) => "TIER0-14",
            Tier0Error::Geometry(e) => e.rule(),
        }
    }
}

impl core::fmt::Display for Tier0Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            Tier0Error::Gap(id) => write!(f, "entity {id} is not in Tier 0 ({rule})"),
            Tier0Error::Duplicate(id) => write!(f, "entity {id} is in Tier 0 twice ({rule})"),
            Tier0Error::OutOfRange(id) => write!(f, "entity {id} is not in the artifact ({rule})"),
            Tier0Error::BadBound => write!(f, "an error bound or budget is not usable ({rule})"),
            Tier0Error::Format { reason } => write!(f, "malformed Tier 0 stream: {reason} ({rule})"),
            Tier0Error::Limit => write!(f, "a Tier 0 count is over its limit ({rule})"),
            Tier0Error::Mismatch(id) => write!(f, "the geometry does not match the preview of {id} ({rule})"),
            Tier0Error::Conflict(id) => write!(f, "{id} is already exact with different geometry ({rule})"),
            Tier0Error::Unknown(id) => write!(f, "no entity {id} in the scene ({rule})"),
            Tier0Error::Geometry(e) => write!(f, "the evaluator refused: {e} ({rule})"),
            Tier0Error::NoLeaf(c) => write!(f, "a Tier 0 block is in cell {c}, which is no leaf ({rule})"),
            Tier0Error::BadCell(c) => write!(f, "a Tier 0 block cannot be in cell {c} ({rule})"),
            Tier0Error::DuplicateLeaf(c) => write!(f, "leaf {c} has more than one Tier 0 block ({rule})"),
            Tier0Error::LeafMismatch(c) => {
                write!(f, "the Tier 0 block of leaf {c} is not its leaf's ({rule})")
            }
            Tier0Error::Read(e) => write!(f, "the container refused a Tier 0 block: {e} ({rule})"),
        }
    }
}

impl std::error::Error for Tier0Error {}

impl From<GeometryError> for Tier0Error {
    fn from(e: GeometryError) -> Tier0Error {
        Tier0Error::Geometry(e)
    }
}

fn fmt_err(reason: &'static str) -> Tier0Error {
    Tier0Error::Format { reason }
}

/// A bounded-error stand-in for one entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    replaces: EntityId,
    form: PreviewForm,
    error_bound: f32,
    origin: [f64; 2],
    vertices: Vec<[f32; 2]>,
}

impl Preview {
    /// A preview from its parts, checked: the bound finite and above zero, between one and
    /// `MAX_PREVIEW_VERTICES` vertices, all finite.
    pub fn new(
        replaces: EntityId,
        form: PreviewForm,
        error_bound: f32,
        origin: [f64; 2],
        vertices: Vec<[f32; 2]>,
    ) -> Result<Preview, Tier0Error> {
        if !(error_bound.is_finite() && error_bound > 0.0) {
            return Err(Tier0Error::BadBound);
        }
        if vertices.is_empty() || vertices.len() > MAX_PREVIEW_VERTICES {
            return Err(Tier0Error::Limit);
        }
        if !origin.iter().all(|c| c.is_finite())
            || !vertices.iter().all(|v| v[0].is_finite() && v[1].is_finite())
        {
            return Err(fmt_err("a preview coordinate is not finite"));
        }
        Ok(Preview { replaces, form, error_bound, origin, vertices })
    }
    /// The entity this preview stands for, and the only identity it has.
    pub fn replaces(&self) -> EntityId {
        self.replaces
    }
    /// The form.
    pub fn form(&self) -> PreviewForm {
        self.form
    }
    /// The stored world-space error bound.
    pub fn error_bound(&self) -> f64 {
        f64::from(self.error_bound)
    }
    /// The origin the stored vertices are offsets from.
    pub fn origin(&self) -> [f64; 2] {
        self.origin
    }
    /// The stored vertices, as `f32` offsets from the origin.
    pub fn offsets(&self) -> &[[f32; 2]] {
        &self.vertices
    }
    /// The vertices in world space, after the stored `f32` quantisation.
    pub fn points(&self) -> Vec<[f64; 2]> {
        self.vertices
            .iter()
            .map(|v| [self.origin[0] + f64::from(v[0]), self.origin[1] + f64::from(v[1])])
            .collect()
    }
    /// Always `Fidelity::Display`.
    pub fn fidelity(&self) -> Fidelity {
        Fidelity::Display
    }
    /// Always false: a preview is never exact.
    pub fn is_exact(&self) -> bool {
        false
    }
}

/// One entry of Tier 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tier0Entry<'a> {
    /// Cheap geometry, emitted in full in its hot block.
    Exact {
        /// The entity.
        id: EntityId,
    },
    /// A bounded-error preview.
    Preview(&'a Preview),
}

impl Tier0Entry<'_> {
    /// The entity this entry stands for.
    pub fn id(&self) -> EntityId {
        match self {
            Tier0Entry::Exact { id } => *id,
            Tier0Entry::Preview(p) => p.replaces(),
        }
    }
    /// How faithful the entry is.
    pub fn fidelity(&self) -> Fidelity {
        match self {
            Tier0Entry::Exact { .. } => Fidelity::Exact,
            Tier0Entry::Preview(p) => p.fidelity(),
        }
    }
}

/// The error budget a writer holds Tier 0 previews to, in pixels at the fit zoom of a 1,920 px screen
/// (`docs/STREAMING.md` section 4.3). A caller turns it into world units with the extent of the drawing:
/// `PREVIEW_BUDGET_PX * extent / 1920`. Tuned in P8.6 (`docs/bench/TUNING.md`).
pub const PREVIEW_BUDGET_PX: f64 = 0.5;

/// What the writer is asked to hold previews to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier0Params {
    /// The most a preview may deviate from its curve, in world units.
    pub max_error: f64,
}

impl Tier0Params {
    fn check(&self) -> Result<(), Tier0Error> {
        if self.max_error.is_finite() && self.max_error > 0.0 { Ok(()) } else { Err(Tier0Error::BadBound) }
    }
}

/// What a conformance report says about Tier 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conformance {
    /// Entities in Tier 0 as exact.
    pub exact: usize,
    /// Entities in Tier 0 as previews, labelled display fidelity.
    pub display_fidelity: usize,
    /// Entities declared infeasible.
    pub infeasible: usize,
}

/// The Tier 0 plan of one artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier0Plan {
    entities: u32,
    /// Ascending, non-overlapping `(start, length)` runs of exact ids.
    runs: Vec<(u32, u32)>,
    previews: Vec<Preview>,
    infeasible: Vec<Infeasible>,
    /// Covers the entities of one leaf and not the artifact (stream version 2).
    partial: bool,
}

/// Assembles a plan and checks coverage when it finishes.
#[derive(Debug)]
pub struct Tier0Builder {
    entities: u32,
    params: Tier0Params,
    seen: Vec<u64>,
    exact: Vec<u32>,
    previews: Vec<Preview>,
    infeasible: Vec<Infeasible>,
}

impl Tier0Builder {
    /// A builder for an artifact of `entities` entities, ids `0..entities`.
    pub fn new(entities: u32, params: Tier0Params) -> Result<Tier0Builder, Tier0Error> {
        params.check()?;
        if entities as usize > MAX_ENTITIES {
            return Err(Tier0Error::Limit);
        }
        Ok(Tier0Builder {
            entities,
            params,
            seen: vec![0; (entities as usize).div_ceil(64)],
            exact: Vec::new(),
            previews: Vec::new(),
            infeasible: Vec::new(),
        })
    }
    fn claim(&mut self, id: EntityId) -> Result<(), Tier0Error> {
        let i = id.index();
        if i >= self.entities {
            return Err(Tier0Error::OutOfRange(id));
        }
        let (word, bit) = (i as usize / 64, 1u64 << (i % 64));
        if self.seen[word] & bit != 0 {
            return Err(Tier0Error::Duplicate(id));
        }
        self.seen[word] |= bit;
        Ok(())
    }
    /// Records an entity whose cheap geometry goes out exact.
    pub fn add_exact(&mut self, id: EntityId) -> Result<(), Tier0Error> {
        self.claim(id)?;
        self.exact.push(id.index());
        Ok(())
    }
    /// Records a ready preview. Its bound must not exceed the budget.
    pub fn add_preview(&mut self, preview: Preview) -> Result<(), Tier0Error> {
        if preview.error_bound() > self.params.max_error {
            return Err(Tier0Error::BadBound);
        }
        self.claim(preview.replaces())?;
        self.previews.push(preview);
        Ok(())
    }
    /// Classifies a curve: cheap geometry is exact, anything else gets a measured preview, or is
    /// declared infeasible.
    pub fn add_curve(&mut self, id: EntityId, curve: &Curve) -> Result<(), Tier0Error> {
        if is_cheap_exact(curve) {
            return match curve.bounds() {
                Ok(_) => self.add_exact(id),
                Err(_) => self.declare(id, InfeasibleReason::EvaluatorRefused),
            };
        }
        match build_preview(id, curve, &self.params)? {
            Ok(preview) => self.add_preview(preview),
            Err(reason) => self.declare(id, reason),
        }
    }
    fn declare(&mut self, id: EntityId, reason: InfeasibleReason) -> Result<(), Tier0Error> {
        self.claim(id)?;
        self.infeasible.push(Infeasible { id, reason });
        Ok(())
    }
    /// Finishes the plan. An entity missing from Tier 0 is a writer error.
    pub fn finish(mut self) -> Result<Tier0Plan, Tier0Error> {
        for (w, word) in self.seen.iter().enumerate() {
            let missing = if w + 1 == self.seen.len() && self.entities % 64 != 0 {
                !word & ((1u64 << (self.entities % 64)) - 1)
            } else {
                !word
            };
            if missing != 0 {
                let index = (w * 64) as u32 + missing.trailing_zeros();
                return Err(Tier0Error::Gap(EntityId::new(index).map_err(|_| Tier0Error::Limit)?));
            }
        }
        self.exact.sort_unstable();
        self.previews.sort_by_key(|p| p.replaces());
        self.infeasible.sort_by_key(|i| i.id);
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for id in self.exact {
            match runs.last_mut() {
                Some((start, len)) if *start + *len == id => *len += 1,
                _ => runs.push((id, 1)),
            }
        }
        Ok(Tier0Plan {
            entities: self.entities,
            runs,
            previews: self.previews,
            infeasible: self.infeasible,
            partial: false,
        })
    }
}

// ------------------------------------------------------------ previews

fn point_at(curve: &Curve, t: f64) -> Result<Point2, InfeasibleReason> {
    curve.eval(t).map_err(|_| InfeasibleReason::EvaluatorRefused)
}

/// Whether a curve is cheap enough to go out exact in Tier 0: a full circle, a line segment or circular
/// arc, or a polyline of at most `EXACT_POLYLINE_VERTICES` vertices.
pub fn is_cheap_exact(curve: &Curve) -> bool {
    match curve {
        Curve::Circle(_) => true,
        Curve::Polyline(p) => p.vertices().len() <= EXACT_POLYLINE_VERTICES,
        Curve::Trimmed(t) => matches!(t.basis(), Curve::Line(_) | Curve::Circle(_)),
        _ => false,
    }
}

/// The control polygon, for the curve kinds that have one.
fn control_polygon(curve: &Curve) -> Option<Vec<Point2>> {
    match curve {
        Curve::Polyline(p) => Some(p.vertices().to_vec()),
        Curve::Bezier(b) => Some(b.control_points().to_vec()),
        Curve::BSpline(b) => Some(b.control_points().to_vec()),
        Curve::RationalBSpline(b) => Some(b.control_points().to_vec()),
        Curve::Trimmed(t) => {
            let trim = t.trim();
            let (placement, a, b) = match t.basis() {
                Curve::Line(_) => {
                    let d = curve.domain();
                    return Some(vec![curve.eval(d.lo()).ok()?, curve.eval(d.hi()).ok()?]);
                }
                Curve::Circle(c) => (*c.placement(), c.radius(), c.radius()),
                Curve::Ellipse(e) => (*e.placement(), e.semi_axis_1(), e.semi_axis_2()),
                _ => return None,
            };
            let (o, x, y) = (placement.origin(), placement.x_axis(), placement.y_axis());
            let at = |u: f64, v: f64| Point2::new(o.x + u * x.x + v * y.x, o.y + u * x.y + v * y.y);
            let n = ((trim.span / core::f64::consts::FRAC_PI_2).ceil() as usize).clamp(1, 64);
            let mut out = vec![at(a * trim.start.cos(), b * trim.start.sin())];
            for k in 0..n {
                let ta = trim.start + trim.span * k as f64 / n as f64;
                let tb = trim.start + trim.span * (k + 1) as f64 / n as f64;
                let (tm, h) = (0.5 * (ta + tb), 0.5 * (tb - ta));
                out.push(at(a * tm.cos() / h.cos(), b * tm.sin() / h.cos()));
                out.push(at(a * tb.cos(), b * tb.sin()));
            }
            if trim.reversed {
                out.reverse();
            }
            Some(out)
        }
        Curve::Composite(c) => {
            let mut out = Vec::new();
            for s in c.segments() {
                out.extend(control_polygon(s)?);
            }
            Some(out)
        }
        _ => None,
    }
}

/// How many pieces the curve is made of, which sets how finely it is sampled.
fn pieces(curve: &Curve) -> usize {
    match curve {
        Curve::BSpline(b) => b.control_points().len().saturating_sub(b.degree()).max(1),
        Curve::RationalBSpline(b) => b.control_points().len().saturating_sub(b.degree()).max(1),
        Curve::Composite(c) => c.segments().iter().map(pieces).sum::<usize>().max(1),
        Curve::Polyline(p) => p.vertices().len().saturating_sub(1).max(1),
        _ => 1,
    }
}

/// The parameters a curve is sampled at: `n + 1` uniform ones, and the joints of a polyline or a
/// composite, so a corner is never cut by the dense polyline the deviation is measured against.
fn sample_params(curve: &Curve, n: usize) -> Vec<f64> {
    let dom = curve.domain();
    let mut ts: Vec<f64> = (0..=n).map(|i| dom.lo() + dom.width() * i as f64 / n as f64).collect();
    let joints = match curve {
        Curve::Polyline(p) => p.vertices().len(),
        Curve::Composite(c) => c.segments().len() + 1,
        _ => 0,
    };
    ts.extend((0..joints).map(|k| dom.lo() + k as f64));
    ts.sort_by(f64::total_cmp);
    ts.dedup();
    ts
}

/// Douglas-Peucker: the vertices of `pts` that keep every dropped vertex within `eps` of the result.
fn simplify(pts: &[Point2], eps: f64) -> Vec<Point2> {
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = ([pts[a].x, pts[a].y], [pts[b].x, pts[b].y]);
        let mut far = (0.0f64, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = seg_dist([p.x, p.y], pa, pb);
            if d > far.0 {
                far = (d, i);
            }
        }
        if far.0 > eps {
            keep[far.1] = true;
            stack.push((a, far.1));
            stack.push((far.1, b));
        }
    }
    pts.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

fn dist_to(p: [f64; 2], poly: &[[f64; 2]]) -> f64 {
    if poly.len() == 1 {
        return (p[0] - poly[0][0]).hypot(p[1] - poly[0][1]);
    }
    poly.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// The two-sided maximum deviation between a preview and its exact curve, in world units: samples of the
/// curve against the preview, and samples of the preview against a dense polyline through the curve.
/// The curve is read through the exact evaluator only.
pub fn measure_deviation(curve: &Curve, preview: &Preview) -> Result<f64, Tier0Error> {
    let poly = preview.points();
    let n = (pieces(curve) * 128).clamp(MIN_SAMPLES, MAX_SAMPLES);
    let mut dense = Vec::with_capacity(n + 1);
    for t in sample_params(curve, n) {
        let p = curve.eval(t)?;
        dense.push([p.x, p.y]);
    }
    let mut worst = 0.0f64;
    for &p in &dense {
        worst = worst.max(dist_to(p, &poly));
    }
    if poly.len() == 1 {
        worst = worst.max(dist_to(poly[0], &dense));
    }
    for w in poly.windows(2) {
        for k in 0..=6 {
            let t = f64::from(k) / 6.0;
            let q = [w[0][0] + t * (w[1][0] - w[0][0]), w[0][1] + t * (w[1][1] - w[0][1])];
            worst = worst.max(dist_to(q, &dense));
        }
    }
    Ok(worst)
}

fn quantise(id: EntityId, form: PreviewForm, pts: &[Point2], bound: f32) -> Result<Preview, Tier0Error> {
    let origin = [
        pts.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
        pts.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
    ];
    let vertices = pts.iter().map(|p| [(p.x - origin[0]) as f32, (p.y - origin[1]) as f32]).collect();
    Preview::new(id, form, bound, origin, vertices)
}

/// The stored bound for a measured deviation: the margin and a sliver for the floating point, rounded up
/// to an `f32`. `extent` is the largest offset of the preview from its origin.
fn stored_bound(measured: f64, extent: f64) -> Option<f32> {
    let want = measured * BOUND_MARGIN + ROUNDING_SLACK * extent.max(1.0);
    let mut f = want as f32;
    if f64::from(f) < want {
        f = f32::from_bits(f.to_bits() + 1);
    }
    (f.is_finite() && f > 0.0).then_some(f)
}

/// Tries one candidate shape: the preview, if its measured and margined deviation is within the budget.
fn try_shape(
    id: EntityId,
    curve: &Curve,
    form: PreviewForm,
    pts: &[Point2],
    params: &Tier0Params,
) -> Result<Option<Preview>, Tier0Error> {
    if pts.is_empty() || pts.len() > MAX_PREVIEW_VERTICES {
        return Ok(None);
    }
    // Measure with a placeholder bound, then store the real one.
    let probe = quantise(id, form, pts, 1.0)?;
    let measured = measure_deviation(curve, &probe)?;
    let extent = probe.offsets().iter().flatten().fold(0.0f64, |m, v| m.max(f64::from(v.abs())));
    let Some(bound) = stored_bound(measured, extent) else { return Ok(None) };
    if f64::from(bound) > params.max_error {
        return Ok(None);
    }
    Ok(Some(Preview { error_bound: bound, ..probe }))
}

fn chord_polyline(curve: &Curve, tol: f64) -> Result<Vec<Point2>, InfeasibleReason> {
    let dom = curve.domain();
    let (lo, hi) = (dom.lo(), dom.hi());
    let init = match curve {
        Curve::BSpline(_) | Curve::RationalBSpline(_) | Curve::Composite(_) => 4 * pieces(curve),
        _ => 8,
    };
    let mut out = vec![point_at(curve, lo)?];
    for i in 0..init {
        let t0 = lo + (hi - lo) * i as f64 / init as f64;
        let t1 = lo + (hi - lo) * (i + 1) as f64 / init as f64;
        split(curve, t0, t1, tol, 0, &mut out)?;
    }
    Ok(out)
}

fn split(
    curve: &Curve,
    t0: f64,
    t1: f64,
    tol: f64,
    depth: u32,
    out: &mut Vec<Point2>,
) -> Result<(), InfeasibleReason> {
    let p0 = out[out.len() - 1];
    let p1 = point_at(curve, t1)?;
    let mut bad = false;
    for f in [0.25, 0.5, 0.75] {
        let q = point_at(curve, t0 + (t1 - t0) * f)?;
        if seg_dist([q.x, q.y], [p0.x, p0.y], [p1.x, p1.y]) > 0.9 * tol {
            bad = true;
        }
    }
    if bad && depth < MAX_DEPTH {
        let m = 0.5 * (t0 + t1);
        split(curve, t0, m, tol, depth + 1, out)?;
        split(curve, m, t1, tol, depth + 1, out)
    } else {
        if out.len() >= MAX_PREVIEW_VERTICES {
            return Err(InfeasibleReason::VertexCap);
        }
        out.push(p1);
        Ok(())
    }
}

/// Builds the preview of a deferred curve, measured against the exact curve, or says why it cannot.
///
/// The form follows the recorded decision (ADR 0007): an ellipse arc gets a chord polyline; anything
/// with a control polygon gets that where its measured deviation is within the budget, and a chord
/// polyline otherwise. The stored bound is the measured deviation with a margin, rounded up, and it is
/// never above the budget.
pub fn build_preview(
    id: EntityId,
    curve: &Curve,
    params: &Tier0Params,
) -> Result<Result<Preview, InfeasibleReason>, Tier0Error> {
    params.check()?;
    if curve.bounds().is_err() {
        return Ok(Err(InfeasibleReason::EvaluatorRefused));
    }
    let is_ellipse_arc = matches!(curve, Curve::Trimmed(t) if matches!(t.basis(), Curve::Ellipse(_)));
    if let Curve::Polyline(line) = curve {
        // Its control polygon is itself, which saves nothing: drop the vertices that are within the
        // budget of the rest.
        for share in CHORD_TARGETS {
            let pts = simplify(line.vertices(), params.max_error * share);
            match try_shape(id, curve, PreviewForm::ChordPolyline, &pts, params) {
                Ok(Some(p)) => return Ok(Ok(p)),
                Ok(None) => {}
                Err(Tier0Error::Geometry(_)) => return Ok(Err(InfeasibleReason::EvaluatorRefused)),
                Err(e) => return Err(e),
            }
        }
        let capped = line.vertices().len() > MAX_PREVIEW_VERTICES;
        return Ok(Err(if capped { InfeasibleReason::VertexCap } else { InfeasibleReason::BoundNotMet }));
    }
    if !is_ellipse_arc && let Some(poly) = control_polygon(curve) {
        match try_shape(id, curve, PreviewForm::ControlPolygon, &poly, params) {
            Ok(Some(p)) => return Ok(Ok(p)),
            Ok(None) => {}
            Err(Tier0Error::Geometry(_)) => return Ok(Err(InfeasibleReason::EvaluatorRefused)),
            Err(e) => return Err(e),
        }
    }
    for share in CHORD_TARGETS {
        let pts = match chord_polyline(curve, params.max_error * share) {
            Ok(pts) => pts,
            Err(reason) => return Ok(Err(reason)),
        };
        match try_shape(id, curve, PreviewForm::ChordPolyline, &pts, params) {
            Ok(Some(p)) => return Ok(Ok(p)),
            Ok(None) => {}
            Err(Tier0Error::Geometry(_)) => return Ok(Err(InfeasibleReason::EvaluatorRefused)),
            Err(e) => return Err(e),
        }
    }
    Ok(Err(InfeasibleReason::BoundNotMet))
}

// --------------------------------------------------------------- plan

impl Tier0Plan {
    /// A plan from curves, one per entity.
    pub fn build<I>(entities: u32, curves: I, params: Tier0Params) -> Result<Tier0Plan, Tier0Error>
    where
        I: IntoIterator<Item = (EntityId, Curve)>,
    {
        let mut b = Tier0Builder::new(entities, params)?;
        for (id, curve) in curves {
            b.add_curve(id, &curve)?;
        }
        b.finish()
    }
    /// The number of entities in the artifact.
    pub fn artifact_entities(&self) -> usize {
        self.entities as usize
    }
    /// The number of entities reachable from Tier 0.
    pub fn tier0_entities(&self) -> usize {
        self.exact_count() + self.previews.len()
    }
    /// Checks that the entities reachable from Tier 0 (and the declared infeasible ones) are exactly
    /// the given set.
    pub fn check_coverage(&self, artifact: &[EntityId]) -> Result<(), Tier0Error> {
        let n = check_dense(artifact).map_err(|e| match e {
            EntityError::Gap(id) => Tier0Error::Gap(id),
            EntityError::Duplicate(id) => Tier0Error::Duplicate(id),
            _ => Tier0Error::Limit,
        })?;
        match n.cmp(&(self.entities as usize)) {
            core::cmp::Ordering::Equal => Ok(()),
            core::cmp::Ordering::Greater => {
                Err(Tier0Error::Gap(EntityId::new(self.entities).map_err(|_| Tier0Error::Limit)?))
            }
            core::cmp::Ordering::Less => {
                Err(Tier0Error::OutOfRange(EntityId::new(n as u32).map_err(|_| Tier0Error::Limit)?))
            }
        }
    }
    /// The entities this plan has an entry for or declares infeasible: the artifact's for a whole plan, the
    /// leaf's for the plan of one leaf.
    pub fn covered(&self) -> usize {
        self.exact_count() + self.previews.len() + self.infeasible.len()
    }
    /// Checks that the plan covers exactly `ids`, in any order: an id with no entry is `Gap`, an id twice
    /// `Duplicate`, and an entry for an id that is not listed `OutOfRange`.
    pub fn check_covers(&self, ids: &[EntityId]) -> Result<(), Tier0Error> {
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        if let Some(w) = sorted.windows(2).find(|w| w[0] == w[1]) {
            return Err(Tier0Error::Duplicate(w[0]));
        }
        let held = |id: EntityId| {
            self.entry(id).is_some() || self.infeasible.binary_search_by_key(&id, |i| i.id).is_ok()
        };
        if let Some(id) = sorted.iter().copied().find(|id| !held(*id)) {
            return Err(Tier0Error::Gap(id));
        }
        if self.covered() != sorted.len() {
            let listed = |id: &EntityId| sorted.binary_search(id).is_ok();
            let stray = self
                .exact_ids()
                .chain(self.previews.iter().map(Preview::replaces))
                .chain(self.infeasible.iter().map(|i| i.id))
                .find(|id| !listed(id))
                .ok_or(Tier0Error::Limit)?;
            return Err(Tier0Error::OutOfRange(stray));
        }
        Ok(())
    }
    /// The plan of the entities `ids` alone, as one leaf's stream: each entity keeps the entry it has here,
    /// exact, preview or infeasible. An id this plan has no entry for is `Gap`.
    pub fn cut(&self, ids: &[EntityId]) -> Result<Tier0Plan, Tier0Error> {
        let mut sorted = ids.to_vec();
        sorted.sort_unstable();
        if let Some(w) = sorted.windows(2).find(|w| w[0] == w[1]) {
            return Err(Tier0Error::Duplicate(w[0]));
        }
        let mut cut = Tier0Plan {
            entities: self.entities,
            runs: Vec::new(),
            previews: Vec::new(),
            infeasible: Vec::new(),
            partial: true,
        };
        for id in sorted {
            match self.entry(id) {
                Some(Tier0Entry::Exact { .. }) => match cut.runs.last_mut() {
                    Some((start, len)) if *start + *len == id.index() => *len += 1,
                    _ => cut.runs.push((id.index(), 1)),
                },
                Some(Tier0Entry::Preview(p)) => cut.previews.push(p.clone()),
                None => match self.infeasible.binary_search_by_key(&id, |i| i.id) {
                    Ok(k) => cut.infeasible.push(self.infeasible[k]),
                    Err(_) => return Err(Tier0Error::Gap(id)),
                },
            }
        }
        Ok(cut)
    }
    /// How many entities go out exact.
    pub fn exact_count(&self) -> usize {
        self.runs.iter().map(|&(_, len)| len as usize).sum()
    }
    /// The ids that go out exact, ascending.
    pub fn exact_ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.runs.iter().flat_map(|&(start, len)| {
            (start..start + len).map(|i| EntityId::new(i).expect("below the limit"))
        })
    }
    /// The previews, ascending by the entity they replace.
    pub fn previews(&self) -> &[Preview] {
        &self.previews
    }
    /// The entities declared infeasible, with their reasons.
    pub fn infeasible(&self) -> &[Infeasible] {
        &self.infeasible
    }
    /// The entry for an entity.
    pub fn entry(&self, id: EntityId) -> Option<Tier0Entry<'_>> {
        let i = id.index();
        let at = self.runs.partition_point(|&(start, _)| start <= i);
        if at > 0 {
            let (start, len) = self.runs[at - 1];
            if i < start + len {
                return Some(Tier0Entry::Exact { id });
            }
        }
        self.previews
            .binary_search_by_key(&id, |p| p.replaces())
            .ok()
            .map(|k| Tier0Entry::Preview(&self.previews[k]))
    }
    /// What conformance output says about this plan.
    pub fn conformance(&self) -> Conformance {
        Conformance {
            exact: self.exact_count(),
            display_fidelity: self.previews.len(),
            infeasible: self.infeasible.len(),
        }
    }
    /// The Tier 0 stream: header, exact runs, previews, infeasible records. See `docs/STREAMING.md`
    /// section 4.4.
    pub fn encode(&self) -> Vec<u8> {
        let size = HEADER_LEN
            + self.runs.len() * RUN_LEN
            + self.previews.iter().map(|p| PREVIEW_FIXED + VERTEX_LEN * p.vertices.len()).sum::<usize>()
            + self.infeasible.len() * INFEASIBLE_LEN;
        let mut out = Vec::with_capacity(size);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&[if self.partial { LEAF_VERSION } else { VERSION }, 0, 0, 0]);
        for n in
            [self.entities, self.runs.len() as u32, self.previews.len() as u32, self.infeasible.len() as u32]
        {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for &(start, len) in &self.runs {
            out.extend_from_slice(&start.to_le_bytes());
            out.extend_from_slice(&len.to_le_bytes());
        }
        for p in &self.previews {
            out.extend_from_slice(&p.replaces.index().to_le_bytes());
            out.push(p.form.code());
            out.extend_from_slice(&(p.vertices.len() as u16).to_le_bytes());
            out.extend_from_slice(&p.error_bound.to_le_bytes());
            for c in p.origin {
                out.extend_from_slice(&c.to_le_bytes());
            }
            for v in &p.vertices {
                out.extend_from_slice(&v[0].to_le_bytes());
                out.extend_from_slice(&v[1].to_le_bytes());
            }
        }
        for i in &self.infeasible {
            out.extend_from_slice(&i.id.index().to_le_bytes());
            out.push(i.reason.code());
        }
        out
    }
    /// Reads a Tier 0 stream. Every count is checked against the bytes that remain before anything is
    /// allocated, and a stream that does not cover its entities exactly once is refused.
    pub fn decode(bytes: &[u8]) -> Result<Tier0Plan, Tier0Error> {
        Tier0Plan::parse(bytes, false)
    }
    /// Reads the stream of one leaf (version 2, `docs/STREAMING.md` section 4.5): the same records, for the
    /// entities of the leaf only. Every count is checked against the bytes first, ids must lie in the
    /// artifact and be listed once, and nothing is said about which entities belong to the leaf: that is
    /// [`Tier0Plan::check_covers`], against the ids of the leaf's hot blocks. A whole-artifact stream is
    /// refused here and a leaf stream by [`Tier0Plan::decode`].
    pub fn decode_leaf(bytes: &[u8]) -> Result<Tier0Plan, Tier0Error> {
        Tier0Plan::parse(bytes, true)
    }
    fn parse(bytes: &[u8], leaf: bool) -> Result<Tier0Plan, Tier0Error> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err(fmt_err("bad magic"));
        }
        if r.take(1)?[0] != if leaf { LEAF_VERSION } else { VERSION } {
            return Err(fmt_err("unknown version"));
        }
        if r.take(3)? != [0, 0, 0] {
            return Err(fmt_err("reserved bytes are not zero"));
        }
        let entities = r.u32()?;
        let (n_runs, n_previews, n_infeasible) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        if entities as usize > MAX_ENTITIES {
            return Err(Tier0Error::Limit);
        }
        let need = n_runs
            .checked_mul(RUN_LEN)
            .zip(n_previews.checked_mul(PREVIEW_FIXED + VERTEX_LEN))
            .zip(n_infeasible.checked_mul(INFEASIBLE_LEN))
            .and_then(|((a, b), c)| a.checked_add(b)?.checked_add(c));
        if need.is_none_or(|n| n > r.remaining()) {
            return Err(fmt_err("the counts do not fit the stream"));
        }
        let mut spans: Vec<(u64, u64)> = Vec::with_capacity(n_runs + n_previews + n_infeasible);
        let mut runs = Vec::with_capacity(n_runs);
        let mut next_free = 0u64;
        for _ in 0..n_runs {
            let (start, len) = (r.u32()?, r.u32()?);
            if len == 0 || u64::from(start) < next_free {
                return Err(fmt_err("exact runs are empty, unordered or overlapping"));
            }
            next_free = u64::from(start) + u64::from(len);
            if next_free > u64::from(entities) {
                return Err(Tier0Error::OutOfRange(EntityId::new(start).map_err(|_| Tier0Error::Limit)?));
            }
            spans.push((u64::from(start), next_free));
            runs.push((start, len));
        }
        let mut previews: Vec<Preview> = Vec::with_capacity(n_previews);
        for _ in 0..n_previews {
            let id = EntityId::new(r.u32()?).map_err(|_| Tier0Error::Limit)?;
            let form = PreviewForm::from_code(r.take(1)?[0]).ok_or(fmt_err("unknown preview form"))?;
            let count = usize::from(u16::from_le_bytes([r.take(1)?[0], r.take(1)?[0]]));
            let bound = f32::from_le_bytes(r.array()?);
            let origin = [f64::from_le_bytes(r.array()?), f64::from_le_bytes(r.array()?)];
            if count == 0 || count > MAX_PREVIEW_VERTICES {
                return Err(Tier0Error::Limit);
            }
            if count * VERTEX_LEN > r.remaining() {
                return Err(fmt_err("a preview runs past the end"));
            }
            let mut vertices = Vec::with_capacity(count);
            for _ in 0..count {
                vertices.push([f32::from_le_bytes(r.array()?), f32::from_le_bytes(r.array()?)]);
            }
            if id.index() >= entities {
                return Err(Tier0Error::OutOfRange(id));
            }
            if previews.last().is_some_and(|p| p.replaces >= id) {
                return Err(fmt_err("previews are not strictly ascending"));
            }
            spans.push((u64::from(id.index()), u64::from(id.index()) + 1));
            previews.push(Preview::new(id, form, bound, origin, vertices)?);
        }
        let mut infeasible: Vec<Infeasible> = Vec::with_capacity(n_infeasible);
        for _ in 0..n_infeasible {
            let id = EntityId::new(r.u32()?).map_err(|_| Tier0Error::Limit)?;
            let reason =
                InfeasibleReason::from_code(r.take(1)?[0]).ok_or(fmt_err("unknown infeasible reason"))?;
            if id.index() >= entities {
                return Err(Tier0Error::OutOfRange(id));
            }
            if infeasible.last().is_some_and(|i| i.id >= id) {
                return Err(fmt_err("infeasible records are not strictly ascending"));
            }
            spans.push((u64::from(id.index()), u64::from(id.index()) + 1));
            infeasible.push(Infeasible { id, reason });
        }
        if r.remaining() != 0 {
            return Err(fmt_err("trailing bytes"));
        }
        spans.sort_unstable();
        if leaf {
            if spans.is_empty() {
                return Err(fmt_err("a leaf stream covers nothing"));
            }
            if let Some(w) = spans.windows(2).find(|w| w[1].0 < w[0].1) {
                return Err(Tier0Error::Duplicate(
                    EntityId::new(w[1].0 as u32).map_err(|_| Tier0Error::Limit)?,
                ));
            }
            return Ok(Tier0Plan { entities, runs, previews, infeasible, partial: true });
        }
        let mut expect = 0u64;
        for (start, end) in spans {
            match start.cmp(&expect) {
                core::cmp::Ordering::Less => {
                    return Err(Tier0Error::Duplicate(
                        EntityId::new(start as u32).map_err(|_| Tier0Error::Limit)?,
                    ));
                }
                core::cmp::Ordering::Greater => {
                    return Err(Tier0Error::Gap(
                        EntityId::new(expect as u32).map_err(|_| Tier0Error::Limit)?,
                    ));
                }
                core::cmp::Ordering::Equal => expect = end,
            }
        }
        if expect != u64::from(entities) {
            return Err(Tier0Error::Gap(EntityId::new(expect as u32).map_err(|_| Tier0Error::Limit)?));
        }
        Ok(Tier0Plan { entities, runs, previews, infeasible, partial: leaf })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.bytes.len() - self.at
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Tier0Error> {
        if n > self.remaining() {
            return Err(fmt_err("the stream ends early"));
        }
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        Ok(s)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], Tier0Error> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }
    fn u32(&mut self) -> Result<u32, Tier0Error> {
        Ok(u32::from_le_bytes(self.array()?))
    }
}

// -------------------------------------------------------------- scene

/// How a refinement ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refined {
    /// The preview was replaced by the exact geometry.
    Replaced,
    /// Nothing changed: the entity was already exact.
    Unchanged,
}

/// What stands for one entity that is not simply exact.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Preview(Preview),
    Refined(Curve),
    Absent,
}

/// The drawable state of an artifact: exactly one thing per entity. Only the entities that are not
/// plain exact ones are stored, so the scene of a huge artifact is as small as its previews.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    entities: u32,
    slots: BTreeMap<u32, Slot>,
    generation: Generation,
    dropped: DropCounts,
}

impl Scene {
    /// The scene a reader has after Tier 0.
    pub fn from_plan(plan: &Tier0Plan) -> Scene {
        let mut slots = BTreeMap::new();
        for p in &plan.previews {
            slots.insert(p.replaces.index(), Slot::Preview(p.clone()));
        }
        for i in &plan.infeasible {
            slots.insert(i.id.index(), Slot::Absent);
        }
        Scene {
            entities: plan.entities,
            slots,
            generation: Generation::INITIAL,
            dropped: DropCounts::default(),
        }
    }
    /// The number of things drawn: one per entity that has anything, before and after any refinement.
    pub fn drawn(&self) -> usize {
        self.entities as usize - self.slots.values().filter(|s| matches!(s, Slot::Absent)).count()
    }
    /// The fidelity of what stands for the entity.
    pub fn fidelity(&self, id: EntityId) -> Option<Fidelity> {
        match self.slots.get(&id.index()) {
            Some(Slot::Preview(_)) => Some(Fidelity::Display),
            Some(Slot::Refined(_)) => Some(Fidelity::Exact),
            Some(Slot::Absent) => None,
            None => (id.index() < self.entities).then_some(Fidelity::Exact),
        }
    }
    /// The generation the scene holds.
    pub fn generation(&self) -> Generation {
        self.generation
    }
    /// A viewport change: results of older generations are refused from now on. Never goes back.
    pub fn advance(&mut self, generation: Generation) {
        self.generation = self.generation.max(generation);
    }
    /// How refined what stands for the entity is. `None` for an entity with nothing to draw.
    pub fn refinement(&self, id: EntityId) -> Option<Refinement> {
        match self.slots.get(&id.index()) {
            Some(Slot::Preview(_)) => Some(Refinement::TIER0),
            Some(Slot::Refined(_)) => Some(Refinement::TIER2),
            Some(Slot::Absent) => None,
            None => (id.index() < self.entities).then_some(Refinement::TIER2),
        }
    }
    /// What the barrier has dropped.
    pub fn dropped(&self) -> DropCounts {
        self.dropped
    }
    /// Whether two scenes draw the same thing, whatever generation and counters they hold.
    pub fn same_drawing(&self, other: &Scene) -> bool {
        self.entities == other.entities && self.slots == other.slots
    }
    /// Applies a tagged result: the only way a read, decode or refinement result reaches the scene.
    pub fn apply(&mut self, result: RefinementResult) -> Result<Applied, Tier0Error> {
        let entity = result.entity();
        if entity.index() >= self.entities {
            return Err(Tier0Error::Unknown(entity));
        }
        // The barrier. Nothing below this point runs for a result of a superseded view, or for one that
        // would put something less refined over something more refined.
        if result.generation() < self.generation {
            self.dropped.stale_generation += 1;
            return Ok(Applied::Dropped(DropReason::StaleGeneration));
        }
        if self.refinement(entity).is_some_and(|held| result.refinement() < held) {
            self.dropped.stale_refinement += 1;
            return Ok(Applied::Dropped(DropReason::StaleRefinement));
        }
        let applied = match result.update() {
            Update::Preview(p) => {
                let same = matches!(self.slots.get(&entity.index()), Some(Slot::Preview(held)) if held == p);
                self.slots.insert(entity.index(), Slot::Preview(p.clone()));
                if same { Applied::Unchanged } else { Applied::Replaced }
            }
            Update::Exact { entity, curve } => match self.refine(*entity, curve)? {
                Refined::Replaced => Applied::Replaced,
                Refined::Unchanged => Applied::Unchanged,
            },
        };
        self.generation = result.generation();
        Ok(applied)
    }
    /// Replaces the preview of `id` by its exact geometry. The curve must agree with the preview
    /// (within its stored bound). Applying the same refinement again changes nothing.
    pub fn refine(&mut self, id: EntityId, exact: &Curve) -> Result<Refined, Tier0Error> {
        if id.index() >= self.entities {
            return Err(Tier0Error::Unknown(id));
        }
        match self.slots.get(&id.index()) {
            None => Ok(Refined::Unchanged),
            Some(Slot::Refined(c)) if c == exact => Ok(Refined::Unchanged),
            Some(Slot::Refined(_)) => Err(Tier0Error::Conflict(id)),
            Some(Slot::Preview(p)) => {
                if measure_deviation(exact, p)? > p.error_bound() {
                    return Err(Tier0Error::Mismatch(id));
                }
                self.slots.insert(id.index(), Slot::Refined(exact.clone()));
                Ok(Refined::Replaced)
            }
            Some(Slot::Absent) => {
                self.slots.insert(id.index(), Slot::Refined(exact.clone()));
                Ok(Refined::Replaced)
            }
        }
    }
}
