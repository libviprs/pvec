//! The entity mix of each corpus class. Every number is drawn from a splitmix64 stream and goes through basic
//! arithmetic and `round` only (no sine, cosine or square root), so the same seed gives the same bits on
//! arm64 and on x86_64. Angles are whole degrees times one constant for the same reason.

use crate::shape::{Shape, SplineData};
use std::f64::consts::TAU;

/// One degree in radians. A constant expression, so it is the same everywhere.
const DEG: f64 = TAU / 360.0;
/// Entities per room (or per site block) before the generator moves to the next one.
const PER_ROOM: usize = 400;

/// What the generator draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// Walls, doors, windows, furniture and hatch lines on a grid of rooms (millimetres).
    Architectural,
    /// Duct and pipe runs, fittings and elbows (millimetres).
    Mep,
    /// Pipe racks, vessels with elliptical heads, instruments and steel (millimetres).
    Industrial,
    /// A site plan in metres at UTM-scale offsets, millimetre detail.
    HugeCoordinate,
    /// Mostly B-splines of degree 2 to 7, some rational (millimetres).
    SplineHeavy,
    /// Near-degenerate arcs, repeated knots, 10^4-control-point splines, coincident points (millimetres).
    Pathological,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// A value in `lo..hi`, rounded to three decimals.
    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        r3(lo + self.unit() * (hi - lo))
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// An index drawn with the given weights.
    fn pick(&mut self, weights: &[u64]) -> usize {
        let mut at = self.below(weights.iter().sum());
        for (i, w) in weights.iter().enumerate() {
            if at < *w {
                return i;
            }
            at -= w;
        }
        weights.len() - 1
    }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::Line([r3(x0), r3(y0)], [r3(x1), r3(y1)])
}

fn circle(cx: f64, cy: f64, r: f64) -> Shape {
    Shape::Arc { centre: [r3(cx), r3(cy)], radius: r3(r), start_sweep: [0.0, TAU] }
}

fn arc(cx: f64, cy: f64, r: f64, start_deg: u64, sweep_deg: u64) -> Shape {
    Shape::Arc {
        centre: [r3(cx), r3(cy)],
        radius: r3(r),
        start_sweep: [start_deg as f64 * DEG, sweep_deg as f64 * DEG],
    }
}

fn ellipse(cx: f64, cy: f64, a: f64, b: f64, rot_deg: u64, start_deg: u64, sweep_deg: u64) -> Shape {
    let sweep = if sweep_deg == 360 { TAU } else { sweep_deg as f64 * DEG };
    Shape::Ellipse {
        centre: [r3(cx), r3(cy)],
        radii: [r3(a), r3(b)],
        angles: [rot_deg as f64 * DEG, start_deg as f64 * DEG, sweep],
    }
}

/// A clamped B-spline over `ncp_range.0` plus up to `ncp_range.1` control points that wander from `(x, y)` by up to `step` each, with uniform
/// interior knots.
fn spline(
    rng: &mut Rng,
    degree: usize,
    ncp_range: (usize, u64),
    x: f64,
    y: f64,
    step: f64,
    rational: bool,
) -> Shape {
    let ncp = ncp_range.0 + rng.below(ncp_range.1.max(1)) as usize;
    let (mut px, mut py) = (x, y);
    let (mut xs, mut ys) = (Vec::with_capacity(ncp), Vec::with_capacity(ncp));
    for _ in 0..ncp {
        xs.push(r3(px));
        ys.push(r3(py));
        px += rng.between(-step, step);
        py += rng.between(-step, step);
    }
    let spans = ncp - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|k| k as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    let weights = rational.then(|| (0..ncp).map(|_| rng.between(0.5, 2.0)).collect());
    Shape::Spline(Box::new(SplineData { degree, knots, x: xs, y: ys, weights }))
}

/// A closed rectangle as a five-vertex polyline.
fn rectangle(x: f64, y: f64, w: f64, h: f64) -> Shape {
    let p = |a: f64, b: f64| [r3(a), r3(b)];
    Shape::Polyline(vec![p(x, y), p(x + w, y), p(x + w, y + h), p(x, y + h), p(x, y)])
}

/// A Manhattan route of `legs_range.0` plus up to `legs_range.1` legs from `(x, y)`, each up to `reach` long.
fn route(rng: &mut Rng, x: f64, y: f64, legs_range: (u64, u64), reach: f64) -> Shape {
    let legs = legs_range.0 + rng.below(legs_range.1.max(1));
    let mut pts = vec![[r3(x), r3(y)]];
    let (mut px, mut py) = (x, y);
    for k in 0..legs {
        let d = rng.between(reach * 0.2, reach);
        let d = if rng.below(2) == 0 { d } else { -d };
        if k % 2 == 0 {
            px += d
        } else {
            py += d
        }
        pts.push([r3(px), r3(py)]);
    }
    Shape::Polyline(pts)
}

/// A frame of rooms: the grid, the room side, and the offset of its origin.
struct Frame {
    nx: usize,
    side: f64,
    origin: [f64; 2],
}

fn emit(model: Model, rng: &mut Rng, f: &Frame, room: usize, out: &mut Vec<Shape>) {
    let ox = f.origin[0] + (room % f.nx) as f64 * f.side;
    let oy = f.origin[1] + (room / f.nx) as f64 * f.side;
    let s = f.side;
    let (x, y) = (ox + rng.unit() * s * 0.9, oy + rng.unit() * s * 0.9);
    match model {
        Model::Architectural => match rng.pick(&[30, 10, 8, 4, 4, 3, 2, 4, 2, 1, 1]) {
            0 => {
                let l = rng.between(50.0, 600.0);
                let dy = if rng.below(2) == 0 { 0.0 } else { l };
                out.push(line(x, y, x + l, y + dy));
            }
            1 => out.push(rectangle(x, y, rng.between(400.0, 2400.0), rng.between(400.0, 2400.0))),
            2 => out.push(route(rng, x, y, (1, 4), s * 0.45)),
            3 => {
                let (r, start) = (rng.between(800.0, 1000.0), 90 * rng.below(4));
                out.push(arc(x, y, r, start, 90));
                out.push(line(x, y, x + r, y));
            }
            4 => {
                (0..3).for_each(|k| out.push(line(x, y + 100.0 * k as f64, x + 1200.0, y + 100.0 * k as f64)))
            }
            5 => out.push(circle(x, y, rng.between(150.0, 300.0))),
            6 => {
                out.push(arc(x, y, rng.between(1500.0, 4000.0), 30 * rng.below(12), 30 + 10 * rng.below(10)))
            }
            7 => {
                let l = rng.between(1000.0, 4000.0);
                out.push(line(x, y, x + l, y));
                out.push(line(x, y - 100.0, x, y + 100.0));
                out.push(line(x + l, y - 100.0, x + l, y + 100.0));
            }
            8 => (0..10)
                .for_each(|k| out.push(line(x, y + 280.0 * k as f64, x + 1100.0, y + 280.0 * k as f64))),
            9 => out.push(spline(rng, 3, (4, 3), x, y, 400.0, false)),
            _ => out.push(ellipse(
                x,
                y,
                rng.between(300.0, 600.0),
                rng.between(150.0, 300.0),
                15 * rng.below(12),
                0,
                360,
            )),
        },
        Model::Mep => match rng.pick(&[30, 30, 12, 10, 4, 4, 3, 7]) {
            0 => out.push(route(rng, x, y, (2, 6), s * 0.5)),
            1 => {
                let l = rng.between(200.0, 3000.0);
                let vertical = rng.below(2) == 0;
                out.push(if vertical { line(x, y, x, y + l) } else { line(x, y, x + l, y) });
            }
            2 => out.push(circle(x, y, rng.between(40.0, 150.0))),
            3 => out.push(arc(x, y, rng.between(150.0, 600.0), 90 * rng.below(4), 90)),
            4 => out.push(spline(rng, 3, (5, 5), x, y, 500.0, false)),
            5 => {
                out.push(circle(x, y, rng.between(100.0, 250.0)));
                out.push(circle(x, y, rng.between(40.0, 90.0)));
            }
            6 => out.push(ellipse(
                x,
                y,
                rng.between(200.0, 500.0),
                rng.between(80.0, 200.0),
                90 * rng.below(2),
                0,
                360,
            )),
            _ => {
                (0..4).for_each(|k| out.push(line(x + 300.0 * k as f64, y, x + 300.0 * k as f64, y + 150.0)))
            }
        },
        Model::Industrial => match rng.pick(&[18, 30, 6, 12, 12, 4, 4, 10]) {
            0 => out.push(route(rng, x, y, (1, 5), s * 0.6)),
            1 => {
                let l = rng.between(300.0, 6000.0);
                out.push(if rng.below(2) == 0 { line(x, y, x + l, y) } else { line(x, y, x, y + l) });
            }
            2 => {
                let r = rng.between(500.0, 2500.0);
                out.push(circle(x, y, r));
                out.push(ellipse(x, y + r, r, r * 0.25, 0, 0, 180));
                out.push(ellipse(x, y - r, r, r * 0.25, 0, 180, 180));
            }
            3 => {
                out.push(circle(x, y, rng.between(150.0, 300.0)));
                out.push(line(x, y, x + 600.0, y + 600.0));
            }
            4 => out.push(arc(x, y, rng.between(200.0, 900.0), 90 * rng.below(4), 90)),
            5 => out.push(ellipse(
                x,
                y,
                rng.between(300.0, 800.0),
                rng.between(100.0, 300.0),
                15 * rng.below(12),
                0,
                360,
            )),
            6 => out.push(spline(rng, 3, (6, 8), x, y, 700.0, false)),
            _ => out.push(rectangle(x, y, rng.between(1000.0, 5000.0), rng.between(1000.0, 3000.0))),
        },
        Model::HugeCoordinate => match rng.pick(&[35, 20, 15, 15, 8, 4, 3]) {
            0 => out.push(line(x, y, x + rng.between(0.5, 40.0), y + rng.between(0.0, 40.0))),
            1 => {
                let n = 5 + rng.below(8);
                let mut pts: Vec<[f64; 2]> =
                    (0..n).map(|_| [r3(x + rng.unit() * 60.0), r3(y + rng.unit() * 60.0)]).collect();
                pts.push(pts[0]);
                out.push(Shape::Polyline(pts));
            }
            2 => out.push(circle(x, y, rng.between(0.5, 3.0))),
            3 => out.push(arc(x, y, rng.between(5.0, 80.0), 30 * rng.below(12), 20 + 10 * rng.below(8))),
            4 => out.push(spline(rng, 3, (8, 13), x, y, 15.0, false)),
            5 => out.push(ellipse(
                x,
                y,
                rng.between(2.0, 20.0),
                rng.between(1.0, 10.0),
                15 * rng.below(12),
                0,
                360,
            )),
            _ => out.push(line(x, y, x + 0.003, y + 0.002)),
        },
        Model::SplineHeavy => match rng.pick(&[60, 15, 10, 10, 5]) {
            0 => {
                let degree = [3, 3, 3, 3, 3, 5, 5, 7, 2, 4][rng.below(10) as usize];
                let rational = rng.below(100) < 15;
                out.push(spline(rng, degree, (degree + 1, 20), x, y, 300.0, rational));
            }
            1 => {
                let l = rng.between(100.0, 2000.0);
                out.push(line(x, y, x + l, y + l));
            }
            2 => out.push(arc(x, y, rng.between(100.0, 2000.0), 30 * rng.below(12), 45)),
            3 => out.push(route(rng, x, y, (2, 4), s * 0.3)),
            _ => out.push(ellipse(
                x,
                y,
                rng.between(200.0, 800.0),
                rng.between(100.0, 400.0),
                30 * rng.below(6),
                0,
                360,
            )),
        },
        Model::Pathological => unreachable!("the pathological class is not drawn room by room"),
    }
}

/// Long runs across the whole drawing (structural axes, mains, pipe racks), a few per row and column.
fn axes(model: Model, rng: &mut Rng, f: &Frame, rows: usize, out: &mut Vec<Shape>) {
    let (w, h) = (f.nx as f64 * f.side, rows as f64 * f.side);
    let [ox, oy] = f.origin;
    let per_band = match model {
        Model::Architectural | Model::HugeCoordinate => 1,
        Model::Mep | Model::Industrial => 3,
        _ => 0,
    };
    for r in 0..rows {
        for _ in 0..per_band {
            let y = oy + (r as f64 + rng.unit()) * f.side;
            out.push(line(ox, y, ox + w, y));
        }
    }
    for c in 0..f.nx {
        for _ in 0..per_band {
            let x = ox + (c as f64 + rng.unit()) * f.side;
            out.push(line(x, oy, x, oy + h));
        }
    }
}

/// The pathological class: near-degenerate arcs, repeated knots, coincident points and a few splines of 10^4
/// control points. Nothing is rounded, so the awkward values survive.
fn pathological(n: usize, rng: &mut Rng) -> Vec<Shape> {
    let mut out = Vec::with_capacity(n);
    for big in [3, 3, 7, 3] {
        out.push(spline(rng, big, (10_000, 0), 0.0, 0.0, 1.0, big == 7));
    }
    let mut k = 0u64;
    while out.len() < n {
        let (x, y) = (rng.unit() * 100_000.0, rng.unit() * 100_000.0);
        match k % 9 {
            0 => out.push(Shape::Arc {
                centre: [x, y],
                radius: 1_000_000.0,
                start_sweep: [rng.unit(), 0.000_000_001],
            }),
            1 => out.push(Shape::Arc { centre: [x, y], radius: 0.000_001, start_sweep: [0.0, TAU] }),
            2 => out.push(Shape::Arc {
                centre: [x, y],
                radius: 50.0,
                start_sweep: [0.0, TAU - 0.000_000_000_001],
            }),
            3 => {
                // a cubic whose interior knot has multiplicity 3, so the curve is only C0 there
                let xs: Vec<f64> = (0..7).map(|i| x + 10.0 * i as f64).collect();
                let ys: Vec<f64> = (0..7).map(|i| y + if i % 2 == 0 { 0.0 } else { 7.0 }).collect();
                let knots = vec![0.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.5, 1.0, 1.0, 1.0, 1.0];
                out.push(Shape::Spline(Box::new(SplineData {
                    degree: 3,
                    knots,
                    x: xs,
                    y: ys,
                    weights: None,
                })));
            }
            4 => {
                // coincident control points
                let xs = vec![x, x, x, x + 5.0, x + 5.0, x + 9.0];
                let ys = vec![y, y, y, y + 5.0, y + 5.0, y];
                let knots = vec![0.0, 0.0, 0.0, 0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0, 1.0, 1.0, 1.0];
                out.push(Shape::Spline(Box::new(SplineData {
                    degree: 3,
                    knots,
                    x: xs,
                    y: ys,
                    weights: None,
                })));
            }
            5 => out.push(Shape::Polyline(vec![[x, y], [x + 1.0, y], [x, y], [x, y + 1.0], [x + 1.0, y]])),
            6 => out.push(Shape::Line([x, y], [x + 0.000_000_001, y - 0.000_000_001])),
            7 => {
                let l = Shape::Line([x, y], [x + 3.0, y + 4.0]);
                out.push(l.clone());
                out.push(l);
            }
            _ => {
                // three segments through one point, the shared endpoint written three times
                out.extend([
                    Shape::Line([x, y], [x + 2.0, y]),
                    Shape::Line([x, y], [x, y + 2.0]),
                    Shape::Line([x, y], [x - 2.0, y - 2.0]),
                ]);
            }
        }
        k += 1;
    }
    out.truncate(n);
    out
}

/// The shapes of one generated entry: `n` of them, from `seed`.
pub fn shapes(model: Model, n: usize, seed: u64) -> Vec<Shape> {
    let mut rng = Rng(seed);
    if model == Model::Pathological {
        return pathological(n, &mut rng);
    }
    let rooms = n.div_ceil(PER_ROOM);
    let nx = (1..).find(|k| k * k >= rooms).unwrap_or(1);
    let (side, origin) = match model {
        Model::HugeCoordinate => (100.0, [6_480_000.125, 5_900_000.25]),
        _ => (8000.0, [0.0, 0.0]),
    };
    let frame = Frame { nx, side, origin };
    let mut out = Vec::with_capacity(n);
    axes(model, &mut rng, &frame, rooms.div_ceil(nx), &mut out);
    out.truncate(n / 10);
    let mut room = 0;
    while out.len() < n {
        for _ in 0..PER_ROOM {
            if out.len() >= n {
                break;
            }
            emit(model, &mut rng, &frame, room, &mut out);
        }
        room += 1;
    }
    out.truncate(n);
    out
}
