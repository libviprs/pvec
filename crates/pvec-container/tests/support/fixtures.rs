//! Deterministic fixtures. Every coordinate is an integer divided by a constant, so it is the same
//! `f64` on every machine: nothing here calls a transcendental function.

use pvec_capabilities::{CapabilityId, Version};
use pvec_components::Stream;
use pvec_container::blocks::advanced::SplineSpec;
use pvec_container::writer::{ContainerWriter, EntityMeta, SortKey, WriteError, WriterConfig};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;

/// A small xorshift generator, so a shuffle depends on the seed and nothing else.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

/// One entity of a fixture, owning its numbers.
#[derive(Clone, Debug)]
pub enum Ent {
    Line {
        id: u32,
        style: u32,
        cell: u32,
        a: [f64; 2],
        b: [f64; 2],
    },
    Poly {
        id: u32,
        style: u32,
        cell: u32,
        pts: Vec<[f64; 2]>,
    },
    Arc {
        id: u32,
        style: u32,
        cell: u32,
        centre: [f64; 2],
        radius: f64,
        ss: [f64; 2],
    },
    Ellipse {
        id: u32,
        style: u32,
        cell: u32,
        centre: [f64; 2],
        radii: [f64; 2],
        angles: [f64; 3],
    },
    Spline {
        id: u32,
        style: u32,
        degree: usize,
        knots: Vec<f64>,
        x: Vec<f64>,
        y: Vec<f64>,
        weights: Option<Vec<f64>>,
    },
    Composite {
        id: u32,
        style: u32,
        segments: Vec<u32>,
        trims: Option<Vec<[f64; 2]>>,
        closed: bool,
    },
}

/// Something a caller hands the writer.
#[derive(Clone, Debug)]
pub enum Item {
    Ent(Ent),
    Stream(Stream),
}

pub fn union(a: Bounds, b: Bounds) -> Bounds {
    Bounds {
        min: std::array::from_fn(|k| a.min[k].min(b.min[k])),
        max: std::array::from_fn(|k| a.max[k].max(b.max[k])),
    }
}

fn boxed(points: &[[f64; 2]]) -> Bounds {
    points
        .iter()
        .fold(Bounds { min: points[0], max: points[0] }, |b, p| union(b, Bounds { min: *p, max: *p }))
}

fn around(c: [f64; 2], r: f64) -> Bounds {
    Bounds { min: [c[0] - r, c[1] - r], max: [c[0] + r, c[1] + r] }
}

pub fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

impl Ent {
    pub fn id(&self) -> u32 {
        match self {
            Ent::Line { id, .. }
            | Ent::Poly { id, .. }
            | Ent::Arc { id, .. }
            | Ent::Ellipse { id, .. }
            | Ent::Spline { id, .. }
            | Ent::Composite { id, .. } => *id,
        }
    }

    pub fn bounds(&self) -> Bounds {
        match self {
            Ent::Line { a, b, .. } => boxed(&[*a, *b]),
            Ent::Poly { pts, .. } => boxed(pts),
            Ent::Arc { centre, radius, .. } => around(*centre, *radius),
            Ent::Ellipse { centre, radii, .. } => around(*centre, radii[0].max(radii[1])),
            Ent::Spline { x, y, .. } => {
                let pts: Vec<[f64; 2]> = x.iter().zip(y).map(|(a, b)| [*a, *b]).collect();
                boxed(&pts)
            }
            Ent::Composite { .. } => Bounds { min: [0.0, 0.0], max: [1.0, 1.0] },
        }
    }

    pub fn push(&self, w: &mut ContainerWriter) -> Result<(), WriteError> {
        let b = self.bounds();
        match self {
            Ent::Line { id, style, cell, a, b: e } => {
                w.push_line(EntityMeta { id: eid(*id), style: *style, cell: *cell, bounds: b }, *a, *e)
            }
            Ent::Poly { id, style, cell, pts } => {
                w.push_polyline(EntityMeta { id: eid(*id), style: *style, cell: *cell, bounds: b }, pts)
            }
            Ent::Arc { id, style, cell, centre, radius, ss } => w.push_circle_arc(
                EntityMeta { id: eid(*id), style: *style, cell: *cell, bounds: b },
                *centre,
                *radius,
                *ss,
            ),
            Ent::Ellipse { id, style, cell, centre, radii, angles } => w.push_ellipse_arc(
                EntityMeta { id: eid(*id), style: *style, cell: *cell, bounds: b },
                *centre,
                *radii,
                *angles,
            ),
            Ent::Spline { id, style, degree, knots, x, y, weights } => w.push_spline(
                EntityMeta { id: eid(*id), style: *style, cell: 0, bounds: b },
                SplineSpec {
                    degree: *degree,
                    knots,
                    x,
                    y,
                    weights: weights.as_deref(),
                    closed: false,
                    periodic: false,
                },
            ),
            Ent::Composite { id, style, segments, trims, closed } => {
                let segs: Vec<EntityId> = segments.iter().map(|s| eid(*s)).collect();
                w.push_composite(
                    EntityMeta { id: eid(*id), style: *style, cell: 0, bounds: b },
                    &segs,
                    trims.as_deref(),
                    *closed,
                )
            }
        }
    }
}

/// Everything a writer is given.
#[derive(Clone, Debug)]
pub struct Input {
    pub name: &'static str,
    pub config: WriterConfig,
    pub items: Vec<Item>,
}

pub fn config(caps: &[u32], bounds: Option<Bounds>) -> WriterConfig {
    WriterConfig {
        unit: LengthUnit::Millimetre,
        metadata: Metadata {
            producer_id: "pvec-test-writer".to_string(),
            producer_version: "0.0.0".to_string(),
            corpus_revision: Some("pvec-p1-fixtures-r1".to_string()),
        },
        bounds,
        capabilities: caps.iter().map(|c| CapabilityId::new(*c)).collect(),
    }
}

impl Input {
    /// Pushes everything in the order held, with the given sorts switched off.
    pub fn load(&self, skip: &[SortKey]) -> Result<ContainerWriter, WriteError> {
        let mut w = ContainerWriter::new(self.config.clone());
        for k in skip {
            w.skip_sort(*k);
        }
        for item in &self.items {
            match item {
                Item::Ent(e) => e.push(&mut w)?,
                Item::Stream(s) => w.push_stream(s.clone())?,
            }
        }
        Ok(w)
    }

    pub fn write(&self) -> Result<Vec<u8>, WriteError> {
        self.load(&[])?.finish()
    }

    pub fn write_skipping(&self, key: SortKey) -> Result<Vec<u8>, WriteError> {
        self.load(&[key])?.finish()
    }

    /// The same input with the entities, the streams and the capability list in another order.
    pub fn shuffled(&self, seed: u64) -> Input {
        let mut rng = Rng(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let mut out = self.clone();
        rng.shuffle(&mut out.items);
        rng.shuffle(&mut out.config.capabilities);
        out
    }

    pub fn entities(&self) -> impl Iterator<Item = &Ent> {
        self.items.iter().filter_map(|i| if let Item::Ent(e) = i { Some(e) } else { None })
    }

    pub fn streams(&self) -> impl Iterator<Item = &Stream> {
        self.items.iter().filter_map(|i| if let Item::Stream(s) = i { Some(s) } else { None })
    }
}

fn frac(n: u64, d: f64) -> f64 {
    n as f64 / d
}

fn hot(i: u32, cells: u32) -> Ent {
    let (id, style, cell) = (i, i % 5, i % cells);
    let u = u64::from(i);
    let p = |k: u64| [frac((u * 37 + k * 11) % 1000, 8.0), frac((u * 91 + k * 5) % 997, 1000.0)];
    match i % 4 {
        0 => Ent::Line { id, style, cell, a: p(0), b: p(1) },
        1 => Ent::Poly { id, style, cell, pts: (0..2 + u % 5).map(p).collect() },
        2 => Ent::Arc {
            id,
            style,
            cell,
            centre: p(0),
            radius: 1.0 + frac(u % 13, 4.0),
            ss: [frac(u % 7, 8.0), 1.5],
        },
        _ => Ent::Ellipse {
            id,
            style,
            cell,
            centre: p(0),
            radii: [2.0 + frac(u % 5, 2.0), 1.0 + frac(u % 3, 4.0)],
            angles: [frac(u % 9, 8.0), 0.25, -1.75],
        },
    }
}

fn clamped_knots(degree: usize, n: usize) -> Vec<f64> {
    let mut k = vec![0.0; degree + 1];
    k.extend((1..=n - degree - 1).map(|v| v as f64));
    k.extend(std::iter::repeat_n((n - degree) as f64, degree + 1));
    k
}

fn spline(id: u32) -> Ent {
    let i = u64::from(id);
    let degree = 1 + (i % 5) as usize;
    let n = degree + 1 + (i % 6) as usize;
    let x: Vec<f64> = (0..n as u64).map(|j| frac((i * 13 + j * 7) % 211, 4.0)).collect();
    let y: Vec<f64> = (0..n as u64).map(|j| frac((i * 17 + j * 9) % 199, 4.0)).collect();
    let weights = (i % 3 == 0).then(|| (0..n as u64).map(|j| 1.0 + frac((i + j) % 4, 4.0)).collect());
    Ent::Spline { id, style: (i % 4) as u32, degree, knots: clamped_knots(degree, n), x, y, weights }
}

fn stream(cap: u32, seed: u64, len: usize) -> Stream {
    let mut rng = Rng(seed | 1);
    Stream {
        capability: CapabilityId::new(cap),
        version: Version::new(1, 0, 0),
        payload: (0..len).map(|_| (rng.next() >> 24) as u8).collect(),
    }
}

/// A plan-like drawing: lines, polylines and arcs over seven cells.
pub fn plan() -> Input {
    Input {
        name: "plan",
        config: config(&[4, 5, 6, 7, 1], None),
        items: (0..480).map(|i| Item::Ent(hot(i, 7))).collect(),
    }
}

/// Splines with shared knot vectors, rational ones, and composites that nest three deep, some of
/// them with a lower id than the curves they name.
pub fn spline_heavy() -> Input {
    let mut items: Vec<Item> = Vec::new();
    for i in 0..10 {
        items.push(Item::Ent(Ent::Line {
            id: i,
            style: 1,
            cell: 0,
            a: [f64::from(i), 0.0],
            b: [f64::from(i), 10.5],
        }));
    }
    for i in 10..40u32 {
        let k = (i - 10) * 2;
        items.push(Item::Ent(Ent::Composite {
            id: i,
            style: 2,
            segments: vec![40 + k % 40, 40 + (k + 1) % 40, 80 + k],
            trims: None,
            closed: false,
        }));
    }
    for i in 40..80u32 {
        let k = i - 40;
        let trims = (k % 2 == 0).then(|| vec![[0.0, 1.0]; 3]);
        items.push(Item::Ent(Ent::Composite {
            id: i,
            style: 3,
            segments: vec![80 + k, 120 + k, 160 + (k * 3) % 200],
            trims,
            closed: k % 5 == 0,
        }));
    }
    for i in 80..380 {
        items.push(Item::Ent(spline(i)));
    }
    for i in 380..400 {
        items.push(Item::Ent(hot(i, 3)));
    }
    Input { name: "spline-heavy", config: config(&[11, 12, 9, 8, 4], None), items }
}

/// Every geometry type, several cells, advanced curves and component streams in one file.
pub fn mixed() -> Input {
    let mut items: Vec<Item> = (0..120).map(|i| Item::Ent(hot(i, 4))).collect();
    for i in 120..140 {
        items.push(Item::Ent(spline(i)));
    }
    items.push(Item::Ent(Ent::Composite {
        id: 140,
        style: 0,
        segments: vec![120, 121, 122],
        trims: None,
        closed: false,
    }));
    items.push(Item::Ent(Ent::Composite {
        id: 141,
        style: 0,
        segments: vec![140, 123],
        trims: None,
        closed: false,
    }));
    for (cap, len) in [(14, 40), (15, 24), (16, 96), (18, 64), (20, 8)] {
        items.push(Item::Stream(stream(cap, u64::from(cap) * 7919, len)));
    }
    let bounds = Bounds { min: [0.0, 0.0], max: [512.0, 512.0] };
    Input { name: "mixed", config: config(&[1, 3, 4, 5, 6, 7, 9, 11, 12, 13], Some(bounds)), items }
}

/// One entity, nothing else.
pub fn minimal() -> Input {
    Input {
        name: "minimal",
        config: config(&[], None),
        items: vec![Item::Ent(Ent::Line { id: 0, style: 0, cell: 0, a: [0.0, 0.0], b: [3.0, 4.0] })],
    }
}

pub fn fixtures() -> Vec<Input> {
    vec![plan(), spline_heavy(), mixed(), minimal()]
}
