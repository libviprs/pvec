//! M1: loose quadtree with ancestor ownership. Every entity is stored exactly once, in the node whose
//! cell is the smallest that is still at least as big as the entity, found from the entity's centre.
//! Node bounds are loose (the cell grown by half its size on each side), so an entity that sticks out
//! of its cell is still inside its node's bounds. The unit of I/O is a node's whole payload.

use crate::{Items, Model, Plan, overlaps};
use std::collections::{HashMap, HashSet};

struct Node {
    level: u32,
    ix: u32,
    iy: u32,
    ents: Vec<u32>,
    children: Vec<usize>,
    pay_off: u64,
    pay_len: u64,
}

pub struct Quadtree {
    nodes: Vec<Node>,
    max_level: u32,
    base: u64,
}

const NODE_BYTES: u64 = 32;
const HEADER: u64 = 64;

impl Quadtree {
    pub fn build(items: &Items, max_level: u32) -> Quadtree {
        let w = items.world[2];
        let mut cells: HashMap<(u32, u32, u32), Vec<u32>> = HashMap::new();
        for i in 0..items.len() {
            let s = items.side(i);
            let l =
                if s <= 0.0 { max_level } else { ((w / s).log2().floor().max(0.0) as u32).min(max_level) };
            let (ix, iy) = items.cell_of(items.center(i), l);
            cells.entry((l, ix, iy)).or_default().push(i as u32);
        }
        // Every ancestor of a non-empty cell exists, empty or not, so a query can walk down to it.
        let mut exists: HashSet<(u32, u32, u32)> = cells.keys().copied().collect();
        for &(l, ix, iy) in cells.keys() {
            let (mut l, mut ix, mut iy) = (l, ix, iy);
            while l > 0 {
                l -= 1;
                ix >>= 1;
                iy >>= 1;
                if !exists.insert((l, ix, iy)) {
                    break;
                }
            }
        }
        exists.insert((0, 0, 0));
        // Depth-first preorder, so a subtree's payload is contiguous.
        let mut nodes: Vec<Node> = Vec::new();
        let mut stack: Vec<(u32, u32, u32, Option<usize>)> = vec![(0, 0, 0, None)];
        let mut cursor = 0u64;
        while let Some((l, ix, iy, parent)) = stack.pop() {
            let idx = nodes.len();
            let ents = cells.remove(&(l, ix, iy)).unwrap_or_default();
            let len: u64 = ents.iter().map(|&e| items.size[e as usize] as u64).sum();
            nodes.push(Node { level: l, ix, iy, ents, children: vec![], pay_off: cursor, pay_len: len });
            cursor += len;
            if let Some(p) = parent {
                nodes[p].children.push(idx);
            }
            // Push in reverse so the first quadrant is visited first.
            for q in (0..4u32).rev() {
                let c = (l + 1, 2 * ix + (q & 1), 2 * iy + (q >> 1));
                if exists.contains(&c) {
                    stack.push((c.0, c.1, c.2, Some(idx)));
                }
            }
        }
        let base = HEADER + NODE_BYTES * nodes.len() as u64;
        Quadtree { nodes, max_level, base }
    }

    fn loose(&self, items: &Items, n: &Node) -> [f64; 4] {
        let c = items.cell_rect(n.level, n.ix, n.iy);
        let h = items.cell_size(n.level) / 2.0;
        [c[0] - h, c[1] - h, c[2] + h, c[3] + h]
    }
}

impl Model for Quadtree {
    fn name(&self) -> String {
        format!("M1 loose quadtree (max level {})", self.max_level)
    }
    fn index_bytes(&self) -> u64 {
        self.base
    }
    fn duplication(&self) -> f64 {
        1.0
    }
    fn query(&self, items: &Items, rect: &[f64; 4]) -> Plan {
        let mut plan = Plan::default();
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i];
            if !overlaps(&self.loose(items, n), rect) {
                continue;
            }
            plan.index_reads.push((HEADER + NODE_BYTES * i as u64, NODE_BYTES));
            if n.pay_len > 0 {
                plan.payload_reads.push((self.base + n.pay_off, n.pay_len));
                plan.fetched.extend(&n.ents);
            }
            stack.extend(&n.children);
        }
        plan
    }
    fn worst(&self, items: &Items) -> Vec<(String, f64)> {
        let big = self.nodes.iter().map(|n| n.pay_len).max().unwrap_or(0);
        let long = crate::longest(items);
        let home = self.nodes.iter().find(|n| n.ents.contains(&long)).map_or(0, |n| n.pay_len);
        vec![
            ("root node payload bytes".into(), self.nodes[0].pay_len as f64),
            ("largest node payload bytes".into(), big as f64),
            (
                format!(
                    "node holding the longest entity (side {:.0}): payload bytes",
                    items.side(long as usize)
                ),
                home as f64,
            ),
        ]
    }
}
