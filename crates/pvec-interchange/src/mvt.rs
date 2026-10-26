//! The display-only Mapbox Vector Tile export (issue P6.6).
//!
//! A vector tile cannot carry a curve: everything in it is quantized integer geometry in tile space.
//! This module is therefore an explicitly display-only derivative. Every emitted entity is recorded
//! as `BoundedApproximation` with its achieved world-space deviation, or as `Unsupported` with a
//! reason, and the ledger here has no way to record anything stronger ([`MvtLedger::record`]).
//!
//! The pipeline is five stages, each a public function so each can be tested alone:
//!
//! 1. [`Projection::tessellation_tolerance`]: the screen-space budget at the tile's zoom, converted
//!    through the source-to-map-to-screen transform into a world-space chord tolerance.
//! 2. [`tessellate`]: adaptive subdivision driven by the exact evaluator of `pvec-geometry`.
//! 3. [`Projection::to_tile`]: world points into tile units (floating point, `0..extent` inside).
//! 4. [`clip_polyline`]: Liang-Barsky clipping against the tile plus its buffer.
//! 5. [`quantize`], then [`encode_geometry`]: integer tile coordinates and MVT geometry commands.
//!
//! Tessellation depends on the zoom level and the entity only, never on the tile coordinates, so two
//! neighbouring tiles clip the very same polyline and their union has no seam.

use crate::fidelity::{
    ErrorBound, Fidelity, FidelityKind, FidelityLedger, FidelityRecord, Reason, Representation,
    TargetConstruct,
};
use crate::report::{Report, ReportError};
use core::fmt;
use pvec_capabilities::{CapabilityId, ids};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, Point2, Transform2};
use pvec_numeric::Policy;
use std::collections::BTreeMap;

/// The tile extent the export writes by default (the value most MVT consumers assume).
pub const DEFAULT_EXTENT: u32 = 4096;
/// The clip buffer around a tile, in extent units, by default.
pub const DEFAULT_BUFFER: u32 = 64;
/// The default screen-space chord budget, in pixels of a 512 pixel tile.
pub const DEFAULT_SCREEN_TOLERANCE_PX: f64 = 0.25;
/// The highest zoom the test-suite measures the export at. `docs/INTERCHANGE.md` chapter 5 states
/// the quantization error at this zoom.
pub const MAX_TESTED_ZOOM: u8 = 14;
/// The highest zoom [`TileId`] accepts.
pub const MAX_ZOOM: u8 = 24;
/// Interior points sampled per final chord when the deviation is measured.
pub const DEVIATION_SAMPLES: usize = 7;

const INITIAL_SPLITS: usize = 8;
const MAX_DEPTH: u32 = 24;
const TARGET: &str = "mvt.LineString";

/// What went wrong setting up an export. A single entity that cannot be exported is not an error: it
/// is an `Unsupported` record in the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MvtError {
    /// A tile address outside `0 <= x, y < 2^z` or a zoom above [`MAX_ZOOM`].
    BadTile,
    /// An option out of range, with the name of the option.
    BadOption(&'static str),
    /// The source-to-map transform is not a similarity (rotation, mirror, uniform scale, shift).
    /// A shear or a non-uniform scale would make "the screen budget in world units" depend on the
    /// direction, so it is refused rather than approximated.
    NotSimilarity,
    /// The quantization error alone is at least the whole screen budget, so no chord tolerance is left.
    BudgetBelowQuantization,
    /// A layer name that is empty.
    EmptyLayerName,
    /// An attempt to record something other than a bounded approximation or `Unsupported`.
    NotDisplayOnly(FidelityKind),
    /// A record the fidelity ledger refused.
    Record(String),
}

impl fmt::Display for MvtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MvtError::BadTile => write!(f, "tile address out of range"),
            MvtError::BadOption(n) => write!(f, "option out of range: {n}"),
            MvtError::NotSimilarity => write!(f, "the source-to-map transform must be a similarity"),
            MvtError::BudgetBelowQuantization => {
                write!(f, "the quantization error alone exceeds the screen tolerance")
            }
            MvtError::EmptyLayerName => write!(f, "a layer name must not be empty"),
            MvtError::NotDisplayOnly(k) => {
                write!(f, "an MVT entity cannot be recorded as {}; tiles are display-only", k.name())
            }
            MvtError::Record(s) => write!(f, "record refused: {s}"),
        }
    }
}

impl std::error::Error for MvtError {}

/// A tile address in the XYZ scheme: `y` counts down from the top, as in tile space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TileId {
    z: u8,
    x: u32,
    y: u32,
}

impl TileId {
    /// A tile, or [`MvtError::BadTile`].
    pub fn new(z: u8, x: u32, y: u32) -> Result<TileId, MvtError> {
        if z > MAX_ZOOM || (x as u64) >= (1u64 << z) || (y as u64) >= (1u64 << z) {
            return Err(MvtError::BadTile);
        }
        Ok(TileId { z, x, y })
    }
    /// Zoom.
    pub fn z(self) -> u8 {
        self.z
    }
    /// Column.
    pub fn x(self) -> u32 {
        self.x
    }
    /// Row.
    pub fn y(self) -> u32 {
        self.y
    }
}

/// The complete source-to-map-to-screen transform.
///
/// The source (drawing) plane maps into a square map plane of side `map_size` with `source_to_map`;
/// the map plane is cut into `2^z x 2^z` tiles at zoom `z`, y down; a tile is shown `tile_px` pixels
/// wide. So one pixel at zoom `z` is `map_size / (2^z * tile_px)` map units, and `1 / scale` world
/// units per map unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projection {
    source_to_map: Transform2,
    scale: f64,
    map_size: f64,
    tile_px: f64,
}

impl Projection {
    /// A projection. The transform must be a similarity (see [`MvtError::NotSimilarity`]).
    pub fn new(source_to_map: Transform2, map_size: f64, tile_px: f64) -> Result<Projection, MvtError> {
        if !(map_size.is_finite() && map_size > 0.0) {
            return Err(MvtError::BadOption("map_size"));
        }
        if !(tile_px.is_finite() && tile_px > 0.0) {
            return Err(MvtError::BadOption("tile_px"));
        }
        let [a, b, c, d, _, _] = source_to_map.matrix();
        let col0 = a * a + c * c;
        let col1 = b * b + d * d;
        let dot = a * b + c * d;
        let rel = Policy::V1.rel * 10.0 * col0.max(col1);
        if col0 <= 0.0 || (col0 - col1).abs() > rel || dot.abs() > rel {
            return Err(MvtError::NotSimilarity);
        }
        Ok(Projection { source_to_map, scale: col0.sqrt(), map_size, tile_px })
    }

    /// Map units per world unit.
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Map units in one screen pixel at zoom `z`.
    pub fn map_units_per_pixel(&self, z: u8) -> f64 {
        self.map_size / ((1u64 << z) as f64 * self.tile_px)
    }

    /// World units in one tile unit at zoom `z` for the given extent.
    pub fn world_per_tile_unit(&self, z: u8, extent: u32) -> f64 {
        self.map_size / ((1u64 << z) as f64 * extent as f64) / self.scale
    }

    /// The largest error, in world units, that rounding a point to a whole tile unit can cause: half
    /// the diagonal of one tile unit.
    pub fn quantization_error(&self, z: u8, extent: u32) -> f64 {
        self.world_per_tile_unit(z, extent) * core::f64::consts::FRAC_1_SQRT_2
    }

    /// The same, in screen pixels (it does not depend on the zoom).
    pub fn quantization_error_px(&self, extent: u32) -> f64 {
        self.tile_px / extent as f64 * core::f64::consts::FRAC_1_SQRT_2
    }

    /// The whole screen budget in world units: `px` pixels at zoom `z`.
    pub fn screen_to_world(&self, z: u8, px: f64) -> f64 {
        px * self.map_units_per_pixel(z) / self.scale
    }

    /// Stage 1: the chord tolerance, in world units, that leaves room for the quantization error
    /// inside the screen budget. Tessellating to this and then quantizing keeps the total world
    /// deviation at or below `screen_to_world(z, screen_px)`.
    pub fn tessellation_tolerance(&self, z: u8, opts: &MvtOptions) -> Result<f64, MvtError> {
        opts.validate()?;
        let budget = self.screen_to_world(z, opts.screen_tolerance_px);
        let left = budget - self.quantization_error(z, opts.extent);
        if left <= 0.0 || !left.is_finite() {
            return Err(MvtError::BudgetBelowQuantization);
        }
        Ok(left)
    }

    /// Stage 3: a world point in tile units of `tile` (`0..extent` inside the tile, y down).
    pub fn to_tile(&self, tile: TileId, extent: u32, p: Point2) -> (f64, f64) {
        let q = self.source_to_map.apply_point(p);
        let size = self.map_size / (1u64 << tile.z) as f64;
        let e = extent as f64;
        ((q.x / size - tile.x as f64) * e, (q.y / size - tile.y as f64) * e)
    }

    /// The inverse of [`Projection::to_tile`] followed by [`quantize`]'s rounding, for measuring:
    /// an integer tile coordinate back to the world plane.
    pub fn from_tile(&self, tile: TileId, extent: u32, t: (f64, f64)) -> Option<Point2> {
        let size = self.map_size / (1u64 << tile.z) as f64;
        let e = extent as f64;
        let map = Point2::new((t.0 / e + tile.x as f64) * size, (t.1 / e + tile.y as f64) * size);
        let inv = self.source_to_map.inverse().ok()?;
        Some(inv.apply_point(map))
    }
}

/// Export options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MvtOptions {
    /// Tile extent: the integer grid is `extent x extent`.
    pub extent: u32,
    /// Clip buffer around the tile, in extent units.
    pub buffer: u32,
    /// The chord budget in screen pixels.
    pub screen_tolerance_px: f64,
}

impl Default for MvtOptions {
    fn default() -> MvtOptions {
        MvtOptions {
            extent: DEFAULT_EXTENT,
            buffer: DEFAULT_BUFFER,
            screen_tolerance_px: DEFAULT_SCREEN_TOLERANCE_PX,
        }
    }
}

impl MvtOptions {
    fn validate(&self) -> Result<(), MvtError> {
        if !(16..=65_536).contains(&self.extent) {
            return Err(MvtError::BadOption("extent"));
        }
        if self.buffer > self.extent {
            return Err(MvtError::BadOption("buffer"));
        }
        if !(self.screen_tolerance_px.is_finite() && self.screen_tolerance_px > 0.0) {
            return Err(MvtError::BadOption("screen_tolerance_px"));
        }
        Ok(())
    }
}

/// Why a curve could not be tessellated.
#[derive(Clone, Debug, PartialEq)]
pub enum TessError {
    /// The tolerance is not finite and positive.
    BadTolerance,
    /// An unbounded line has no finite extent to draw.
    Unbounded,
    /// The depth limit was reached with the chord still further than the tolerance.
    ToleranceNotMet {
        /// The deviation actually measured.
        achieved: f64,
    },
    /// The exact evaluator refused, with its rule.
    Geometry(String),
}

impl fmt::Display for TessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TessError::BadTolerance => write!(f, "the chord tolerance must be finite and positive"),
            TessError::Unbounded => write!(f, "an unbounded line has no finite extent to tessellate"),
            TessError::ToleranceNotMet { achieved } => {
                write!(f, "the chord tolerance was not met at the depth limit (achieved {achieved:e})")
            }
            TessError::Geometry(s) => write!(f, "exact evaluation refused: {s}"),
        }
    }
}

/// A tessellated curve in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Tessellation {
    /// The polyline, start to end. For a closed curve the last point repeats the first.
    pub points: Vec<Point2>,
    /// The largest distance measured between the exact curve and the polyline, in world units.
    /// Measured at [`DEVIATION_SAMPLES`] interior points per final chord against the exact
    /// evaluator; zero for a curve that is already a polyline or a straight segment.
    pub deviation: f64,
}

fn geom<E: fmt::Display>(e: E) -> TessError {
    TessError::Geometry(e.to_string())
}

fn point_segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point2::new(a.x + t * dx, a.y + t * dy))
}

fn chord_deviation(c: &Curve, t0: f64, p0: Point2, t1: f64, p1: Point2) -> Result<f64, TessError> {
    let mut worst = 0.0f64;
    for k in 1..=DEVIATION_SAMPLES {
        let t = t0 + (t1 - t0) * (k as f64 / (DEVIATION_SAMPLES + 1) as f64);
        let p = c.eval(t).map_err(geom)?;
        worst = worst.max(point_segment_distance(p, p0, p1));
    }
    Ok(worst)
}

struct Refine<'a> {
    curve: &'a Curve,
    tol: f64,
    out: Vec<Point2>,
    worst: f64,
    failed: Option<f64>,
}

impl Refine<'_> {
    fn go(&mut self, t0: f64, p0: Point2, t1: f64, p1: Point2, depth: u32) -> Result<(), TessError> {
        let dev = chord_deviation(self.curve, t0, p0, t1, p1)?;
        if dev <= self.tol || depth >= MAX_DEPTH {
            if dev > self.tol {
                self.failed = Some(self.failed.unwrap_or(0.0).max(dev));
            }
            self.worst = self.worst.max(dev);
            self.out.push(p1);
            return Ok(());
        }
        let tm = t0 + (t1 - t0) * 0.5;
        let pm = self.curve.eval(tm).map_err(geom)?;
        self.go(t0, p0, tm, pm, depth + 1)?;
        self.go(tm, pm, t1, p1, depth + 1)
    }
}

/// Stage 2: tessellate `curve` so that no final chord is further than `tolerance` (world units) from
/// the exact curve at the sampled points. A polyline is passed through, a trimmed line is its two
/// ends, a composite is its segments joined, everything else is subdivided adaptively from
/// eight equal parameter spans (so a closed curve cannot collapse to a point).
pub fn tessellate(curve: &Curve, tolerance: f64) -> Result<Tessellation, TessError> {
    if !(tolerance.is_finite() && tolerance > 0.0) {
        return Err(TessError::BadTolerance);
    }
    match curve {
        Curve::Line(_) => Err(TessError::Unbounded),
        Curve::Polyline(p) => Ok(Tessellation { points: p.vertices().to_vec(), deviation: 0.0 }),
        Curve::Trimmed(t) if matches!(t.basis(), Curve::Line(_)) => {
            let d = curve.domain();
            let a = curve.eval(d.lo()).map_err(geom)?;
            let b = curve.eval(d.hi()).map_err(geom)?;
            Ok(Tessellation { points: vec![a, b], deviation: 0.0 })
        }
        Curve::Composite(c) => {
            let mut points: Vec<Point2> = Vec::new();
            let mut deviation = 0.0f64;
            for seg in c.segments() {
                let t = tessellate(seg, tolerance)?;
                deviation = deviation.max(t.deviation);
                let skip = usize::from(!points.is_empty());
                points.extend(t.points.into_iter().skip(skip));
            }
            Ok(Tessellation { points, deviation })
        }
        other => {
            let d = other.domain();
            let (lo, hi) = (d.lo(), d.hi());
            let first = other.eval(lo).map_err(geom)?;
            let mut r = Refine { curve: other, tol: tolerance, out: vec![first], worst: 0.0, failed: None };
            let mut prev_t = lo;
            let mut prev_p = first;
            for k in 1..=INITIAL_SPLITS {
                let t = if k == INITIAL_SPLITS {
                    hi
                } else {
                    lo + (hi - lo) * (k as f64 / INITIAL_SPLITS as f64)
                };
                let p = other.eval(t).map_err(geom)?;
                r.go(prev_t, prev_p, t, p, 0)?;
                prev_t = t;
                prev_p = p;
            }
            if let Some(achieved) = r.failed {
                return Err(TessError::ToleranceNotMet { achieved });
            }
            Ok(Tessellation { points: r.out, deviation: r.worst })
        }
    }
}

/// The clip rectangle in tile units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipRect {
    /// Left edge.
    pub x0: f64,
    /// Top edge.
    pub y0: f64,
    /// Right edge.
    pub x1: f64,
    /// Bottom edge.
    pub y1: f64,
}

impl ClipRect {
    /// The tile plus `buffer` units on every side.
    pub fn buffered(extent: u32, buffer: u32) -> ClipRect {
        let (e, b) = (extent as f64, buffer as f64);
        ClipRect { x0: -b, y0: -b, x1: e + b, y1: e + b }
    }
}

fn clip_segment(a: (f64, f64), b: (f64, f64), r: ClipRect) -> Option<(f64, f64)> {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, a.0 - r.x0), (dx, r.x1 - a.0), (-dy, a.1 - r.y0), (dy, r.y1 - a.1)] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                if t > t1 {
                    return None;
                }
                t0 = t0.max(t);
            } else {
                if t < t0 {
                    return None;
                }
                t1 = t1.min(t);
            }
        }
    }
    Some((t0, t1))
}

/// Stage 4: clip a polyline (tile units) to `rect`. The result is the pieces that lie inside, each
/// at least two points. Consecutive segments that stay inside stay in one piece; a piece ends where
/// the polyline leaves and a new one starts where it comes back.
pub fn clip_polyline(points: &[(f64, f64)], rect: ClipRect) -> Vec<Vec<(f64, f64)>> {
    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut cur: Vec<(f64, f64)> = Vec::new();
    let flush = |cur: &mut Vec<(f64, f64)>, out: &mut Vec<Vec<(f64, f64)>>| {
        if cur.len() >= 2 {
            out.push(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        match clip_segment(a, b, rect) {
            None => flush(&mut cur, &mut out),
            // a segment that only touches the rectangle at one point draws nothing
            Some((t0, t1)) if t1 <= t0 => flush(&mut cur, &mut out),
            Some((t0, t1)) => {
                let at = |t: f64| {
                    if t == 0.0 {
                        a
                    } else if t == 1.0 {
                        b
                    } else {
                        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
                    }
                };
                if cur.is_empty() || t0 > 0.0 {
                    flush(&mut cur, &mut out);
                    cur.push(at(t0));
                }
                cur.push(at(t1));
                if t1 < 1.0 {
                    flush(&mut cur, &mut out);
                }
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// Stage 5a: round to the integer grid. Repeated points are dropped, and a piece that no longer has
/// two distinct points is dropped with them.
pub fn quantize(pieces: &[Vec<(f64, f64)>]) -> Vec<Vec<(i32, i32)>> {
    let mut out = Vec::new();
    for piece in pieces {
        let mut q: Vec<(i32, i32)> = Vec::with_capacity(piece.len());
        for &(x, y) in piece {
            let p = (x.round() as i32, y.round() as i32);
            if q.last() != Some(&p) {
                q.push(p);
            }
        }
        if q.len() >= 2 {
            out.push(q);
        }
    }
    out
}

fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

/// Stage 5b: the MVT geometry commands for a multi-linestring: `MoveTo(1)`, `LineTo(n)` per piece,
/// with zigzag deltas from the cursor, which carries over between pieces.
pub fn encode_geometry(pieces: &[Vec<(i32, i32)>]) -> Vec<u32> {
    let mut out = Vec::new();
    let (mut cx, mut cy) = (0i32, 0i32);
    for piece in pieces {
        let (first, rest) = match piece.split_first() {
            Some(s) if !s.1.is_empty() => s,
            _ => continue,
        };
        out.push(1 | (1 << 3));
        out.push(zigzag(first.0 - cx));
        out.push(zigzag(first.1 - cy));
        (cx, cy) = *first;
        out.push(2 | ((rest.len() as u32) << 3));
        for &(x, y) in rest {
            out.push(zigzag(x - cx));
            out.push(zigzag(y - cy));
            (cx, cy) = (x, y);
        }
    }
    out
}

/// An entity to export: its id, the layer (component grouping) it goes to, and its exact curve.
#[derive(Clone, Debug, PartialEq)]
pub struct MvtEntity {
    id: EntityId,
    layer: String,
    curve: Curve,
}

impl MvtEntity {
    /// An entity. The layer name must not be empty.
    pub fn new(id: EntityId, layer: &str, curve: Curve) -> Result<MvtEntity, MvtError> {
        if layer.is_empty() {
            return Err(MvtError::EmptyLayerName);
        }
        Ok(MvtEntity { id, layer: layer.to_string(), curve })
    }

    /// The entity id, which becomes the `entity_id` feature attribute.
    pub fn id(&self) -> EntityId {
        self.id
    }

    /// The exact curve.
    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    /// The capability the curve uses, for the ledger.
    pub fn capability(&self) -> CapabilityId {
        match self.curve {
            Curve::Line(_) => ids::LINE,
            Curve::Circle(_) => ids::CIRCLE,
            Curve::Ellipse(_) => ids::ELLIPSE,
            Curve::Polyline(_) => ids::POLYLINE,
            Curve::Trimmed(_) => ids::TRIMMED_CURVE,
            Curve::Composite(_) => ids::COMPOSITE_CURVE,
            Curve::Bezier(_) => ids::BEZIER_CURVE,
            Curve::BSpline(_) => ids::B_SPLINE_CURVE,
            Curve::RationalBSpline(_) => ids::RATIONAL_B_SPLINE_CURVE,
            _ => ids::TRIMMED_CURVE,
        }
    }
}

/// A fidelity ledger that can only hold display-only records.
///
/// [`MvtLedger::record`] refuses `Exact`, `ExactReparameterized` and `EquivalentCompound` at
/// construction, on top of the rule of [`FidelityRecord::new`] that a tessellated target cannot claim
/// exactness. The inner ledger is only ever readable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MvtLedger {
    inner: FidelityLedger,
}

impl MvtLedger {
    /// An empty ledger.
    pub fn new() -> MvtLedger {
        MvtLedger::default()
    }

    /// Records one entity. Only `BoundedApproximation` and `Unsupported` are accepted.
    pub fn record(
        &mut self,
        entity: EntityId,
        capability: CapabilityId,
        fidelity: Fidelity,
        note: &str,
    ) -> Result<(), MvtError> {
        match fidelity.kind() {
            FidelityKind::BoundedApproximation | FidelityKind::Unsupported => {}
            k => return Err(MvtError::NotDisplayOnly(k)),
        }
        let target = TargetConstruct::new(TARGET, Representation::Tessellated)
            .map_err(|_| MvtError::Record("blank target".into()))?;
        let rec = FidelityRecord::new(entity, capability, target, fidelity, note)
            .map_err(|e| MvtError::Record(e.to_string()))?;
        self.inner.record(rec);
        Ok(())
    }

    /// The accumulated ledger.
    pub fn ledger(&self) -> &FidelityLedger {
        &self.inner
    }
}

/// One exported tile.
#[derive(Clone, Debug, PartialEq)]
pub struct MvtTile {
    /// The tile address.
    pub tile: TileId,
    /// The encoded vector tile. Empty (zero layers, still valid) when nothing falls in the tile.
    pub bytes: Vec<u8>,
    /// What the export did to each entity that reaches this tile.
    pub ledger: MvtLedger,
    /// The chord tolerance the tile was tessellated to, in world units.
    pub tessellation_tolerance: f64,
    /// The total world-space deviation every emitted feature stays within (screen budget in world units).
    pub world_budget: f64,
}

// ---- protobuf encoding ------------------------------------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8 & 0x7f) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_key(out: &mut Vec<u8>, field: u32, wire: u8) {
    put_varint(out, ((field as u64) << 3) | wire as u64);
}

fn put_bytes(out: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    put_key(out, field, 2);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn put_packed(out: &mut Vec<u8>, field: u32, values: &[u32]) {
    let mut body = Vec::new();
    for &v in values {
        put_varint(&mut body, v as u64);
    }
    put_bytes(out, field, &body);
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Val {
    Uint(u64),
    Double(u64),
    Bool(bool),
}

struct Feature {
    id: u32,
    deviation: f64,
    geometry: Vec<u32>,
}

#[derive(Default)]
struct LayerBuilder {
    features: Vec<Feature>,
}

impl LayerBuilder {
    fn encode(&self, name: &str, extent: u32) -> Vec<u8> {
        let keys = ["entity_id", "world_deviation", "display_only"];
        let mut values: Vec<Val> = Vec::new();
        let mut intern = |v: Val| -> u32 {
            if let Some(i) = values.iter().position(|x| *x == v) {
                i as u32
            } else {
                values.push(v);
                (values.len() - 1) as u32
            }
        };
        let mut feats = Vec::new();
        for f in &self.features {
            let mut b = Vec::new();
            put_key(&mut b, 1, 0);
            put_varint(&mut b, f.id as u64);
            let tags = [
                0,
                intern(Val::Uint(f.id as u64)),
                1,
                intern(Val::Double(f.deviation.to_bits())),
                2,
                intern(Val::Bool(true)),
            ];
            put_packed(&mut b, 2, &tags);
            put_key(&mut b, 3, 0);
            put_varint(&mut b, 2);
            put_packed(&mut b, 4, &f.geometry);
            feats.push(b);
        }
        let mut out = Vec::new();
        put_bytes(&mut out, 1, name.as_bytes());
        for f in &feats {
            put_bytes(&mut out, 2, f);
        }
        for k in keys {
            put_bytes(&mut out, 3, k.as_bytes());
        }
        for v in &values {
            let mut b = Vec::new();
            match *v {
                Val::Uint(u) => {
                    put_key(&mut b, 5, 0);
                    put_varint(&mut b, u);
                }
                Val::Double(bits) => {
                    put_key(&mut b, 3, 1);
                    b.extend_from_slice(&bits.to_le_bytes());
                }
                Val::Bool(x) => {
                    put_key(&mut b, 7, 0);
                    put_varint(&mut b, x as u64);
                }
            }
            put_bytes(&mut out, 4, &b);
        }
        put_key(&mut out, 5, 0);
        put_varint(&mut out, extent as u64);
        put_key(&mut out, 15, 0);
        put_varint(&mut out, 2);
        out
    }
}

fn reason(text: &str) -> Reason {
    Reason::new(text).unwrap_or_else(|_| unreachable!("reasons written in this module are never blank"))
}

/// Exports one tile: tessellates every entity that can reach it, clips, quantizes, encodes, and
/// records what happened to each one.
///
/// Deterministic: the same entities (in the same order), projection, tile and options give
/// byte-identical output. Layers are written in name order and features in input order.
pub fn export_tile(
    entities: &[MvtEntity],
    projection: &Projection,
    tile: TileId,
    opts: &MvtOptions,
) -> Result<MvtTile, MvtError> {
    let tol = projection.tessellation_tolerance(tile.z, opts)?;
    let world_budget = projection.screen_to_world(tile.z, opts.screen_tolerance_px);
    let q_world = projection.quantization_error(tile.z, opts.extent);
    let rect = ClipRect::buffered(opts.extent, opts.buffer);
    let mut ledger = MvtLedger::new();
    let mut layers: BTreeMap<&str, LayerBuilder> = BTreeMap::new();

    for e in entities {
        if !may_reach(e, projection, tile, opts) {
            continue;
        }
        let cap = e.capability();
        let tess = match tessellate(&e.curve, tol) {
            Ok(t) => t,
            Err(err) => {
                ledger.record(e.id, cap, Fidelity::Unsupported(reason(&err.to_string())), "not drawn")?;
                continue;
            }
        };
        let tile_pts: Vec<(f64, f64)> =
            tess.points.iter().map(|p| projection.to_tile(tile, opts.extent, *p)).collect();
        let clipped = clip_polyline(&tile_pts, rect);
        if clipped.is_empty() {
            continue;
        }
        let pieces = quantize(&clipped);
        if pieces.is_empty() {
            ledger.record(
                e.id,
                cap,
                Fidelity::Unsupported(reason("collapsed to less than one tile unit at this zoom")),
                "not drawn",
            )?;
            continue;
        }
        let total = tess.deviation + q_world;
        let bound = ErrorBound::new(total).map_err(|_| MvtError::Record("non-positive deviation".into()))?;
        ledger.record(
            e.id,
            cap,
            Fidelity::BoundedApproximation(bound),
            "display-only tile geometry: tessellated, clipped and quantized",
        )?;
        layers.entry(e.layer.as_str()).or_default().features.push(Feature {
            id: e.id.index(),
            deviation: total,
            geometry: encode_geometry(&pieces),
        });
    }

    let mut bytes = Vec::new();
    for (name, layer) in &layers {
        put_bytes(&mut bytes, 3, &layer.encode(name, opts.extent));
    }
    Ok(MvtTile { tile, bytes, ledger, tessellation_tolerance: tol, world_budget })
}

/// A cheap reject: whether the entity's bounds, mapped into the tile, touch the buffered tile. When
/// the bounds are unavailable the entity is kept.
fn may_reach(e: &MvtEntity, projection: &Projection, tile: TileId, opts: &MvtOptions) -> bool {
    let Ok(b) = e.curve.bounds() else { return true };
    let (lo, hi) = (b.min(), b.max());
    let rect = ClipRect::buffered(opts.extent, opts.buffer);
    let pts =
        [Point2::new(lo.x, lo.y), Point2::new(hi.x, lo.y), Point2::new(lo.x, hi.y), Point2::new(hi.x, hi.y)]
            .map(|p| projection.to_tile(tile, opts.extent, p));
    let minx = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let maxx = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let miny = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let maxy = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    !(maxx < rect.x0 || minx > rect.x1 || maxy < rect.y0 || miny > rect.y1)
}

/// The fidelity report of a set of tiles (P6.8). `artifact_entities` is the number of entities that reach
/// the tile set: an entity outside every tile has no record, so it is not part of what these tiles say.
/// An entity in several tiles is counted once, in the weakest class it has in any of them.
pub fn report(
    tiles: &[MvtTile],
    source_digest: [u8; 32],
    artifact_entities: u32,
) -> Result<Report, ReportError> {
    Report::new(
        "Mapbox Vector Tile",
        "2",
        source_digest,
        artifact_entities,
        tiles.iter().flat_map(|t| t.ledger.ledger().records()),
    )
}
