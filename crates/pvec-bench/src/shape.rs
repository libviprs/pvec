//! The shapes the corpus generator draws, their counts by capability, and the one function that turns a list
//! of them into a `.pvec` artifact through the production writer.

use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::directory::STRADDLER_CELL_FIRST;
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, WriteError, WriterConfig};
use pvec_container::{BlockId, Bounds, ContainerReader, LengthUnit, Metadata, ReaderConfig};
use pvec_entity::EntityId;
use pvec_stream::{
    Aabb, EntityBox, LongEntity, PayloadRef, STRADDLER_FACTOR, build_directory, build_straddlers, classify,
    payload_cells, scan_payloads,
};
use std::collections::{BTreeMap, HashMap};
use std::f64::consts::TAU;

/// A B-spline: the numbers that vary together per curve.
#[derive(Clone, Debug, PartialEq)]
pub struct SplineData {
    /// Degree.
    pub degree: usize,
    /// Knots with multiplicities, `control points + degree + 1` of them.
    pub knots: Vec<f64>,
    /// X of each control point.
    pub x: Vec<f64>,
    /// Y of each control point.
    pub y: Vec<f64>,
    /// Weights, which makes the curve rational.
    pub weights: Option<Vec<f64>>,
}

/// One drawn entity, in the vocabulary of the hot and advanced blocks.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// A segment.
    Line([f64; 2], [f64; 2]),
    /// A polyline of at least two vertices.
    Polyline(Vec<[f64; 2]>),
    /// A circle or arc: centre, radius, start angle and signed sweep.
    Arc {
        /// Centre.
        centre: [f64; 2],
        /// Radius.
        radius: f64,
        /// Start angle and signed sweep in radians.
        start_sweep: [f64; 2],
    },
    /// An ellipse or arc of one: centre, semi-axes, rotation, start angle and signed sweep.
    Ellipse {
        /// Centre.
        centre: [f64; 2],
        /// The two semi-axes.
        radii: [f64; 2],
        /// Rotation, start angle and signed sweep in radians.
        angles: [f64; 3],
    },
    /// A B-spline, rational when it has weights.
    Spline(Box<SplineData>),
}

impl Shape {
    /// The registry capability this shape exercises: a full circle is a circle, a partial one a trimmed curve.
    pub fn capability(&self) -> &'static str {
        match self {
            Shape::Line(..) => "line",
            Shape::Polyline(_) => "polyline",
            Shape::Arc { start_sweep, .. } if start_sweep[1].abs() == TAU => "circle",
            Shape::Ellipse { angles, .. } if angles[2].abs() == TAU => "ellipse",
            Shape::Arc { .. } | Shape::Ellipse { .. } => "trimmed_curve",
            Shape::Spline(s) if s.weights.is_some() => "rational_b_spline_curve",
            Shape::Spline(_) => "b_spline_curve",
        }
    }

    /// A box that holds the shape: exact for lines and polylines, the circle's box for an arc, the box of the
    /// larger semi-axis for an ellipse, the control hull for a spline. It uses no trigonometry, so it is the
    /// same on every machine.
    pub fn bounds(&self) -> [f64; 4] {
        fn hull(pts: impl Iterator<Item = [f64; 2]>) -> [f64; 4] {
            pts.fold([f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY], |b, p| {
                [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])]
            })
        }
        match self {
            Shape::Line(a, b) => hull([*a, *b].into_iter()),
            Shape::Polyline(p) => hull(p.iter().copied()),
            Shape::Arc { centre: c, radius: r, .. } => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
            Shape::Ellipse { centre: c, radii, .. } => {
                let m = radii[0].max(radii[1]);
                [c[0] - m, c[1] - m, c[0] + m, c[1] + m]
            }
            Shape::Spline(s) => hull(s.x.iter().zip(&s.y).map(|(x, y)| [*x, *y])),
        }
    }
}

/// Counts by capability name, and the box of everything.
pub fn summarise(shapes: &[Shape]) -> (BTreeMap<&'static str, u64>, [f64; 4]) {
    let mut counts = BTreeMap::new();
    let mut all = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for s in shapes {
        *counts.entry(s.capability()).or_insert(0) += 1;
        let b = s.bounds();
        all = [all[0].min(b[0]), all[1].min(b[1]), all[2].max(b[2]), all[3].max(b[3])];
    }
    (counts, all)
}

fn eid(i: usize) -> EntityId {
    EntityId::new(u32::try_from(i).expect("fewer than 2^32 entities")).expect("an entity id")
}

fn aabb(b: [f64; 4]) -> Aabb {
    Aabb::new([b[0], b[1]], [b[2], b[3]]).expect("a box")
}

fn to_bounds(b: [f64; 4]) -> Bounds {
    Bounds { min: [b[0], b[1]], max: [b[2], b[3]] }
}

fn push(w: &mut ContainerWriter, id: usize, cell: u32, shape: &Shape) -> Result<(), WriteError> {
    let meta = EntityMeta { id: eid(id), style: 0, cell, bounds: to_bounds(shape.bounds()) };
    match shape {
        Shape::Line(a, b) => w.push_line(meta, *a, *b),
        Shape::Polyline(p) => w.push_polyline(meta, p),
        Shape::Arc { centre, radius, start_sweep } => w.push_circle_arc(meta, *centre, *radius, *start_sweep),
        Shape::Ellipse { centre, radii, angles } => w.push_ellipse_arc(meta, *centre, *radii, *angles),
        Shape::Spline(s) => {
            let spec = SplineSpec {
                degree: s.degree,
                knots: &s.knots,
                x: &s.x,
                y: &s.y,
                weights: s.weights.as_deref(),
                closed: false,
                periodic: false,
            };
            w.push_spline(meta, spec)
        }
    }
}

/// Writes `shapes` as one artifact: ids are the indices, B-splines go to the advanced set (cell 0), everything
/// else to the hot blocks under a spatial directory, and the entities that span many cells go through the
/// straddler strategy (P5.3b: lay out once to learn the block ids, then again with the lists).
pub fn artifact(shapes: &[Shape], unit: LengthUnit, producer: &str) -> Result<Vec<u8>, String> {
    let (_, all) = summarise(shapes);
    let config = WriterConfig {
        unit,
        metadata: Metadata {
            producer_id: "pvec-bench".into(),
            producer_version: producer.into(),
            corpus_revision: None,
        },
        bounds: Some(to_bounds(all)),
        capabilities: Vec::new(),
    };
    let hot: Vec<EntityBox> = shapes
        .iter()
        .enumerate()
        .filter(|(_, s)| !matches!(s, Shape::Spline(_)))
        .map(|(i, s)| EntityBox { id: eid(i), bounds: aabb(s.bounds()) })
        .collect();
    let classified = classify(&hot, STRADDLER_FACTOR).ok().filter(|c| !c.local.is_empty());
    let (local, long) = match &classified {
        Some(c) => (c.local.as_slice(), c.long.as_slice()),
        None => (hot.as_slice(), &[][..]),
    };
    let built = build_directory(local).expect("a directory");
    let mut cell_of: HashMap<u32, u32> =
        local.iter().zip(&built.cells).map(|(e, c)| (e.id.index(), c.get())).collect();
    if let Some(c) = classified.as_ref().filter(|c| !c.long.is_empty()) {
        let placeholder = |e: &EntityBox| LongEntity {
            id: e.id,
            bounds: e.bounds,
            payload: PayloadRef { block_id: BlockId(0), slot: 0 },
        };
        let first: Vec<LongEntity> = long.iter().map(placeholder).collect();
        for (id, cell) in payload_cells(&c.scale, &first).expect("payload cells") {
            assert!(cell >= STRADDLER_CELL_FIRST);
            cell_of.insert(id.index(), cell);
        }
    }
    let lay_out = |lists: Option<(u32, Vec<u8>)>| -> Result<Vec<u8>, String> {
        let mut w = ContainerWriter::new(config.clone());
        for (i, s) in shapes.iter().enumerate() {
            let cell = if matches!(s, Shape::Spline(_)) { 0 } else { cell_of[&(i as u32)] };
            push(&mut w, i, cell, s).map_err(|e| format!("entity {i} ({}): {e}", s.capability()))?;
        }
        w.set_spatial_directory(
            built.nodes,
            Bounds { min: built.bounds.min(), max: built.bounds.max() },
            built.bytes.clone(),
        )
        .expect("directory");
        if let Some((entries, bytes)) = lists {
            let world = long.iter().skip(1).fold(long[0].bounds, |u, e| {
                aabb([
                    u.min()[0].min(e.bounds.min()[0]),
                    u.min()[1].min(e.bounds.min()[1]),
                    u.max()[0].max(e.bounds.max()[0]),
                    u.max()[1].max(e.bounds.max()[1]),
                ])
            });
            w.set_straddler_lists(entries, Bounds { min: world.min(), max: world.max() }, bytes)
                .expect("lists");
        }
        w.finish().map_err(|e| e.to_string())
    };
    let Some(c) = classified.as_ref().filter(|c| !c.long.is_empty()) else {
        return lay_out(None);
    };
    let first = lay_out(None)?;
    let reader =
        ContainerReader::open(MemorySource::new(first), ReaderConfig::default()).expect("first pass");
    let slots: HashMap<u32, PayloadRef> =
        scan_payloads(&reader).expect("slots").into_iter().map(|s| (s.id.index(), s.payload)).collect();
    let placed: Vec<LongEntity> = long
        .iter()
        .map(|e| LongEntity { id: e.id, bounds: e.bounds, payload: slots[&e.id.index()] })
        .collect();
    let lists = build_straddlers(&c.scale, &placed).expect("lists");
    lay_out(Some((lists.entries, lists.bytes)))
}
