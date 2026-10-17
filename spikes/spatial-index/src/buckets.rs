//! M4: single-ownership leaf cells plus per-level buckets for long entities. An entity no longer than
//! a leaf cell is owned by the leaf cell holding its centre (cell bounds are loose by half a cell).
//! Anything longer goes to the bucket of its size class, `ceil(log2(side / cell))`, where entities are
//! sorted by Hilbert key and cut into chunks of 64 with a bounds table that is always read, so a long
//! entity never inflates a leaf and is only fetched when its chunk's bounds touch the viewport.

#![allow(dead_code)]

use crate::curve::{hilbert, morton};
use crate::{Items, Model, Plan, overlaps};
use std::collections::{BTreeMap, HashMap};

const DIR_ENTRY: u64 = 16;
const CHUNK_ENTRY: u64 = 32;
const HEADER: u64 = 64;
const CHUNK: usize = 64;

struct Leaf {
    ix: u32,
    iy: u32,
    ents: Vec<u32>,
    pay_off: u64,
    pay_len: u64,
}

struct Chunk {
    bbox: [f64; 4],
    ents: Vec<u32>,
    pay_off: u64,
    pay_len: u64,
    class: u32,
}

pub struct Buckets {
    leaf_level: u32,
    leaves: Vec<Leaf>,
    lookup: HashMap<(u32, u32), usize>,
    chunks: Vec<Chunk>,
    base: u64,
    n: usize,
}

impl Buckets {
    pub fn build(items: &Items, leaf_level: u32) -> Buckets {
        let c = items.cell_size(leaf_level);
        let mut cells: BTreeMap<u64, (u32, u32, Vec<u32>)> = BTreeMap::new();
        let mut long: BTreeMap<u32, Vec<(u64, u32)>> = BTreeMap::new();
        for i in 0..items.len() {
            let s = items.side(i);
            if s <= c {
                let (ix, iy) = items.cell_of(items.center(i), leaf_level);
                cells.entry(morton(ix, iy)).or_insert((ix, iy, vec![])).2.push(i as u32);
            } else {
                let class = (s / c).log2().ceil().max(1.0) as u32;
                let (ix, iy) = items.cell_of(items.center(i), 16);
                long.entry(class).or_default().push((hilbert(16, ix, iy), i as u32));
            }
        }
        let mut cursor = 0u64;
        let mut leaves = Vec::new();
        let mut lookup = HashMap::new();
        for (_, (ix, iy, ents)) in cells {
            let len: u64 = ents.iter().map(|&e| items.size[e as usize] as u64).sum();
            lookup.insert((ix, iy), leaves.len());
            leaves.push(Leaf { ix, iy, ents, pay_off: cursor, pay_len: len });
            cursor += len;
        }
        let mut chunks = Vec::new();
        for (class, mut v) in long {
            v.sort_unstable();
            for part in v.chunks(CHUNK) {
                let ents: Vec<u32> = part.iter().map(|p| p.1).collect();
                let len: u64 = ents.iter().map(|&e| items.size[e as usize] as u64).sum();
                let bbox = ents
                    .iter()
                    .map(|&e| items.bbox[e as usize])
                    .reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
                    .unwrap();
                chunks.push(Chunk { bbox, ents, pay_off: cursor, pay_len: len, class });
                cursor += len;
            }
        }
        let base = HEADER + DIR_ENTRY * leaves.len() as u64 + CHUNK_ENTRY * chunks.len() as u64;
        Buckets { leaf_level, leaves, lookup, chunks, base, n: items.len() }
    }
}

impl Model for Buckets {
    fn name(&self) -> String {
        format!("M4 leaf cells + long-entity buckets (leaf level {})", self.leaf_level)
    }
    fn index_bytes(&self) -> u64 {
        self.base
    }
    fn duplication(&self) -> f64 {
        let _ = self.n;
        1.0
    }
    fn query(&self, items: &Items, rect: &[f64; 4]) -> Plan {
        let mut plan = Plan::default();
        let h = items.cell_size(self.leaf_level) / 2.0;
        let grown = [rect[0] - h, rect[1] - h, rect[2] + h, rect[3] + h];
        let (x0, y0) = items.cell_of([grown[0], grown[1]], self.leaf_level);
        let (x1, y1) = items.cell_of([grown[2], grown[3]], self.leaf_level);
        for ix in x0..=x1 {
            for iy in y0..=y1 {
                if let Some(&k) = self.lookup.get(&(ix, iy)) {
                    let l = &self.leaves[k];
                    plan.index_reads.push((HEADER + DIR_ENTRY * k as u64, DIR_ENTRY));
                    plan.payload_reads.push((self.base + l.pay_off, l.pay_len));
                    plan.fetched.extend(&l.ents);
                }
            }
        }
        if !self.chunks.is_empty() {
            plan.index_reads.push((
                HEADER + DIR_ENTRY * self.leaves.len() as u64,
                CHUNK_ENTRY * self.chunks.len() as u64,
            ));
            for c in &self.chunks {
                if overlaps(&c.bbox, rect) {
                    plan.payload_reads.push((self.base + c.pay_off, c.pay_len));
                    plan.fetched.extend(&c.ents);
                }
            }
        }
        plan
    }
    fn worst(&self, items: &Items) -> Vec<(String, f64)> {
        let big = self.leaves.iter().map(|l| l.pay_len).max().unwrap_or(0);
        let mut by_class: BTreeMap<u32, u64> = BTreeMap::new();
        for c in &self.chunks {
            *by_class.entry(c.class).or_default() += c.pay_len;
        }
        let bucket = by_class.values().copied().max().unwrap_or(0);
        let long = crate::longest(items);
        let home = match self.chunks.iter().find(|c| c.ents.contains(&long)) {
            Some(c) => by_class[&c.class],
            None => self.leaves.iter().find(|l| l.ents.contains(&long)).map_or(0, |l| l.pay_len),
        };
        vec![
            (
                "size-class bucket (or leaf cell) holding the longest entity: payload bytes".into(),
                home as f64,
            ),
            ("largest leaf-cell payload bytes".into(), big as f64),
            ("largest bucket payload bytes (all chunks of one size class)".into(), bucket as f64),
            (
                "chunk table bytes (read on every query)".into(),
                (CHUNK_ENTRY * self.chunks.len() as u64) as f64,
            ),
        ]
    }
}
