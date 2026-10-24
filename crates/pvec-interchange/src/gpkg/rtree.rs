//! The spatial index: the shadow tables of a SQLite R-tree virtual table, built bulk-load style.
//!
//! GeoPackage's index is `CREATE VIRTUAL TABLE rtree_<table>_<column> USING rtree(...)`. A reader that
//! has the R-tree module loaded finds the tree in three ordinary tables, `_node`, `_parent` and
//! `_rowid`, and this module writes them directly. The layout is the one SQLite's `rtree.c` documents:
//! a node is a blob of exactly `page_size - 64` bytes, holding a 2 byte tree depth (the root only),
//! a 2 byte cell count, then cells of an 8 byte big endian id and four 32 bit big endian floats
//! (`min x`, `max x`, `min y`, `max y`). The root is node 1. Boxes are rounded outward when they are
//! narrowed to 32 bits, so the index never misses a feature.
//!
//! The tree is packed with the Sort-Tile-Recursive method: a pure function of the boxes and their
//! order, with ties broken by id, so the same features always give the same nodes.

use super::sqlite::Value;

/// Cells a node holds: the most that fit, less some room for growth if a reader inserts later.
const FILL: usize = 100;
const CELL: usize = 24;

fn prev_f32(f: f32) -> f32 {
    if f == 0.0 {
        -f32::from_bits(1)
    } else if f > 0.0 {
        f32::from_bits(f.to_bits() - 1)
    } else {
        f32::from_bits(f.to_bits() + 1)
    }
}

fn next_f32(f: f32) -> f32 {
    if f == 0.0 {
        f32::from_bits(1)
    } else if f > 0.0 {
        f32::from_bits(f.to_bits() + 1)
    } else {
        f32::from_bits(f.to_bits() - 1)
    }
}

/// The largest 32-bit float at or below `v`.
fn down(v: f64) -> f32 {
    let f = v as f32;
    if f64::from(f) > v { prev_f32(f) } else { f }
}

/// The smallest 32-bit float at or above `v`.
fn up(v: f64) -> f32 {
    let f = v as f32;
    if f64::from(f) < v { next_f32(f) } else { f }
}

/// A box as the index stores it: `[min x, max x, min y, max y]`, widened to 32-bit floats.
pub(super) fn widen(b: [f64; 4]) -> [f32; 4] {
    [down(b[0]), up(b[1]), down(b[2]), up(b[3])]
}

struct Entry {
    id: i64,
    bx: [f32; 4],
}

fn union(boxes: impl Iterator<Item = [f32; 4]>) -> [f32; 4] {
    let mut u = [f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY];
    for b in boxes {
        u = [u[0].min(b[0]), u[1].max(b[1]), u[2].min(b[2]), u[3].max(b[3])];
    }
    u
}

fn node_blob(size: usize, depth: u16, cells: &[Entry]) -> Vec<u8> {
    let mut data = vec![0u8; size];
    data[..2].copy_from_slice(&depth.to_be_bytes());
    data[2..4].copy_from_slice(&(cells.len() as u16).to_be_bytes());
    for (i, c) in cells.iter().enumerate() {
        let at = 4 + i * CELL;
        data[at..at + 8].copy_from_slice(&c.id.to_be_bytes());
        for (k, v) in c.bx.iter().enumerate() {
            data[at + 8 + 4 * k..at + 12 + 4 * k].copy_from_slice(&v.to_be_bytes());
        }
    }
    data
}

/// The rows of the three shadow tables, each in rowid order.
pub(super) struct Shadow {
    pub node: Vec<(i64, Vec<Value>)>,
    pub parent: Vec<(i64, Vec<Value>)>,
    pub rowid: Vec<(i64, Vec<Value>)>,
}

/// Builds the tree for `(id, box)` pairs, where `node_size` is the page size less 64.
pub(super) fn build(items: &[(i64, [f64; 4])], node_size: usize) -> Shadow {
    assert!(4 + FILL * CELL <= node_size);
    let mut entries: Vec<Entry> = items.iter().map(|(id, b)| Entry { id: *id, bx: widen(*b) }).collect();
    // leaves: sort by x centre, cut into vertical slices, sort each slice by y centre, cut into nodes
    let centre = |b: &[f32; 4], axis: usize| f64::from(b[2 * axis]) + f64::from(b[2 * axis + 1]);
    entries.sort_by(|a, b| centre(&a.bx, 0).total_cmp(&centre(&b.bx, 0)).then(a.id.cmp(&b.id)));
    let leaf_count = entries.len().div_ceil(FILL).max(1);
    let slices = (leaf_count as f64).sqrt().ceil() as usize;
    let per_slice = (slices * FILL).max(1);
    let mut ordered: Vec<Entry> = Vec::with_capacity(entries.len());
    let mut rest = entries;
    while !rest.is_empty() {
        let tail = rest.split_off(per_slice.min(rest.len()));
        rest.sort_by(|a, b| centre(&a.bx, 1).total_cmp(&centre(&b.bx, 1)).then(a.id.cmp(&b.id)));
        ordered.append(&mut rest);
        rest = tail;
    }
    // levels[0] are the leaves; each node is its list of cells
    let mut levels: Vec<Vec<Vec<Entry>>> = Vec::new();
    let mut leaves: Vec<Vec<Entry>> = Vec::new();
    let mut it = ordered.into_iter().peekable();
    while it.peek().is_some() {
        leaves.push(it.by_ref().take(FILL).collect());
    }
    if leaves.is_empty() {
        leaves.push(Vec::new());
    }
    levels.push(leaves);
    // `placeholder` ids are replaced by node numbers once they are known
    while levels.last().unwrap().len() > 1 {
        let below = levels.last().unwrap();
        let cells: Vec<Entry> = below
            .iter()
            .enumerate()
            .map(|(i, n)| Entry { id: i as i64, bx: union(n.iter().map(|c| c.bx)) })
            .collect();
        let mut parents: Vec<Vec<Entry>> = Vec::new();
        let mut it = cells.into_iter().peekable();
        while it.peek().is_some() {
            parents.push(it.by_ref().take(FILL).collect());
        }
        levels.push(parents);
    }
    // Number the nodes from the root down: the root is node 1, then each level below it in order.
    let depth = levels.len() - 1;
    let mut first_number = vec![0i64; levels.len()];
    let mut next = 1i64;
    for l in (0..levels.len()).rev() {
        first_number[l] = next;
        next += levels[l].len() as i64;
    }
    let mut out = Shadow { node: Vec::new(), parent: Vec::new(), rowid: Vec::new() };
    for l in (0..levels.len()).rev() {
        for (i, node) in levels[l].iter().enumerate() {
            let number = first_number[l] + i as i64;
            let cells: Vec<Entry> = if l == 0 {
                node.iter().map(|c| Entry { id: c.id, bx: c.bx }).collect()
            } else {
                node.iter()
                    .map(|c| {
                        let child = first_number[l - 1] + c.id;
                        Entry { id: child, bx: c.bx }
                    })
                    .collect()
            };
            let d = if number == 1 { depth as u16 } else { 0 };
            out.node.push((number, vec![Value::Null, Value::Blob(node_blob(node_size, d, &cells))]));
            for c in &cells {
                if l == 0 {
                    out.rowid.push((c.id, vec![Value::Null, Value::Int(number)]));
                } else {
                    out.parent.push((c.id, vec![Value::Null, Value::Int(number)]));
                }
            }
        }
    }
    out.node.sort_by_key(|r| r.0);
    out.parent.sort_by_key(|r| r.0);
    out.rowid.sort_by_key(|r| r.0);
    out
}
