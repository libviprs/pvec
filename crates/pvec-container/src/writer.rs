//! The deterministic container writer (issue P4.4).
//!
//! The same semantic input gives the same bytes: whatever order the entities, capabilities and
//! component streams were handed over in, whichever thread pushed them, whichever process or
//! machine ran the write, and at whatever time. That is what lets an artifact be cached by digest,
//! diffed in review and used as the baseline of a fidelity report.
//!
//! Three things make it so, and `docs/CONTAINER.md` chapter 4 is the contract for all of them:
//!
//! * the canonical order of blocks, entities, capabilities and streams is decided here, by keys
//!   the caller never sees, and never by the order of arrival ([`SortKey`]);
//! * every scalar is written with `to_le_bytes`, so the host byte order is not consulted;
//! * nothing ambient reaches the output: no clock, path, host name, hash-map iteration or address.
//!   The producer identity is whatever the caller put in [`WriterConfig`].

use crate::blocks::advanced::{AdvancedBuilder, SplineSpec};
use crate::blocks::arena::{ArenaError, PayloadKind};
use crate::blocks::hot::{HotBlockBuilder, HotError, HotKind};
use crate::directory::{
    BLOCK_ALIGN, BlockEntry, BlockKind, CODEC_NONE, ENTRY_LEN, FLAG_HAS_BOUNDS, FLAG_MUST_UNDERSTAND,
};
use crate::error::ContainerError;
use crate::header::{ChecksumAlgorithm, Header};
use crate::manifest::{Bounds, DirectoryPointer, LengthUnit, Manifest, Metadata};
use crate::spec::{cap, dir};
use core::cmp::Ordering;
use core::fmt;
use pvec_capabilities::{CapabilityId, Declared, Registry, Version};
use pvec_components::{Stream, StreamError, encode_streams};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d, GeometryError, Point2};
use pvec_numeric::check_value;
use std::collections::BTreeMap;

/// A key of the canonical ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// Blocks by block kind.
    BlockKind,
    /// Blocks by spatial cell id.
    SpatialCell,
    /// Blocks by geometry type.
    GeometryType,
    /// Entities in a block by `EntityId`.
    EntityId,
    /// Manifest capabilities by capability id.
    CapabilityId,
    /// Component streams by stream id.
    StreamId,
}

impl SortKey {
    /// Every key.
    pub const ALL: [SortKey; 6] = [
        SortKey::BlockKind,
        SortKey::SpatialCell,
        SortKey::GeometryType,
        SortKey::EntityId,
        SortKey::CapabilityId,
        SortKey::StreamId,
    ];
}

/// What the caller decides about an artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct WriterConfig {
    /// Unit of length of source space.
    pub unit: LengthUnit,
    /// Producer identity and corpus revision, as the caller supplies them.
    pub metadata: Metadata,
    /// Source bounds; the union of the entity bounds when `None`.
    pub bounds: Option<Bounds>,
    /// Capabilities to declare, in any order.
    pub capabilities: Vec<CapabilityId>,
}

/// What every entity carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityMeta {
    /// The entity id.
    pub id: EntityId,
    /// Opaque style reference.
    pub style: u32,
    /// Spatial cell id.
    pub cell: u32,
    /// Bounds of the entity in source space.
    pub bounds: Bounds,
}

/// Why writing failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WriteError {
    /// There is no entity to write.
    Empty,
    /// An entity id is used twice.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// An advanced curve was given a spatial cell.
    CellOnAdvanced {
        /// The id.
        id: u32,
    },
    /// A capability id the registry does not know.
    UnknownCapability {
        /// The id.
        id: u32,
    },
    /// Two component streams of one capability.
    DuplicateStream {
        /// The capability id.
        capability: u32,
    },
    /// Bounds that are not finite, in range and ordered.
    BadBounds {
        /// The entity, or `None` for the source bounds.
        id: Option<u32>,
    },
    /// A block or the file does not fit its length fields.
    TooLarge,
    /// A curve the writer cannot store.
    Unsupported {
        /// What it was.
        what: &'static str,
    },
    /// A hot block builder refused.
    Hot(HotError),
    /// An advanced block builder refused.
    Arena(ArenaError),
    /// The component stream encoder refused.
    Stream(StreamError),
    /// The manifest refused.
    Container(ContainerError),
    /// The semantic model refused.
    Geometry(GeometryError),
}

impl WriteError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            WriteError::Empty => "WRT-01",
            WriteError::DuplicateId { .. } => "WRT-02",
            WriteError::CellOnAdvanced { .. } => "WRT-03",
            WriteError::UnknownCapability { .. } => "WRT-04",
            WriteError::DuplicateStream { .. } => "WRT-05",
            WriteError::BadBounds { .. } => "WRT-06",
            WriteError::TooLarge => "WRT-07",
            WriteError::Unsupported { .. } => "WRT-08",
            WriteError::Hot(_) => "WRT-09",
            WriteError::Arena(_) => "WRT-10",
            WriteError::Stream(_) => "WRT-11",
            WriteError::Container(_) => "WRT-12",
            WriteError::Geometry(_) => "WRT-13",
        }
    }
}

/// The refusals, rule by rule.
pub const WRITE_RULES: &[(&str, &str)] = &[
    ("WRT-01", "there is no entity to write (component streams alone are not an artifact)"),
    ("WRT-02", "an entity id is used by two entities, whatever their geometry types"),
    ("WRT-03", "an advanced curve was given a spatial cell; cells belong to hot blocks"),
    ("WRT-04", "a capability id the registry does not know"),
    ("WRT-05", "two component streams of one capability"),
    ("WRT-06", "bounds that are not finite, in range and ordered, on an entity or as the source bounds"),
    ("WRT-07", "a block or the artifact does not fit its 32-bit length fields"),
    ("WRT-08", "a curve the container cannot store exactly and deterministically"),
    ("WRT-09", "a hot block builder refused (the error carries its HOT rule)"),
    ("WRT-10", "an advanced block builder refused (the error carries its ARN rule)"),
    ("WRT-11", "the component stream encoder refused"),
    ("WRT-12", "the manifest refused (the error carries its container error)"),
    ("WRT-13", "the semantic model refused"),
];

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            WriteError::Empty => write!(f, "{rule}: there is no entity to write"),
            WriteError::DuplicateId { id } => write!(f, "{rule}: entity id {id} is used twice"),
            WriteError::CellOnAdvanced { id } => {
                write!(f, "{rule}: advanced curve {id} was given a spatial cell")
            }
            WriteError::UnknownCapability { id } => write!(f, "{rule}: the registry has no capability {id}"),
            WriteError::DuplicateStream { capability } => {
                write!(f, "{rule}: two component streams of capability {capability}")
            }
            WriteError::BadBounds { id: Some(id) } => write!(f, "{rule}: entity {id} has invalid bounds"),
            WriteError::BadBounds { id: None } => write!(f, "{rule}: the source bounds are invalid"),
            WriteError::TooLarge => {
                write!(f, "{rule}: a block or the artifact does not fit its length fields")
            }
            WriteError::Unsupported { what } => write!(f, "{rule}: cannot store a {what}"),
            WriteError::Hot(e) => write!(f, "{rule}: {e}"),
            WriteError::Arena(e) => write!(f, "{rule}: {e}"),
            WriteError::Stream(e) => write!(f, "{rule}: component stream: {e:?}"),
            WriteError::Container(e) => write!(f, "{rule}: {e}"),
            WriteError::Geometry(e) => write!(f, "{rule}: {e:?}"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<HotError> for WriteError {
    fn from(e: HotError) -> WriteError {
        WriteError::Hot(e)
    }
}

impl From<ArenaError> for WriteError {
    fn from(e: ArenaError) -> WriteError {
        WriteError::Arena(e)
    }
}

impl From<StreamError> for WriteError {
    fn from(e: StreamError) -> WriteError {
        WriteError::Stream(e)
    }
}

impl From<ContainerError> for WriteError {
    fn from(e: ContainerError) -> WriteError {
        WriteError::Container(e)
    }
}

impl From<GeometryError> for WriteError {
    fn from(e: GeometryError) -> WriteError {
        WriteError::Geometry(e)
    }
}

/// The geometry of a hot entity, as it was pushed.
#[derive(Clone, Debug)]
enum HotShape {
    Line([f64; 2], [f64; 2]),
    Polyline(Vec<[f64; 2]>),
    CircleArc { centre: [f64; 2], radius: f64, start_sweep: [f64; 2] },
    EllipseArc { centre: [f64; 2], radii: [f64; 2], angles: [f64; 3] },
}

impl HotShape {
    fn kind(&self) -> HotKind {
        match self {
            HotShape::Line(..) => HotKind::Line,
            HotShape::Polyline(_) => HotKind::Polyline,
            HotShape::CircleArc { .. } => HotKind::CircleArc,
            HotShape::EllipseArc { .. } => HotKind::EllipseArc,
        }
    }
}

#[derive(Clone, Debug)]
struct HotItem {
    seq: usize,
    meta: EntityMeta,
    shape: HotShape,
}

#[derive(Clone, Debug)]
enum AdvancedShape {
    Spline {
        degree: usize,
        knots: Vec<f64>,
        x: Vec<f64>,
        y: Vec<f64>,
        weights: Option<Vec<f64>>,
        closed: bool,
    },
    Composite {
        segments: Vec<EntityId>,
        trims: Option<Vec<[f64; 2]>>,
        closed: bool,
    },
}

#[derive(Clone, Debug)]
struct AdvancedItem {
    seq: usize,
    meta: EntityMeta,
    shape: AdvancedShape,
}

/// A block ready to be placed.
struct Planned {
    first_seen: usize,
    kind: BlockKind,
    cell: u32,
    subtype: u16,
    count: u32,
    bounds: Option<Bounds>,
    bytes: Vec<u8>,
}

/// The writer.
///
/// Push entities and streams in any order, then [`ContainerWriter::finish`]. The order of the pushes
/// is never visible in the output.
#[derive(Debug)]
pub struct ContainerWriter {
    config: WriterConfig,
    skipped: Vec<SortKey>,
    seq: usize,
    hot: Vec<HotItem>,
    advanced: Vec<AdvancedItem>,
    streams: Vec<(usize, Stream)>,
}

fn normal(v: f64) -> f64 {
    if v == 0.0 { 0.0 } else { v }
}

fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: [normal(a.min[0].min(b.min[0])), normal(a.min[1].min(b.min[1]))],
        max: [normal(a.max[0].max(b.max[0])), normal(a.max[1].max(b.max[1]))],
    }
}

/// `bounds` if every value is finite and in range under the numeric policy and the box is ordered.
fn checked_bounds(bounds: Bounds) -> Option<Bounds> {
    let mut v = [0.0f64; 4];
    for (slot, x) in v.iter_mut().zip([bounds.min[0], bounds.min[1], bounds.max[0], bounds.max[1]]) {
        *slot = check_value(x).ok()?;
    }
    if v[0] > v[2] || v[1] > v[3] {
        return None;
    }
    Some(Bounds { min: [v[0], v[1]], max: [v[2], v[3]] })
}

fn bounds_of(curve: &Curve) -> Result<Bounds, WriteError> {
    let b = curve.bounds()?;
    Ok(Bounds { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] })
}

fn xy(p: Point2) -> [f64; 2] {
    [p.x, p.y]
}

fn align8(n: u64) -> u64 {
    n.div_ceil(BLOCK_ALIGN) * BLOCK_ALIGN
}

impl ContainerWriter {
    /// A writer with nothing in it.
    pub fn new(config: WriterConfig) -> ContainerWriter {
        ContainerWriter {
            config,
            skipped: Vec::new(),
            seq: 0,
            hot: Vec::new(),
            advanced: Vec::new(),
            streams: Vec::new(),
        }
    }

    /// Switches one sort off, so a test can show that the determinism suite notices.
    ///
    /// With a sort off the writer falls back to the order of arrival for that key, which is exactly
    /// the bug the key exists to prevent. Nothing but the mutation tests calls it.
    #[doc(hidden)]
    pub fn skip_sort(&mut self, key: SortKey) {
        if !self.skipped.contains(&key) {
            self.skipped.push(key);
        }
    }

    fn sorts(&self, key: SortKey) -> bool {
        !self.skipped.contains(&key)
    }

    fn next_seq(&mut self) -> usize {
        self.seq += 1;
        self.seq
    }

    fn checked_meta(meta: EntityMeta) -> Result<EntityMeta, WriteError> {
        match checked_bounds(meta.bounds) {
            Some(bounds) => Ok(EntityMeta { bounds, ..meta }),
            None => Err(WriteError::BadBounds { id: Some(meta.id.index()) }),
        }
    }

    fn push_hot(&mut self, meta: EntityMeta, shape: HotShape) -> Result<(), WriteError> {
        let meta = ContainerWriter::checked_meta(meta)?;
        let seq = self.next_seq();
        self.hot.push(HotItem { seq, meta, shape });
        Ok(())
    }

    /// Adds a line segment from `a` to `b`.
    pub fn push_line(&mut self, meta: EntityMeta, a: [f64; 2], b: [f64; 2]) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::Line(a, b))
    }

    /// Adds a polyline of 2 to 2^20 vertices.
    pub fn push_polyline(&mut self, meta: EntityMeta, points: &[[f64; 2]]) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::Polyline(points.to_vec()))
    }

    /// Adds a circle or circular arc: the centre, the radius, and the start angle and signed sweep in
    /// radians.
    pub fn push_circle_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radius: f64,
        start_sweep: [f64; 2],
    ) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::CircleArc { centre, radius, start_sweep })
    }

    /// Adds an ellipse or elliptical arc: the centre, the two semi-axes, and the rotation, start angle
    /// and signed sweep in radians.
    pub fn push_ellipse_arc(
        &mut self,
        meta: EntityMeta,
        centre: [f64; 2],
        radii: [f64; 2],
        angles: [f64; 3],
    ) -> Result<(), WriteError> {
        self.push_hot(meta, HotShape::EllipseArc { centre, radii, angles })
    }

    fn push_advanced(&mut self, meta: EntityMeta, shape: AdvancedShape) -> Result<(), WriteError> {
        let meta = ContainerWriter::checked_meta(meta)?;
        if meta.cell != 0 {
            return Err(WriteError::CellOnAdvanced { id: meta.id.index() });
        }
        let seq = self.next_seq();
        self.advanced.push(AdvancedItem { seq, meta, shape });
        Ok(())
    }

    /// Adds a B-spline. Its cell must be 0: advanced curves share one set of blocks.
    pub fn push_spline(&mut self, meta: EntityMeta, spec: SplineSpec<'_>) -> Result<(), WriteError> {
        let shape = AdvancedShape::Spline {
            degree: spec.degree,
            knots: spec.knots.to_vec(),
            x: spec.x.to_vec(),
            y: spec.y.to_vec(),
            weights: spec.weights.map(<[f64]>::to_vec),
            closed: spec.closed,
        };
        if spec.periodic {
            return Err(WriteError::Arena(ArenaError::PeriodicUnsupported));
        }
        self.push_advanced(meta, shape)
    }

    /// Adds a composite curve naming other advanced curves by id, with optional `(t1, t2)` trims.
    /// Its cell must be 0.
    pub fn push_composite(
        &mut self,
        meta: EntityMeta,
        segments: &[EntityId],
        trims: Option<&[[f64; 2]]>,
        closed: bool,
    ) -> Result<(), WriteError> {
        let shape = AdvancedShape::Composite {
            segments: segments.to_vec(),
            trims: trims.map(<[[f64; 2]]>::to_vec),
            closed,
        };
        self.push_advanced(meta, shape)
    }

    /// Adds a curve of the semantic model (P3.1) with the bounds the model reports for it.
    ///
    /// Polylines, B-splines, rational B-splines and Bezier curves are stored as they are; a trimmed
    /// line is the segment between its end points; a circle, an ellipse or an arc of either is stored
    /// when its frame is the axis-aligned one. What cannot be stored exactly and the same on every
    /// machine is refused, never approximated: an unbounded line, a composite (its segments need ids,
    /// so push them with [`ContainerWriter::push_composite`]), a trimmed spline, and a rotated frame
    /// (its angle needs `atan2`, whose last bit belongs to the C library and not to the format).
    pub fn push_curve(
        &mut self,
        id: EntityId,
        style: u32,
        cell: u32,
        curve: &Curve,
    ) -> Result<(), WriteError> {
        let unsupported = |what| Err(WriteError::Unsupported { what });
        let axis_aligned = |x: pvec_geometry::Vector2| x.x == 1.0 && x.y == 0.0;
        match curve {
            Curve::Line(_) => unsupported("unbounded line"),
            Curve::Composite(_) => unsupported("composite curve"),
            Curve::Polyline(p) => {
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                let points: Vec<[f64; 2]> = p.vertices().iter().map(|v| xy(*v)).collect();
                self.push_polyline(meta, &points)
            }
            Curve::BSpline(b) => {
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let spec = SplineSpec {
                    degree: b.degree(),
                    knots: b.knots(),
                    x: &x,
                    y: &y,
                    weights: None,
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta, spec)
            }
            Curve::RationalBSpline(b) => {
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let spec = SplineSpec {
                    degree: b.degree(),
                    knots: b.knots(),
                    x: &x,
                    y: &y,
                    weights: Some(b.weights()),
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta, spec)
            }
            Curve::Bezier(b) => {
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                let (x, y): (Vec<f64>, Vec<f64>) = b.control_points().iter().map(|p| (p.x, p.y)).unzip();
                let degree = b.degree();
                let mut knots = vec![0.0; degree + 1];
                knots.extend(std::iter::repeat_n(1.0, degree + 1));
                let spec = SplineSpec {
                    degree,
                    knots: &knots,
                    x: &x,
                    y: &y,
                    weights: None,
                    closed: b.is_declared_closed(),
                    periodic: false,
                };
                self.push_spline(meta, spec)
            }
            Curve::Circle(c) => {
                if !axis_aligned(c.placement().x_axis()) {
                    return unsupported("rotated placement");
                }
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                self.push_circle_arc(
                    meta,
                    xy(c.placement().origin()),
                    c.radius(),
                    [0.0, core::f64::consts::TAU],
                )
            }
            Curve::Ellipse(e) => {
                if !axis_aligned(e.placement().x_axis()) {
                    return unsupported("rotated placement");
                }
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                let radii = [e.semi_axis_1(), e.semi_axis_2()];
                self.push_ellipse_arc(
                    meta,
                    xy(e.placement().origin()),
                    radii,
                    [0.0, 0.0, core::f64::consts::TAU],
                )
            }
            Curve::Trimmed(t) => {
                let trim = t.trim();
                let (start, sweep) = if trim.reversed {
                    (trim.start + trim.span, -trim.span)
                } else {
                    (trim.start, trim.span)
                };
                let meta = EntityMeta { id, style, cell, bounds: bounds_of(curve)? };
                match t.basis() {
                    Curve::Line(_) => {
                        let d = curve.domain();
                        let (a, b) = (curve.eval(d.lo())?, curve.eval(d.hi())?);
                        self.push_line(meta, xy(a), xy(b))
                    }
                    Curve::Circle(c) => {
                        if !axis_aligned(c.placement().x_axis()) {
                            return unsupported("rotated placement");
                        }
                        self.push_circle_arc(meta, xy(c.placement().origin()), c.radius(), [start, sweep])
                    }
                    Curve::Ellipse(e) => {
                        if !axis_aligned(e.placement().x_axis()) {
                            return unsupported("rotated placement");
                        }
                        let radii = [e.semi_axis_1(), e.semi_axis_2()];
                        self.push_ellipse_arc(meta, xy(e.placement().origin()), radii, [0.0, start, sweep])
                    }
                    _ => unsupported("trimmed spline"),
                }
            }
            _ => unsupported("curve of a kind this writer does not know"),
        }
    }

    /// Adds a component stream. Its capability is declared in the manifest as the registry declares it.
    pub fn push_stream(&mut self, stream: Stream) -> Result<(), WriteError> {
        let seq = self.next_seq();
        self.streams.push((seq, stream));
        Ok(())
    }

    /// The capabilities to declare: the caller's and those of the streams, each once, in id order.
    fn declarations(&self) -> Result<(Vec<Declared>, Version), WriteError> {
        let mut wanted: Vec<CapabilityId> = Vec::new();
        for id in
            self.config.capabilities.iter().copied().chain(self.streams.iter().map(|(_, s)| s.capability))
        {
            if !wanted.contains(&id) {
                wanted.push(id);
            }
        }
        if self.sorts(SortKey::CapabilityId) {
            wanted.sort_unstable();
        }
        let registry = Registry::v1();
        let mut min_reader = Version::new(1, 0, 0);
        let mut out = Vec::with_capacity(wanted.len());
        for id in wanted {
            let c = registry.get(id).ok_or(WriteError::UnknownCapability { id: id.get() })?;
            min_reader = min_reader.max(c.min_reader);
            out.push(Declared { id, requirement: c.requirement, version: c.version });
        }
        Ok((out, min_reader))
    }

    fn hot_blocks(&self) -> Result<Vec<Planned>, WriteError> {
        let mut index: BTreeMap<(u32, u16), usize> = BTreeMap::new();
        let mut groups: Vec<(HotKind, u32, Vec<&HotItem>)> = Vec::new();
        for item in &self.hot {
            let (kind, key) = (item.shape.kind(), (item.meta.cell, item.shape.kind().id()));
            let at = match index.get(&key) {
                Some(at) => *at,
                None => {
                    groups.push((kind, item.meta.cell, Vec::new()));
                    index.insert(key, groups.len() - 1);
                    groups.len() - 1
                }
            };
            groups[at].2.push(item);
        }
        let mut out = Vec::with_capacity(groups.len());
        for (kind, cell, mut items) in groups {
            let first_seen = items.first().map_or(0, |i| i.seq);
            if self.sorts(SortKey::EntityId) {
                items.sort_by_key(|i| i.meta.id);
            }
            let mut builder = HotBlockBuilder::new(kind);
            for i in &items {
                let (id, style) = (i.meta.id, i.meta.style);
                match &i.shape {
                    HotShape::Line(a, b) => builder.push_line(id, style, *a, *b)?,
                    HotShape::Polyline(points) => builder.push_polyline(id, style, points)?,
                    HotShape::CircleArc { centre, radius, start_sweep } => {
                        builder.push_circle_arc(id, style, *centre, *radius, *start_sweep)?
                    }
                    HotShape::EllipseArc { centre, radii, angles } => {
                        builder.push_ellipse_arc(id, style, *centre, *radii, *angles)?
                    }
                }
            }
            let block =
                if self.sorts(SortKey::EntityId) { builder.finish()? } else { builder.finish_unsorted()? };
            let bounds = items.iter().map(|i| i.meta.bounds).reduce(union);
            out.push(Planned {
                first_seen,
                kind: BlockKind::Hot,
                cell,
                subtype: kind.id(),
                count: u32::try_from(items.len()).map_err(|_| WriteError::TooLarge)?,
                bounds,
                bytes: block.as_bytes().to_vec(),
            });
        }
        Ok(out)
    }

    fn advanced_blocks(&self) -> Result<Vec<Planned>, WriteError> {
        let Some(first) = self.advanced.first() else {
            return Ok(Vec::new());
        };
        let mut items: Vec<&AdvancedItem> = self.advanced.iter().collect();
        if self.sorts(SortKey::EntityId) {
            items.sort_by_key(|i| i.meta.id);
        }
        let mut builder = AdvancedBuilder::new();
        for i in &items {
            let (id, style) = (i.meta.id, i.meta.style);
            match &i.shape {
                AdvancedShape::Spline { degree, knots, x, y, weights, closed } => builder.push_spline(
                    id,
                    style,
                    SplineSpec {
                        degree: *degree,
                        knots,
                        x,
                        y,
                        weights: weights.as_deref(),
                        closed: *closed,
                        periodic: false,
                    },
                )?,
                AdvancedShape::Composite { segments, trims, closed } => {
                    builder.push_composite(id, style, segments, trims.as_deref(), *closed)?
                }
            }
        }
        let blocks = builder.finish()?;
        let s = blocks.slices();
        let bounds = items.iter().map(|i| i.meta.bounds).reduce(union);
        let first_seen = first.seq;
        let mut out = Vec::with_capacity(6);
        let parts: [(BlockKind, u16, &[u8], Option<Bounds>); 6] = [
            (BlockKind::Records, 0, s.records, bounds),
            (BlockKind::Arena, PayloadKind::Knots.id(), s.knots, None),
            (BlockKind::Arena, PayloadKind::Weights.id(), s.weights, None),
            (BlockKind::Arena, PayloadKind::ControlPoints.id(), s.control_points, None),
            (BlockKind::Arena, PayloadKind::Segments.id(), s.segments, None),
            (BlockKind::Arena, PayloadKind::Trims.id(), s.trims, None),
        ];
        for (kind, subtype, bytes, bounds) in parts {
            // every block of the six starts with its magic, version and kind, then the entry count
            let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
            out.push(Planned { first_seen, kind, cell: 0, subtype, count, bounds, bytes: bytes.to_vec() });
        }
        Ok(out)
    }

    fn stream_block(&self) -> Result<Option<Planned>, WriteError> {
        let Some((first_seen, _)) = self.streams.first() else {
            return Ok(None);
        };
        let mut seen: Vec<u32> = self.streams.iter().map(|(_, s)| s.capability.get()).collect();
        seen.sort_unstable();
        if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
            return Err(WriteError::DuplicateStream { capability: w[0] });
        }
        let mut streams: Vec<Stream> = self.streams.iter().map(|(_, s)| s.clone()).collect();
        if self.sorts(SortKey::StreamId) {
            streams.sort_by_key(|s| s.capability);
        }
        let bytes = encode_streams(&streams)?;
        let count = u32::try_from(streams.len()).map_err(|_| WriteError::TooLarge)?;
        Ok(Some(Planned {
            first_seen: *first_seen,
            kind: BlockKind::Components,
            cell: 0,
            subtype: 0,
            count,
            bounds: None,
            bytes,
        }))
    }

    /// The artifact.
    ///
    /// Header, manifest, the blocks in canonical order each on a multiple of 8 bytes, then the block
    /// directory, and nothing else. Every padding byte is zero.
    pub fn finish(self) -> Result<Vec<u8>, WriteError> {
        if self.hot.is_empty() && self.advanced.is_empty() {
            return Err(WriteError::Empty);
        }
        let mut ids: Vec<u32> = self
            .hot
            .iter()
            .map(|i| i.meta.id.index())
            .chain(self.advanced.iter().map(|i| i.meta.id.index()))
            .collect();
        ids.sort_unstable();
        if let Some(w) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(WriteError::DuplicateId { id: w[0] });
        }
        let (capabilities, min_reader) = self.declarations()?;
        let source_bounds = match self.config.bounds {
            Some(b) => checked_bounds(b).ok_or(WriteError::BadBounds { id: None })?,
            None => self
                .hot
                .iter()
                .map(|i| i.meta.bounds)
                .chain(self.advanced.iter().map(|i| i.meta.bounds))
                .reduce(union)
                .ok_or(WriteError::Empty)?,
        };

        let mut blocks = self.hot_blocks()?;
        blocks.extend(self.advanced_blocks()?);
        blocks.extend(self.stream_block()?);
        // arrival order first, so that a key that is switched off falls back to it and nothing else
        blocks.sort_by_key(|b| b.first_seen);
        let by = |key: SortKey, a: Ordering| if self.sorts(key) { a } else { Ordering::Equal };
        blocks.sort_by(|a, b| {
            by(SortKey::BlockKind, a.kind.id().cmp(&b.kind.id()))
                .then(by(SortKey::SpatialCell, a.cell.cmp(&b.cell)))
                .then(by(SortKey::GeometryType, a.subtype.cmp(&b.subtype)))
        });

        let mut manifest = Manifest {
            unit: self.config.unit,
            bounds: source_bounds,
            min_reader,
            capabilities,
            directories: vec![DirectoryPointer { kind: DirectoryPointer::KIND_BLOCK, offset: 0, length: 0 }],
            block_checksums: None,
            artifact_digest: None,
            metadata: self.config.metadata.clone(),
        };
        // the manifest's length depends on counts, not on the offsets it holds, so it is known now
        let manifest_len = manifest.encode()?.len() as u64;
        let mut at = Header::LEN as u64 + manifest_len;
        let mut entries: Vec<BlockEntry> = Vec::with_capacity(blocks.len());
        let mut offsets: Vec<u64> = Vec::with_capacity(blocks.len());
        for b in &blocks {
            let stored_length = u32::try_from(b.bytes.len()).map_err(|_| WriteError::TooLarge)?;
            at = align8(at);
            offsets.push(at);
            let mut flags = if b.kind == BlockKind::Components { 0 } else { FLAG_MUST_UNDERSTAND };
            if b.bounds.is_some() {
                flags |= FLAG_HAS_BOUNDS;
            }
            entries.push(BlockEntry {
                offset: at,
                stored_length,
                uncompressed_length: stored_length,
                count: b.count,
                cell: b.cell,
                bounds: b.bounds,
                kind: b.kind.id(),
                subtype: b.subtype,
                flags,
                codec: CODEC_NONE,
            });
            at += u64::from(stored_length);
        }
        let directory_offset = align8(at);
        let directory_length = (entries.len() * ENTRY_LEN) as u64;
        let total = directory_offset.checked_add(directory_length).ok_or(WriteError::TooLarge)?;
        let total = usize::try_from(total).map_err(|_| WriteError::TooLarge)?;
        manifest.directories[0] = DirectoryPointer {
            kind: DirectoryPointer::KIND_BLOCK,
            offset: directory_offset,
            length: directory_length,
        };
        let manifest_bytes = manifest.encode()?;
        let header = Header {
            major: Header::MAJOR,
            minor: Header::MINOR,
            header_len: Header::LEN as u16,
            block_checksum: ChecksumAlgorithm::Xxh3_64,
            artifact_checksum: ChecksumAlgorithm::Sha256,
            manifest_offset: Header::LEN as u64,
            manifest_length: manifest_len,
            directory_table_offset: Header::LEN as u64
                + Manifest::FIXED_LEN as u64
                + (cap::SIZE * manifest.capabilities.len()) as u64,
            directory_table_length: dir::SIZE as u64,
        };

        let mut out: Vec<u8> = Vec::with_capacity(total);
        out.extend_from_slice(&header.encode());
        out.extend_from_slice(&manifest_bytes);
        for (b, offset) in blocks.iter().zip(&offsets) {
            out.resize(*offset as usize, 0);
            out.extend_from_slice(&b.bytes);
        }
        out.resize(directory_offset as usize, 0);
        for e in &entries {
            out.extend_from_slice(&e.encode());
        }
        debug_assert_eq!(out.len(), total);
        Ok(out)
    }
}
