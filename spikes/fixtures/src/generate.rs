//! The four generators. Same seed, same bytes, on every platform.

use crate::{Drawing, Entity, Geom, Pt};
use core::f64::consts::{PI, TAU};

/// splitmix64: tiny, well mixed, and defined by its code rather than by a library version.
pub struct Rng(pub u64);

impl Rng {
    /// Next 64 random bits.
    pub fn u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    pub fn f(&mut self) -> f64 {
        (self.u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Uniform in `[lo, hi)`.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f()
    }
    /// Uniform integer in `[lo, hi]`.
    pub fn int(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.u64() % (hi - lo + 1) as u64) as usize
    }
}

/// Relative weights of each kind in a room.
#[derive(Clone, Copy)]
pub struct Mix {
    pub line: f64,
    pub polyline: f64,
    pub arc: f64,
    pub circle: f64,
    pub ellipse: f64,
    pub spline: f64,
    pub composite: f64,
    /// Spline degree range and control point count range.
    pub degree: (usize, usize),
    pub cps: (usize, usize),
}

/// Plan-like drawings: mostly short axis-aligned lines.
pub const PLAN: Mix = Mix {
    line: 60.0,
    polyline: 20.0,
    arc: 8.0,
    circle: 4.0,
    ellipse: 3.0,
    spline: 3.0,
    composite: 2.0,
    degree: (3, 3),
    cps: (4, 10),
};
/// Spline-heavy drawings (site plans, contour-like linework).
pub const SPLINEY: Mix = Mix {
    line: 25.0,
    polyline: 10.0,
    arc: 3.0,
    circle: 1.0,
    ellipse: 1.0,
    spline: 58.0,
    composite: 2.0,
    degree: (2, 5),
    cps: (6, 60),
};

fn q(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn qp(p: Pt) -> Pt {
    [q(p[0]), q(p[1])]
}

fn spline(rng: &mut Rng, o: Pt, s: f64, mix: &Mix) -> Geom {
    let degree = rng.int(mix.degree.0, mix.degree.1);
    let n = rng.int(mix.cps.0.max(degree + 1), mix.cps.1.max(degree + 1));
    let step = s * 0.012;
    let mut p = o;
    let mut dir = rng.range(0.0, TAU);
    let mut cps = Vec::with_capacity(n);
    for _ in 0..n {
        cps.push(qp(p));
        dir += rng.range(-0.6, 0.6);
        let l = step * rng.range(0.4, 1.6);
        p = [p[0] + l * dir.cos(), p[1] + l * dir.sin()];
    }
    let interior = n - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    let mut last = 0.0;
    for j in 1..=interior {
        let base = j as f64 / (interior + 1) as f64;
        let jitter = 0.3 / (interior + 1) as f64 * (rng.f() - 0.5);
        last = (base + jitter).max(last + 1.0 / (4.0 * (interior + 1) as f64));
        knots.push(last.min(0.999));
    }
    knots.extend(vec![1.0; degree + 1]);
    let weights = (rng.f() < 0.25).then(|| (0..n).map(|_| rng.range(0.5, 2.0)).collect());
    Geom::Spline { degree: degree as u8, cps, knots, weights }
}

fn one(rng: &mut Rng, ox: f64, oy: f64, s: f64, mix: &Mix) -> Geom {
    let total = mix.line + mix.polyline + mix.arc + mix.circle + mix.ellipse + mix.spline + mix.composite;
    let mut pick = rng.f() * total;
    let o = qp([ox + rng.f() * s, oy + rng.f() * s]);
    let mut take = |w: f64| {
        let hit = pick < w;
        pick -= w;
        hit
    };
    if take(mix.line) {
        let len = s * 0.02 * (1.0 + 20.0 * rng.f().powi(3));
        let ang = if rng.f() < 0.7 { (rng.int(0, 3) as f64) * PI / 2.0 } else { rng.range(0.0, TAU) };
        Geom::Line { a: o, b: qp([o[0] + len * ang.cos(), o[1] + len * ang.sin()]) }
    } else if take(mix.polyline) {
        let n = rng.int(3, 12);
        let mut p = o;
        let mut pts = vec![p];
        let mut ang = rng.range(0.0, TAU);
        for _ in 1..n {
            ang += if rng.f() < 0.6 {
                (rng.int(0, 1) as f64 * 2.0 - 1.0) * PI / 2.0
            } else {
                rng.range(-1.0, 1.0)
            };
            let l = s * 0.01 * rng.range(0.3, 2.0);
            p = qp([p[0] + l * ang.cos(), p[1] + l * ang.sin()]);
            pts.push(p);
        }
        Geom::Polyline { pts }
    } else if take(mix.arc) {
        let a0 = rng.range(0.0, TAU);
        Geom::Arc { c: o, r: q(s * 0.005 * (1.0 + 10.0 * rng.f())), a0, a1: a0 + rng.range(0.3, PI * 1.5) }
    } else if take(mix.circle) {
        Geom::Circle { c: o, r: q(s * 0.004 * (1.0 + 8.0 * rng.f())) }
    } else if take(mix.ellipse) {
        let a = q(s * 0.006 * (1.0 + 6.0 * rng.f()));
        let t0 = if rng.f() < 0.5 { 0.0 } else { rng.range(0.0, PI) };
        Geom::Ellipse {
            c: o,
            a,
            b: q(a * rng.range(0.2, 0.9)),
            rot: rng.range(0.0, PI),
            t0,
            t1: t0 + if t0 == 0.0 { TAU } else { rng.range(0.5, PI) },
        }
    } else if take(mix.spline) {
        spline(rng, o, s, mix)
    } else {
        // Composite: line, arc, line, arc ... each part starts where the last one ended.
        let n = rng.int(3, 6);
        let mut p = o;
        let mut parts = Vec::new();
        let mut ang = rng.range(0.0, TAU);
        for i in 0..n {
            if i % 2 == 0 {
                let l = s * 0.01 * rng.range(0.5, 2.0);
                let e = [p[0] + l * ang.cos(), p[1] + l * ang.sin()];
                parts.push(Geom::Line { a: p, b: e });
                p = e;
            } else {
                let r = s * 0.004 * rng.range(0.5, 2.0);
                let a0 = ang - PI / 2.0;
                let sweep = rng.range(0.4, 1.4);
                let c = [p[0] - r * a0.cos(), p[1] - r * a0.sin()];
                parts.push(Geom::Arc { c, r, a0, a1: a0 + sweep });
                p = [c[0] + r * (a0 + sweep).cos(), c[1] + r * (a0 + sweep).sin()];
                ang += sweep;
            }
        }
        Geom::Composite { parts }
    }
}

fn rooms(name: &'static str, seed: u64, grid: usize, pitch: f64, total: usize, mix: &Mix) -> Drawing {
    let mut rng = Rng(seed);
    let n_rooms = grid * grid;
    let mut entities = Vec::with_capacity(total);
    for r in 0..n_rooms {
        let (gx, gy) = (r % grid, r / grid);
        // Spread the remainder over the first rooms so the total is exact.
        let n = total / n_rooms + usize::from(r < total % n_rooms);
        for _ in 0..n {
            let g = one(&mut rng, gx as f64 * pitch, gy as f64 * pitch, pitch, mix);
            entities.push(Entity { id: entities.len() as u32, geom: g });
        }
    }
    let side = grid as f64 * pitch;
    Drawing { name, extent: [0.0, 0.0, side, side], entities }
}

/// F1: 1,000 mixed entities in 3 x 3 rooms.
pub fn small_synthetic() -> Drawing {
    rooms("small-synthetic", 0x51a11, 3, 2000.0, 1_000, &PLAN)
}

/// F2: 50,000 entities, 58% splines of degree 2 to 5 with 6 to 60 control points.
pub fn spline_heavy() -> Drawing {
    rooms("spline-heavy", 0x5b11e, 10, 2000.0, 50_000, &SPLINEY)
}

/// F3: 1,000,000 entities in 50 x 50 rooms, a plan-like mix.
pub fn million() -> Drawing {
    rooms("million", 0x1_000_000, 50, 2000.0, 1_000_000, &PLAN)
}

/// F4: 100,000 entities plus four that span the whole extent: a full-width line, a full-height
/// line, a diagonal, and a circle that circumscribes the drawing's centre square. For P1.6.
pub fn straddler() -> Drawing {
    let mut d = rooms("straddler", 0x57_4a_dd, 16, 2000.0, 99_996, &PLAN);
    let w = d.extent[2];
    let long = [
        Geom::Line { a: [0.0, w / 2.0], b: [w, w / 2.0] },
        Geom::Line { a: [w / 2.0, 0.0], b: [w / 2.0, w] },
        Geom::Line { a: [0.0, 0.0], b: [w, w] },
        Geom::Circle { c: [w / 2.0, w / 2.0], r: w / 2.0 },
    ];
    for g in long {
        d.entities.push(Entity { id: d.entities.len() as u32, geom: g });
    }
    d
}

/// All four, in the order used by every table.
pub fn all() -> Vec<Drawing> {
    vec![small_synthetic(), spline_heavy(), million(), straddler()]
}
