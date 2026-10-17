//! Four ways to find the entities of a viewport and one way to exclude, compared on bytes and ranges.
//!
//! Nothing here writes a file. Each model assigns every entity a payload offset and a length under a
//! stated byte model, builds its index in memory, states how many bytes that index would take on disk,
//! and answers a viewport by listing the byte ranges a remote reader would have to fetch. The ranges
//! are merged the same way P1.4 merges them, so the tables can be read against each other.
#![allow(missing_docs)]

pub mod buckets;
pub mod cells;
pub mod curve;
pub mod quadtree;
pub mod rtree;

use pvec_fixtures::{Drawing, Geom};

/// Number of bytes per payload under the model, one per entity, plus bounds and the square world.
pub struct Items {
    pub bbox: Vec<[f64; 4]>,
    pub size: Vec<u32>,
    /// `[x0, y0, side]` of the smallest square that holds every bound.
    pub world: [f64; 3],
}

/// Bytes one entity takes in the payload, calibrated to layout B's measured bytes per entity on
/// `million` (`01-5-layouts.md`): 8 bytes per `f64` plus 5 bytes of id and framing, plus the 16-byte
/// record header and 6 bytes of padding that B pays for splines, and for composites the header, 22
/// bytes of padding and 4 bytes per part tag.
pub fn size_model(g: &Geom) -> u32 {
    fn f64s(g: &Geom) -> usize {
        match g {
            Geom::Line { .. } => 4,
            Geom::Circle { .. } => 3,
            Geom::Arc { .. } => 5,
            Geom::Ellipse { .. } => 7,
            Geom::Polyline { pts } => 2 * pts.len(),
            Geom::Spline { cps, knots, weights, .. } => {
                2 * cps.len() + knots.len() + weights.as_ref().map_or(0, |w| w.len())
            }
            Geom::Composite { parts } => parts.iter().map(f64s).sum(),
        }
    }
    let extra = match g {
        Geom::Spline { .. } => 16 + 6,
        Geom::Composite { parts } => 16 + 22 + 4 * parts.len(),
        _ => 0,
    };
    (8 * f64s(g) + 5 + extra) as u32
}

pub fn items_of(d: &Drawing) -> Items {
    let bbox: Vec<[f64; 4]> = d.entities.iter().map(|e| e.geom.bbox()).collect();
    let size = d.entities.iter().map(|e| size_model(&e.geom)).collect();
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for r in &bbox {
        b[0] = b[0].min(r[0]);
        b[1] = b[1].min(r[1]);
        b[2] = b[2].max(r[2]);
        b[3] = b[3].max(r[3]);
    }
    let side = (b[2] - b[0]).max(b[3] - b[1]);
    Items { bbox, size, world: [b[0], b[1], side] }
}

impl Items {
    pub fn len(&self) -> usize {
        self.bbox.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bbox.is_empty()
    }
    /// Longest side of an entity's bounds.
    pub fn side(&self, i: usize) -> f64 {
        let b = &self.bbox[i];
        (b[2] - b[0]).max(b[3] - b[1])
    }
    pub fn center(&self, i: usize) -> [f64; 2] {
        let b = &self.bbox[i];
        [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0]
    }
    /// Cell coordinates of a point at `level` (cells per axis `2^level`), clamped into the world.
    pub fn cell_of(&self, p: [f64; 2], level: u32) -> (u32, u32) {
        let n = 1u64 << level;
        let f =
            |v: f64, o: f64| (((v - o) / self.world[2] * n as f64).floor().max(0.0) as u64).min(n - 1) as u32;
        (f(p[0], self.world[0]), f(p[1], self.world[1]))
    }
    /// Bounds of cell `(ix, iy)` at `level`.
    pub fn cell_rect(&self, level: u32, ix: u32, iy: u32) -> [f64; 4] {
        let c = self.world[2] / (1u64 << level) as f64;
        let (x, y) = (self.world[0] + ix as f64 * c, self.world[1] + iy as f64 * c);
        [x, y, x + c, y + c]
    }
    pub fn cell_size(&self, level: u32) -> f64 {
        self.world[2] / (1u64 << level) as f64
    }
}

pub fn overlaps(a: &[f64; 4], b: &[f64; 4]) -> bool {
    a[0] <= b[2] && a[2] >= b[0] && a[1] <= b[3] && a[3] >= b[1]
}

/// What a remote reader would have to do to answer one viewport.
#[derive(Default, Debug)]
pub struct Plan {
    /// Index byte ranges `(start, len)`, absolute in the file (index region first).
    pub index_reads: Vec<(u64, u64)>,
    /// Payload byte ranges `(start, len)`, absolute.
    pub payload_reads: Vec<(u64, u64)>,
    /// Every entity whose payload the reads cover, intersecting or not.
    pub fetched: Vec<u32>,
}

impl Plan {
    pub fn all_reads(&self) -> Vec<(u64, u64)> {
        let mut v = self.index_reads.clone();
        v.extend(&self.payload_reads);
        v
    }
}

/// One candidate.
pub trait Model {
    fn name(&self) -> String;
    /// Bytes of the index region on disk, under the structure's stated record sizes.
    fn index_bytes(&self) -> u64;
    /// References stored per entity (1.0 when every entity is owned exactly once).
    fn duplication(&self) -> f64;
    fn query(&self, items: &Items, rect: &[f64; 4]) -> Plan;
    /// The largest single unit a query can be forced to fetch, with a label, and the unit holding the
    /// whole-extent entities (root cell, longest bucket, or the leaf containing them).
    fn worst(&self, items: &Items) -> Vec<(String, f64)>;
}

/// Merges ranges whose gap is at most `gap` bytes: `(ranges, bytes)`.
pub fn merge(reads: &[(u64, u64)], gap: u64) -> (usize, u64) {
    let mut v: Vec<(u64, u64)> = reads.iter().map(|&(s, n)| (s, s + n)).collect();
    v.sort_unstable();
    let mut out: Vec<(u64, u64)> = Vec::new();
    for (s, e) in v {
        match out.last_mut() {
            Some(l) if s <= l.1 + gap => l.1 = l.1.max(e),
            _ => out.push((s, e)),
        }
    }
    (out.len(), out.iter().map(|r| r.1 - r.0).sum())
}

/// The entity with the longest side: the straddler every model has to home somewhere.
pub fn longest(items: &Items) -> u32 {
    (0..items.len() as u32)
        .max_by(|&a, &b| items.side(a as usize).total_cmp(&items.side(b as usize)))
        .unwrap_or(0)
}

/// Ground truth by brute force: entities whose bounds overlap `rect`.
pub fn truth(items: &Items, rect: &[f64; 4]) -> Vec<u32> {
    (0..items.len() as u32).filter(|&i| overlaps(&items.bbox[i as usize], rect)).collect()
}

/// Bytes of payload for a set of entity ids.
pub fn payload_bytes(items: &Items, ids: &[u32]) -> u64 {
    ids.iter().map(|&i| items.size[i as usize] as u64).sum()
}

/// CPU seconds used by this process. MARS is shared and heavily loaded, so timings are CPU time.
pub fn cpu_secs() -> f64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid, writable `timespec` and CLOCK_PROCESS_CPUTIME_ID exists on Linux.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut ts) };
    ts.tv_sec as f64 + ts.tv_nsec as f64 / 1_000_000_000.0
}

/// A drawing with extra long polylines on top of `base`, for the congestion stress case. Spike-local,
/// not part of corpus revision r1: `count` polylines of `verts` vertices, each a random walk whose
/// first and last vertex are at least `min_span` of the extent apart.
pub fn with_long_polylines(base: &Drawing, count: usize, verts: usize, seed: u64) -> Drawing {
    use pvec_fixtures::{Entity, generate::Rng};
    let mut rng = Rng(seed);
    let mut d = base.clone();
    let w = d.extent[2];
    for _ in 0..count {
        let y = rng.range(0.0, w);
        let mut pts = Vec::with_capacity(verts);
        for k in 0..verts {
            let x = w * k as f64 / (verts - 1) as f64;
            pts.push([x, y + rng.range(-w / 200.0, w / 200.0)]);
        }
        d.entities.push(Entity { id: d.entities.len() as u32, geom: Geom::Polyline { pts } });
    }
    d.name = "stress-long";
    d
}
