//! M3: hierarchical cells that hold compact references to payload stored once. An entity goes to the
//! level whose cell is at least as big as the entity, so it overlaps at most 2 x 2 cells of that
//! level, and a reference to it is written in each (bounded duplication, at most 4). The payload is
//! stored once, in Hilbert order, so references into one area point at nearby bytes. With
//! `ref_bbox` a reference also carries the entity's bounds, so the reader can drop references that
//! do not touch the viewport before fetching any payload.

use crate::curve::{hilbert, morton};
use crate::{Items, Model, Plan, overlaps};
use std::collections::{BTreeMap, HashMap};

const DIR_ENTRY: u64 = 16;
const HEADER: u64 = 64;

pub struct Cells {
    max_level: u32,
    ref_bbox: bool,
    /// Directory in (level, morton) order: references of cell `k` are `refs[start[k]..start[k+1]]`.
    keys: Vec<(u32, u32, u32)>,
    start: Vec<usize>,
    refs: Vec<u32>,
    lookup: HashMap<(u32, u32, u32), usize>,
    levels_present: Vec<u32>,
    /// Payload offset per entity.
    off: Vec<u64>,
    base: u64,
    n_entities: usize,
}

impl Cells {
    pub fn ref_bytes(&self) -> u64 {
        if self.ref_bbox { 12 + 16 } else { 12 }
    }

    pub fn build(items: &Items, max_level: u32, ref_bbox: bool) -> Cells {
        let n = items.len();
        let w = items.world[2];
        let mut map: BTreeMap<(u32, u64), Vec<u32>> = BTreeMap::new();
        let mut cellxy: HashMap<(u32, u64), (u32, u32)> = HashMap::new();
        for i in 0..n {
            let s = items.side(i);
            let l =
                if s <= 0.0 { max_level } else { ((w / s).log2().floor().max(0.0) as u32).min(max_level) };
            let b = &items.bbox[i];
            let (x0, y0) = items.cell_of([b[0], b[1]], l);
            let (x1, y1) = items.cell_of([b[2], b[3]], l);
            for ix in x0..=x1 {
                for iy in y0..=y1 {
                    let k = (l, morton(ix, iy));
                    map.entry(k).or_default().push(i as u32);
                    cellxy.insert(k, (ix, iy));
                }
            }
        }
        let mut keys = Vec::with_capacity(map.len());
        let mut start = vec![0usize];
        let mut refs = Vec::new();
        let mut lookup = HashMap::new();
        for (k, v) in &map {
            let (ix, iy) = cellxy[k];
            lookup.insert((k.0, ix, iy), keys.len());
            keys.push((k.0, ix, iy));
            refs.extend(v);
            start.push(refs.len());
        }
        let mut levels_present: Vec<u32> = keys.iter().map(|k| k.0).collect();
        levels_present.dedup();
        // Payload in Hilbert order of the centre.
        let mut keyed: Vec<(u64, u32)> = (0..n as u32)
            .map(|i| {
                let (ix, iy) = items.cell_of(items.center(i as usize), 16);
                (hilbert(16, ix, iy), i)
            })
            .collect();
        keyed.sort_unstable();
        let mut off = vec![0u64; n];
        let mut cursor = 0u64;
        for (_, e) in keyed {
            off[e as usize] = cursor;
            cursor += items.size[e as usize] as u64;
        }
        let me = Cells {
            max_level,
            ref_bbox,
            keys,
            start,
            refs,
            lookup,
            levels_present,
            off,
            base: 0,
            n_entities: n,
        };
        let base = HEADER + DIR_ENTRY * me.keys.len() as u64 + me.ref_bytes() * me.refs.len() as u64;
        Cells { base, ..me }
    }

    fn refs_base(&self) -> u64 {
        HEADER + DIR_ENTRY * self.keys.len() as u64
    }
}

impl Model for Cells {
    fn name(&self) -> String {
        format!(
            "M3 cells + duplicated references (max level {}, {})",
            self.max_level,
            if self.ref_bbox { "refs carry bounds" } else { "refs without bounds" }
        )
    }
    fn index_bytes(&self) -> u64 {
        self.base
    }
    fn duplication(&self) -> f64 {
        self.refs.len() as f64 / self.n_entities as f64
    }
    fn query(&self, items: &Items, rect: &[f64; 4]) -> Plan {
        let mut plan = Plan::default();
        let mut found: Vec<u32> = Vec::new();
        for &l in &self.levels_present {
            let (x0, y0) = items.cell_of([rect[0], rect[1]], l);
            let (x1, y1) = items.cell_of([rect[2], rect[3]], l);
            for ix in x0..=x1 {
                for iy in y0..=y1 {
                    if let Some(&k) = self.lookup.get(&(l, ix, iy)) {
                        plan.index_reads.push((HEADER + DIR_ENTRY * k as u64, DIR_ENTRY));
                        let (a, b) = (self.start[k], self.start[k + 1]);
                        plan.index_reads.push((
                            self.refs_base() + self.ref_bytes() * a as u64,
                            self.ref_bytes() * (b - a) as u64,
                        ));
                        found.extend(&self.refs[a..b]);
                    }
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        if self.ref_bbox {
            found.retain(|&e| overlaps(&items.bbox[e as usize], rect));
        }
        for &e in &found {
            plan.payload_reads.push((self.base + self.off[e as usize], items.size[e as usize] as u64));
        }
        plan.fetched = found;
        plan
    }
    fn worst(&self, items: &Items) -> Vec<(String, f64)> {
        let big = (0..self.keys.len())
            .map(|k| (self.start[k + 1] - self.start[k]) as u64 * self.ref_bytes())
            .max()
            .unwrap_or(0);
        let root = self
            .lookup
            .get(&(0, 0, 0))
            .map_or(0, |&k| (self.start[k + 1] - self.start[k]) as u64 * self.ref_bytes());
        let long = crate::longest(items);
        let home = (0..self.keys.len())
            .filter(|&k| self.refs[self.start[k]..self.start[k + 1]].contains(&long))
            .map(|k| (self.start[k + 1] - self.start[k]) as u64 * self.ref_bytes())
            .max()
            .unwrap_or(0);
        vec![
            ("largest cell reference list bytes".into(), big as f64),
            ("level-0 cell reference list bytes".into(), root as f64),
            ("reference list of the cell holding the longest entity: bytes (and it is referenced from up to 4 cells)".into(), home as f64),
        ]
    }
}
