//! M2: packed R-tree over a Hilbert ordering. Entities are sorted by the Hilbert key of their bounds
//! centre and cut into leaves of `cap` entities; each internal level groups `fanout` children. Every
//! entity is stored once. The unit of I/O is a leaf's payload, and a leaf's bounds grow to cover any
//! long entity in it.

use crate::curve::hilbert;
use crate::{Items, Model, Plan, overlaps};

const ENTRY: u64 = 32;
const HEADER: u64 = 64;

pub struct RTree {
    cap: usize,
    fanout: usize,
    /// Entities in file order.
    order: Vec<u32>,
    /// Payload offset of each position in `order`, plus the end (length n + 1).
    pay: Vec<u64>,
    /// `levels[0]` are leaves; each entry is a bounding box.
    levels: Vec<Vec<[f64; 4]>>,
    /// Absolute offset of each level's first entry (root level first in the file).
    level_off: Vec<u64>,
    base: u64,
    world_side: f64,
}

fn union(a: &[f64; 4], b: &[f64; 4]) -> [f64; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

impl RTree {
    pub fn build(items: &Items, cap: usize, fanout: usize) -> RTree {
        let n = items.len();
        let mut keyed: Vec<(u64, u32)> = (0..n as u32)
            .map(|i| {
                let (ix, iy) = items.cell_of(items.center(i as usize), 16);
                (hilbert(16, ix, iy), i)
            })
            .collect();
        keyed.sort_unstable();
        let order: Vec<u32> = keyed.into_iter().map(|k| k.1).collect();
        let mut pay = Vec::with_capacity(n + 1);
        let mut cursor = 0u64;
        for &e in &order {
            pay.push(cursor);
            cursor += items.size[e as usize] as u64;
        }
        pay.push(cursor);
        let mut levels: Vec<Vec<[f64; 4]>> = vec![
            order
                .chunks(cap)
                .map(|c| c.iter().map(|&e| items.bbox[e as usize]).reduce(|a, b| union(&a, &b)).unwrap())
                .collect(),
        ];
        while levels.last().unwrap().len() > 1 {
            let next: Vec<[f64; 4]> = levels
                .last()
                .unwrap()
                .chunks(fanout)
                .map(|c| c.iter().copied().reduce(|a, b| union(&a, &b)).unwrap())
                .collect();
            levels.push(next);
        }
        // File order: root level first.
        let mut level_off = vec![0u64; levels.len()];
        let mut at = HEADER;
        for l in (0..levels.len()).rev() {
            level_off[l] = at;
            at += ENTRY * levels[l].len() as u64;
        }
        RTree { cap, fanout, order, pay, levels, level_off, base: at, world_side: items.world[2] }
    }
}

impl Model for RTree {
    fn name(&self) -> String {
        format!("M2 packed Hilbert R-tree (leaf {}, fanout {})", self.cap, self.fanout)
    }
    fn index_bytes(&self) -> u64 {
        self.base
    }
    fn duplication(&self) -> f64 {
        1.0
    }
    fn query(&self, items: &Items, rect: &[f64; 4]) -> Plan {
        let _ = items;
        let mut plan = Plan::default();
        let top = self.levels.len() - 1;
        plan.index_reads.push((self.level_off[top], ENTRY));
        if !overlaps(&self.levels[top][0], rect) {
            return plan;
        }
        let mut frontier = vec![0usize];
        for l in (0..top).rev() {
            let mut next = Vec::new();
            for &p in &frontier {
                let (a, b) = (p * self.fanout, ((p + 1) * self.fanout).min(self.levels[l].len()));
                plan.index_reads.push((self.level_off[l] + ENTRY * a as u64, ENTRY * (b - a) as u64));
                for j in a..b {
                    if overlaps(&self.levels[l][j], rect) {
                        next.push(j);
                    }
                }
            }
            frontier = next;
        }
        let n = self.order.len();
        for leaf in frontier {
            let (a, b) = (leaf * self.cap, ((leaf + 1) * self.cap).min(n));
            plan.payload_reads.push((self.base + self.pay[a], self.pay[b] - self.pay[a]));
            plan.fetched.extend(&self.order[a..b]);
        }
        plan
    }
    fn worst(&self, items: &Items) -> Vec<(String, f64)> {
        let n = self.order.len();
        let mut big = 0u64;
        for leaf in 0..self.levels[0].len() {
            let (a, b) = (leaf * self.cap, ((leaf + 1) * self.cap).min(n));
            big = big.max(self.pay[b] - self.pay[a]);
        }
        let widest = self.levels[0].iter().map(|b| (b[2] - b[0]).max(b[3] - b[1])).fold(0.0, f64::max)
            / self.world_side;
        let long = crate::longest(items);
        let pos = self.order.iter().position(|&e| e == long).unwrap_or(0);
        let leaf = pos / self.cap;
        let (a, b) = (leaf * self.cap, ((leaf + 1) * self.cap).min(n));
        vec![
            ("largest leaf payload bytes".into(), big as f64),
            ("widest leaf bounds, fraction of world side".into(), widest),
            ("leaf holding the longest entity: payload bytes".into(), (self.pay[b] - self.pay[a]) as f64),
            (
                "leaf holding the longest entity: bounds, fraction of world side".into(),
                (self.levels[0][leaf][2] - self.levels[0][leaf][0])
                    .max(self.levels[0][leaf][3] - self.levels[0][leaf][1])
                    / self.world_side,
            ),
        ]
    }
}
