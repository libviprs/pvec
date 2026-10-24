//! Fixtures shared by the issue 131 tests: the seeded generators of the P5.2 tests, copied so those
//! tests stay as they were.
#![allow(dead_code)]

use pvec_entity::EntityId;
use pvec_stream::Aabb;

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

pub fn bx(a: f64, b: f64, c: f64, d: f64) -> Aabb {
    Aabb::new([a, b], [c, d]).unwrap()
}

pub fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

pub fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// One entity of a fixture: its control points (two make a line, more make a polyline).
#[derive(Clone)]
pub struct Fx {
    pub id: EntityId,
    pub points: Vec<[f64; 2]>,
}

impl Fx {
    pub fn bounds(&self) -> Aabb {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for p in &self.points {
            for k in 0..2 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Aabb::new(lo, hi).unwrap()
    }
}

/// A grid of `side` x `side` rooms of `per_room` plan-like lines each, one in twenty a wall as long as its
/// room: the shape of P1.6's `million` at a size a debug build can write.
pub fn rooms(seed: u64, side: u32, per_room: u32, room: f64) -> Vec<Fx> {
    let mut rng = Rng(seed);
    let mut out = Vec::new();
    for ry in 0..side {
        for rx in 0..side {
            for _ in 0..per_room {
                let (ox, oy) = (f64::from(rx) * room, f64::from(ry) * room);
                let wall = rng.next() % 20 == 0;
                let (w, h) = if wall {
                    if rng.next() % 2 == 0 { (room * 0.9, 0.0) } else { (0.0, room * 0.9) }
                } else {
                    (rng.unit() * room * 0.04, rng.unit() * room * 0.04)
                };
                let x = ox + rng.unit() * (room - w);
                let y = oy + rng.unit() * (room - h);
                let id = eid(out.len() as u32);
                out.push(Fx { id, points: vec![[r3(x), r3(y)], [r3(x + w), r3(y + h)]] });
            }
        }
    }
    out
}

/// `count` polylines of five random control points each, every one reaching across `span` of a drawing
/// `extent` wide: the long entities of P1.6's `stress-long`.
pub fn long_polylines(rng: &mut Rng, first_id: u32, count: u32, extent: f64, span: f64) -> Vec<Fx> {
    (0..count)
        .map(|i| {
            let x0 = rng.unit() * (extent - span);
            let y0 = rng.unit() * (extent - span);
            let points =
                (0..5).map(|k| [r3(x0 + span * f64::from(k) / 4.0), r3(y0 + rng.unit() * span)]).collect();
            Fx { id: eid(first_id + i), points }
        })
        .collect()
}

pub fn stress_long() -> Vec<Fx> {
    let mut v = rooms(11, 6, 300, 2000.0);
    let mut rng = Rng(12);
    let n = v.len() as u32;
    v.extend(long_polylines(&mut rng, n, 24, 12_000.0, 9_000.0));
    v
}

pub fn identical_boxes() -> Vec<Fx> {
    (0..640).map(|i| Fx { id: eid(i), points: vec![[10.0, 10.0], [500.0, 400.0]] }).collect()
}

/// 575 small lines in a 3 x 3 grid of clusters and one long polyline across all of them: 576 entities, so
/// exactly nine cells of 64, and the polyline's bounds are the drawing's.
pub fn nine_cells() -> Vec<Fx> {
    let mut rng = Rng(5);
    let mut v = Vec::new();
    for i in 0..575u32 {
        let (cx, cy) = (f64::from(i % 3) * 3000.0 + 500.0, f64::from((i / 3) % 3) * 3000.0 + 500.0);
        let (x, y) = (cx + rng.unit() * 900.0, cy + rng.unit() * 900.0);
        v.push(Fx { id: eid(i), points: vec![[r3(x), r3(y)], [r3(x + 20.0), r3(y + 5.0)]] });
    }
    let points = vec![[100.0, 100.0], [2500.0, 4100.0], [4500.0, 2600.0], [6200.0, 6900.0], [8900.0, 8900.0]];
    v.push(Fx { id: eid(575), points });
    v
}
