//! Deterministic drawings shared by every phase 1 spike, so P1.4, P1.5, P1.6 and P1.7 read the same
//! data and their tables can be compared line by line.
//!
//! Corpus revision: [`REVISION`]. Any change to a generator changes the content hash printed by
//! `fixture-info`, and a table is only comparable with another if the hashes agree.
//!
//! The geometry here is plain data. It is not the P1.3 API and carries no layer, style or text:
//! those are independent components and outside the geometry oracle.

pub mod generate;
pub mod p21;

/// A 2D point.
pub type Pt = [f64; 2];

/// Version of the generators. Bump it when any fixture's content changes.
pub const REVISION: &str = "pvec-p1-fixtures-r1";

/// One piece of geometry, in the vocabulary of `docs/ISO_PROFILE.md`.
#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    /// A bounded segment.
    Line { a: Pt, b: Pt },
    /// A full circle.
    Circle { c: Pt, r: f64 },
    /// A counter-clockwise arc from angle `a0` to `a1` (radians, `a0 < a1 <= a0 + 2 pi`).
    Arc { c: Pt, r: f64, a0: f64, a1: f64 },
    /// An elliptic arc: semi-axes `a` (along the direction `rot`) and `b`, eccentric angles `t0..t1`.
    Ellipse { c: Pt, a: f64, b: f64, rot: f64, t0: f64, t1: f64 },
    /// A polyline of at least two vertices.
    Polyline { pts: Vec<Pt> },
    /// A (possibly rational) clamped B-spline.
    Spline { degree: u8, cps: Vec<Pt>, knots: Vec<f64>, weights: Option<Vec<f64>> },
    /// Joined segments (lines and arcs only in these fixtures).
    Composite { parts: Vec<Geom> },
}

/// One entity: a file-local id and its geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    /// File-local id, equal to the entity's index.
    pub id: u32,
    /// The geometry.
    pub geom: Geom,
}

/// A named drawing.
#[derive(Clone, Debug)]
pub struct Drawing {
    /// Fixture name used in every table.
    pub name: &'static str,
    /// `[min_x, min_y, max_x, max_y]` of the intended extent (the drawing may not touch every edge).
    pub extent: [f64; 4],
    /// Entities, `entities[i].id == i`.
    pub entities: Vec<Entity>,
}

impl Geom {
    /// A small integer naming the kind, used by every layout and by the checksum.
    pub fn tag(&self) -> u8 {
        match self {
            Geom::Line { .. } => 0,
            Geom::Circle { .. } => 1,
            Geom::Arc { .. } => 2,
            Geom::Ellipse { .. } => 3,
            Geom::Polyline { .. } => 4,
            Geom::Spline { .. } => 5,
            Geom::Composite { .. } => 6,
        }
    }

    /// Source-space bounds `[min_x, min_y, max_x, max_y]`. Exact for lines, polylines, circles and
    /// arcs; conservative (full ellipse, control hull, union of parts) for the others.
    pub fn bbox(&self) -> [f64; 4] {
        fn grow(b: &mut [f64; 4], p: Pt) {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        let empty = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        match self {
            Geom::Line { a, b } => [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])],
            Geom::Circle { c, r } => [c[0] - r, c[1] - r, c[0] + r, c[1] + r],
            Geom::Arc { c, r, a0, a1 } => {
                let mut b = empty;
                for t in [*a0, *a1] {
                    grow(&mut b, [c[0] + r * t.cos(), c[1] + r * t.sin()]);
                }
                for k in 0..4 {
                    let t = k as f64 * core::f64::consts::FRAC_PI_2;
                    let along = (t - a0).rem_euclid(core::f64::consts::TAU);
                    if along <= a1 - a0 {
                        grow(&mut b, [c[0] + r * t.cos(), c[1] + r * t.sin()]);
                    }
                }
                b
            }
            Geom::Ellipse { c, a, b, rot, .. } => {
                let (s, co) = rot.sin_cos();
                let hx = ((a * co).powi(2) + (b * s).powi(2)).sqrt();
                let hy = ((a * s).powi(2) + (b * co).powi(2)).sqrt();
                [c[0] - hx, c[1] - hy, c[0] + hx, c[1] + hy]
            }
            Geom::Polyline { pts } | Geom::Spline { cps: pts, .. } => {
                let mut b = empty;
                for p in pts {
                    grow(&mut b, *p);
                }
                b
            }
            Geom::Composite { parts } => {
                let mut b = empty;
                for p in parts {
                    let q = p.bbox();
                    grow(&mut b, [q[0], q[1]]);
                    grow(&mut b, [q[2], q[3]]);
                }
                b
            }
        }
    }
}

/// FNV-1a over a stream of 64-bit words. Std only, order sensitive, and stable across platforms
/// because floats are hashed by bit pattern.
#[derive(Clone, Copy)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf29ce484222325)
    }
}

impl Fnv {
    /// Mixes one word.
    #[inline]
    pub fn word(&mut self, w: u64) {
        for i in 0..8 {
            self.0 ^= (w >> (8 * i)) & 0xff;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    /// Mixes a float by bit pattern.
    #[inline]
    pub fn f(&mut self, x: f64) {
        self.word(x.to_bits());
    }
}

fn hash_geom(h: &mut Fnv, g: &Geom) {
    h.word(g.tag() as u64);
    match g {
        Geom::Line { a, b } => {
            for v in [a[0], a[1], b[0], b[1]] {
                h.f(v);
            }
        }
        Geom::Circle { c, r } => {
            for v in [c[0], c[1], *r] {
                h.f(v);
            }
        }
        Geom::Arc { c, r, a0, a1 } => {
            for v in [c[0], c[1], *r, *a0, *a1] {
                h.f(v);
            }
        }
        Geom::Ellipse { c, a, b, rot, t0, t1 } => {
            for v in [c[0], c[1], *a, *b, *rot, *t0, *t1] {
                h.f(v);
            }
        }
        Geom::Polyline { pts } => {
            h.word(pts.len() as u64);
            for p in pts {
                h.f(p[0]);
                h.f(p[1]);
            }
        }
        Geom::Spline { degree, cps, knots, weights } => {
            h.word(*degree as u64);
            h.word(cps.len() as u64);
            for p in cps {
                h.f(p[0]);
                h.f(p[1]);
            }
            for k in knots {
                h.f(*k);
            }
            h.word(weights.is_some() as u64);
            if let Some(w) = weights {
                for x in w {
                    h.f(*x);
                }
            }
        }
        Geom::Composite { parts } => {
            h.word(parts.len() as u64);
            for p in parts {
                hash_geom(h, p);
            }
        }
    }
}

impl Drawing {
    /// Content hash of every entity, in order. A layout decoder must reproduce it.
    pub fn checksum(&self) -> u64 {
        let mut h = Fnv::default();
        for e in &self.entities {
            h.word(e.id as u64);
            hash_geom(&mut h, &e.geom);
        }
        h.0
    }
    /// Counts of entities per [`Geom::tag`].
    pub fn kind_counts(&self) -> [usize; 7] {
        let mut n = [0; 7];
        for e in &self.entities {
            n[e.geom.tag() as usize] += 1;
        }
        n
    }
}

/// One step of the committed viewport script, as fractions of the drawing extent.
#[derive(Clone, Debug)]
pub struct Step {
    /// What the step models.
    pub name: &'static str,
    /// `[x0, y0, x1, y1]` as fractions in `[0, 1]` of the extent, y up.
    pub rect: [f64; 4],
}

/// The viewport script replayed against every candidate (P1.4, P1.5, P1.6): cold open on the whole
/// drawing, zoom in, pan across, zoom in again, zoom back out. Fractions of the extent, so the same
/// script applies to every fixture. Each step is a visible rectangle in source space.
pub fn viewport_script() -> Vec<Step> {
    let mut v = vec![
        Step { name: "cold open (whole drawing)", rect: [0.0, 0.0, 1.0, 1.0] },
        Step { name: "zoom in (1/4 x 1/4)", rect: [0.375, 0.375, 0.625, 0.625] },
    ];
    for i in 1..=4 {
        let dx = 0.125 * i as f64;
        v.push(Step { name: "pan right", rect: [0.375 + dx, 0.375, 0.625 + dx, 0.625] });
    }
    v.push(Step { name: "zoom in (1/16 x 1/16)", rect: [0.84375, 0.4375, 0.90625, 0.5] });
    v.push(Step { name: "zoom out (1/4 x 1/4)", rect: [0.75, 0.375, 1.0, 0.625] });
    v.push(Step { name: "zoom out (whole drawing)", rect: [0.0, 0.0, 1.0, 1.0] });
    v
}

impl Drawing {
    /// Turns a script step into a source-space rectangle for this drawing.
    pub fn viewport(&self, s: &Step) -> [f64; 4] {
        let [x0, y0, x1, y1] = self.extent;
        let (w, h) = (x1 - x0, y1 - y0);
        [x0 + s.rect[0] * w, y0 + s.rect[1] * h, x0 + s.rect[2] * w, y0 + s.rect[3] * h]
    }
}

/// True when two rectangles `[x0, y0, x1, y1]` overlap (touching counts).
pub fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}
