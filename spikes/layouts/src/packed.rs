//! Candidate B: custom packed binary with an offset table.
//!
//! Immediate geometry is type-batched structure of arrays: one section per field, 8-byte aligned,
//! little endian. Advanced geometry (splines, composites) lives in an arena of self-describing
//! records with a byte-offset table, and each curve's control points, knots and weights are
//! contiguous inside its record.
//!
//! Block layout: 8 byte prefix (magic, version, flags), total length, seven counts, a table of 33
//! `(offset, length)` pairs, then the sections. The reader never trusts an offset: every section is
//! checked against the block length, its alignment and its element size before it is sliced, every
//! offset-table entry is checked to be monotone and in range, and arena records are re-validated
//! against their own headers. There is no `unsafe` in this file and no indexing by a file value.

use crate::common::Codec;
use pvec_fixtures::{Entity, Geom};

const MAGIC: [u8; 4] = *b"PVB1";
const NSEC: usize = 33;
const TABLE_AT: usize = 40;
/// Bytes before the first section.
pub const HEADER_LEN: usize = TABLE_AT + NSEC * 8;

/// Element size of each section, in the order lines, circles, arcs, ellipses, polylines, splines,
/// composites.
const ESIZE: [usize; NSEC] = [
    4, 8, 8, 8, 8, // lines
    4, 8, 8, 8, // circles
    4, 8, 8, 8, 8, 8, // arcs
    4, 8, 8, 8, 8, 8, 8, 8, // ellipses
    4, 4, 8, 8, // polylines
    4, 4, 8, // splines: ids, byte offsets, arena
    4, 4, 8, // composites: ids, byte offsets, arena
];
/// First section of each kind.
const FIRST: [usize; 7] = [0, 5, 9, 15, 23, 27, 30];
/// Bytes of record header in the spline arena (degree, rational, pad, ncps, nknots, pad).
pub const SPLINE_HEADER: usize = 16;
/// Of those, the bytes that carry no information.
pub const SPLINE_HEADER_PAD: usize = 6;

struct Writer {
    out: Vec<u8>,
    secs: [(u32, u32); NSEC],
}

impl Writer {
    fn put(&mut self, idx: usize, bytes: &[u8]) {
        while self.out.len() % 8 != 0 {
            self.out.push(0);
        }
        self.secs[idx] = (self.out.len() as u32, bytes.len() as u32);
        self.out.extend_from_slice(bytes);
    }
    fn put_u32(&mut self, idx: usize, v: &[u32]) {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.put(idx, &b);
    }
    fn put_f64(&mut self, idx: usize, v: &[f64]) {
        let b: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        self.put(idx, &b);
    }
}

/// Candidate B.
pub struct Packed;

fn tag_of(g: &Geom) -> usize {
    g.tag() as usize
}

impl Codec for Packed {
    const NAME: &'static str = "B packed";

    fn encode_block(ents: &[&Entity]) -> Vec<u8> {
        // Kind order of the section table: lines, circles, arcs, ellipses, polylines, splines, composites.
        let order = [0usize, 1, 2, 3, 4, 5, 6];
        let mut by: [Vec<&Entity>; 7] = Default::default();
        for e in ents {
            by[tag_of(&e.geom)].push(e);
        }
        let mut w = Writer { out: vec![0u8; HEADER_LEN], secs: [(0, 0); NSEC] };
        let ids = |v: &[&Entity]| v.iter().map(|e| e.id).collect::<Vec<u32>>();
        for k in order {
            let es = &by[k];
            if es.is_empty() {
                continue;
            }
            let f = FIRST[k];
            w.put_u32(f, &ids(es));
            let col = |pick: &dyn Fn(&Geom) -> f64| es.iter().map(|e| pick(&e.geom)).collect::<Vec<f64>>();
            match k {
                0 => {
                    let g = |e: &Geom, i: usize| match e {
                        Geom::Line { a, b } => [a[0], a[1], b[0], b[1]][i],
                        _ => unreachable!(),
                    };
                    for i in 0..4 {
                        w.put_f64(f + 1 + i, &col(&|e| g(e, i)));
                    }
                }
                1 => {
                    let g = |e: &Geom, i: usize| match e {
                        Geom::Circle { c, r } => [c[0], c[1], *r][i],
                        _ => unreachable!(),
                    };
                    for i in 0..3 {
                        w.put_f64(f + 1 + i, &col(&|e| g(e, i)));
                    }
                }
                2 => {
                    let g = |e: &Geom, i: usize| match e {
                        Geom::Arc { c, r, a0, a1 } => [c[0], c[1], *r, *a0, *a1][i],
                        _ => unreachable!(),
                    };
                    for i in 0..5 {
                        w.put_f64(f + 1 + i, &col(&|e| g(e, i)));
                    }
                }
                3 => {
                    let g = |e: &Geom, i: usize| match e {
                        Geom::Ellipse { c, a, b, rot, t0, t1 } => [c[0], c[1], *a, *b, *rot, *t0, *t1][i],
                        _ => unreachable!(),
                    };
                    for i in 0..7 {
                        w.put_f64(f + 1 + i, &col(&|e| g(e, i)));
                    }
                }
                4 => {
                    let (mut voff, mut xs, mut ys) = (vec![0u32], vec![], vec![]);
                    for e in es {
                        let Geom::Polyline { pts } = &e.geom else { unreachable!() };
                        for p in pts {
                            xs.push(p[0]);
                            ys.push(p[1]);
                        }
                        voff.push(xs.len() as u32);
                    }
                    w.put_u32(f + 1, &voff);
                    w.put_f64(f + 2, &xs);
                    w.put_f64(f + 3, &ys);
                }
                5 => {
                    let (mut offs, mut arena) = (vec![0u32], Vec::<u8>::new());
                    for e in es {
                        let Geom::Spline { degree, cps, knots, weights } = &e.geom else { unreachable!() };
                        arena.push(*degree);
                        arena.push(u8::from(weights.is_some()));
                        arena.extend_from_slice(&[0, 0]);
                        arena.extend_from_slice(&(cps.len() as u32).to_le_bytes());
                        arena.extend_from_slice(&(knots.len() as u32).to_le_bytes());
                        arena.extend_from_slice(&[0; 4]);
                        for p in cps {
                            arena.extend_from_slice(&p[0].to_le_bytes());
                            arena.extend_from_slice(&p[1].to_le_bytes());
                        }
                        for k in knots {
                            arena.extend_from_slice(&k.to_le_bytes());
                        }
                        for x in weights.iter().flatten() {
                            arena.extend_from_slice(&x.to_le_bytes());
                        }
                        offs.push(arena.len() as u32);
                    }
                    w.put_u32(f + 1, &offs);
                    w.put(f + 2, &arena);
                }
                _ => {
                    let (mut offs, mut arena) = (vec![0u32], Vec::<u8>::new());
                    for e in es {
                        let Geom::Composite { parts } = &e.geom else { unreachable!() };
                        arena.extend_from_slice(&(parts.len() as u32).to_le_bytes());
                        arena.extend_from_slice(&[0; 4]);
                        for p in parts {
                            let (tag, vals): (u32, Vec<f64>) = match p {
                                Geom::Line { a, b } => (0, vec![a[0], a[1], b[0], b[1]]),
                                Geom::Arc { c, r, a0, a1 } => (1, vec![c[0], c[1], *r, *a0, *a1]),
                                _ => unreachable!("composites hold lines and arcs"),
                            };
                            arena.extend_from_slice(&tag.to_le_bytes());
                            arena.extend_from_slice(&[0; 4]);
                            for v in vals {
                                arena.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                        offs.push(arena.len() as u32);
                    }
                    w.put_u32(f + 1, &offs);
                    w.put(f + 2, &arena);
                }
            }
        }
        let mut out = w.out;
        while out.len() % 8 != 0 {
            out.push(0);
        }
        out[0..4].copy_from_slice(&MAGIC);
        out[4..6].copy_from_slice(&1u16.to_le_bytes());
        let total = out.len() as u32;
        out[8..12].copy_from_slice(&total.to_le_bytes());
        for k in 0..7 {
            out[12 + 4 * k..16 + 4 * k].copy_from_slice(&(by[k].len() as u32).to_le_bytes());
        }
        for (i, (o, l)) in w.secs.iter().enumerate() {
            out[TABLE_AT + 8 * i..TABLE_AT + 8 * i + 4].copy_from_slice(&o.to_le_bytes());
            out[TABLE_AT + 8 * i + 4..TABLE_AT + 8 * i + 8].copy_from_slice(&l.to_le_bytes());
        }
        out
    }

    fn touch(block: &[u8]) -> Result<u64, String> {
        let v = View::parse(block)?;
        let mut sum = 0u64;
        // Fixed-width kinds: ids plus every float column.
        for k in 0..4 {
            let f = FIRST[k];
            sum = sum.wrapping_add(sum_u32(v.sec[f]));
            for s in f + 1..FIRST[k + 1] {
                sum = sum.wrapping_add(sum_f64(v.sec[s]));
            }
        }
        sum = sum.wrapping_add(touch_polylines(&v)?);
        sum = sum.wrapping_add(touch_splines(&v)?);
        sum = sum.wrapping_add(touch_composites(&v)?);
        Ok(sum)
    }

    fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
        let v = View::parse(block)?;
        let mut out = Vec::new();
        for i in 0..v.n[0] {
            let a = |s: usize| v.f(FIRST[0] + s, i);
            out.push(Entity { id: v.id(0, i), geom: Geom::Line { a: [a(1), a(2)], b: [a(3), a(4)] } });
        }
        for i in 0..v.n[1] {
            let a = |s: usize| v.f(FIRST[1] + s, i);
            out.push(Entity { id: v.id(1, i), geom: Geom::Circle { c: [a(1), a(2)], r: a(3) } });
        }
        for i in 0..v.n[2] {
            let a = |s: usize| v.f(FIRST[2] + s, i);
            out.push(Entity {
                id: v.id(2, i),
                geom: Geom::Arc { c: [a(1), a(2)], r: a(3), a0: a(4), a1: a(5) },
            });
        }
        for i in 0..v.n[3] {
            let a = |s: usize| v.f(FIRST[3] + s, i);
            out.push(Entity {
                id: v.id(3, i),
                geom: Geom::Ellipse { c: [a(1), a(2)], a: a(3), b: a(4), rot: a(5), t0: a(6), t1: a(7) },
            });
        }
        let voff = v.offsets(FIRST[4] + 1, v.n[4], v.sec[FIRST[4] + 2].len() / 8)?;
        for i in 0..v.n[4] {
            let pts = (voff[i]..voff[i + 1])
                .map(|j| [v.f_idx(FIRST[4] + 2, j), v.f_idx(FIRST[4] + 3, j)])
                .collect();
            out.push(Entity { id: v.id(4, i), geom: Geom::Polyline { pts } });
        }
        let offs = v.offsets(FIRST[5] + 1, v.n[5], v.sec[FIRST[5] + 2].len())?;
        for i in 0..v.n[5] {
            let rec = SplineRec::parse(&v.sec[FIRST[5] + 2][offs[i]..offs[i + 1]])?;
            out.push(Entity { id: v.id(5, i), geom: rec.to_geom() });
        }
        let offs = v.offsets(FIRST[6] + 1, v.n[6], v.sec[FIRST[6] + 2].len())?;
        for i in 0..v.n[6] {
            out.push(Entity {
                id: v.id(6, i),
                geom: composite_geom(&v.sec[FIRST[6] + 2][offs[i]..offs[i + 1]])?,
            });
        }
        Ok(out)
    }
}

#[inline(never)]
fn sum_f64(b: &[u8]) -> u64 {
    b.chunks_exact(8).fold(0u64, |a, c| a.wrapping_add(u64::from_le_bytes(c.try_into().unwrap_or([0; 8]))))
}

#[inline(never)]
fn sum_u32(b: &[u8]) -> u64 {
    b.chunks_exact(4)
        .fold(0u64, |a, c| a.wrapping_add(u32::from_le_bytes(c.try_into().unwrap_or([0; 4])) as u64))
}

fn rd_u32(b: &[u8], o: usize) -> Result<u32, String> {
    let s = b.get(o..o.checked_add(4).ok_or("overflow")?).ok_or("record truncated")?;
    Ok(u32::from_le_bytes(s.try_into().map_err(|_| "record truncated")?))
}

struct View<'a> {
    n: [usize; 7],
    sec: [&'a [u8]; NSEC],
}

impl<'a> View<'a> {
    fn parse(b: &'a [u8]) -> Result<View<'a>, String> {
        if b.len() < HEADER_LEN || b[0..4] != MAGIC {
            return Err("bad block header".into());
        }
        if rd_u32(b, 8)? as usize != b.len() {
            return Err("block length mismatch".into());
        }
        let mut n = [0usize; 7];
        for (k, c) in n.iter_mut().enumerate() {
            *c = rd_u32(b, 12 + 4 * k)? as usize;
        }
        let mut sec: [&[u8]; NSEC] = [&[]; NSEC];
        for (i, s) in sec.iter_mut().enumerate() {
            let off = rd_u32(b, TABLE_AT + 8 * i)? as usize;
            let len = rd_u32(b, TABLE_AT + 8 * i + 4)? as usize;
            if len == 0 {
                continue;
            }
            let end = off.checked_add(len).ok_or("section overflow")?;
            if off % 8 != 0 || end > b.len() || off < HEADER_LEN || len % ESIZE[i] != 0 {
                return Err(format!("section {i} invalid"));
            }
            *s = &b[off..end];
        }
        let v = View { n, sec };
        // Element counts must agree with the kind counts. Offset-table sections hold n + 1 entries.
        for k in 0..7 {
            let f = FIRST[k];
            let want = v.n[k];
            let ids_ok = v.sec[f].len() / 4 == want;
            let cols = match k {
                0..=3 => (f + 1..FIRST[k + 1]).all(|s| v.sec[s].len() / 8 == want),
                4 => v.sec[f + 1].len() / 4 == want + 1 && v.sec[f + 2].len() == v.sec[f + 3].len(),
                _ => v.sec[f + 1].len() / 4 == want + 1,
            };
            if !(ids_ok && cols) && want != 0 {
                return Err(format!("kind {k} sections disagree with its count"));
            }
            if want == 0 && !v.sec[f].is_empty() {
                return Err(format!("kind {k} has data but a zero count"));
            }
        }
        Ok(v)
    }
    fn id(&self, k: usize, i: usize) -> u32 {
        rd_u32(self.sec[FIRST[k]], 4 * i).unwrap_or(0)
    }
    fn f(&self, s: usize, i: usize) -> f64 {
        self.f_idx(s, i)
    }
    fn f_idx(&self, s: usize, i: usize) -> f64 {
        self.sec[s].get(8 * i..8 * i + 8).map_or(0.0, |c| f64::from_le_bytes(c.try_into().unwrap_or([0; 8])))
    }
    /// A monotone offset table of `n + 1` entries starting at 0 and ending at `limit`, as `usize`.
    fn offsets(&self, s: usize, n: usize, limit: usize) -> Result<Vec<usize>, String> {
        if n == 0 {
            return Ok(vec![0]);
        }
        let b = self.sec[s];
        let mut out = Vec::with_capacity(n + 1);
        let mut prev = 0usize;
        for i in 0..=n {
            let o = rd_u32(b, 4 * i)? as usize;
            if o < prev || o > limit || (i == 0 && o != 0) {
                return Err("offset table not monotone or out of range".into());
            }
            prev = o;
            out.push(o);
        }
        if prev != limit {
            return Err("offset table does not cover its arena".into());
        }
        Ok(out)
    }
}

fn touch_polylines(v: &View) -> Result<u64, String> {
    if v.n[4] == 0 {
        return Ok(0);
    }
    let f = FIRST[4];
    v.offsets(f + 1, v.n[4], v.sec[f + 2].len() / 8)?;
    Ok(sum_u32(v.sec[f]).wrapping_add(sum_f64(v.sec[f + 2])).wrapping_add(sum_f64(v.sec[f + 3])))
}

struct SplineRec<'a> {
    degree: u8,
    ncps: usize,
    nknots: usize,
    rational: bool,
    body: &'a [u8],
}

impl<'a> SplineRec<'a> {
    fn parse(rec: &'a [u8]) -> Result<Self, String> {
        let degree = *rec.first().ok_or("empty spline record")?;
        let rational = *rec.get(1).ok_or("truncated")? == 1;
        let ncps = rd_u32(rec, 4)? as usize;
        let nknots = rd_u32(rec, 8)? as usize;
        // Checked arithmetic: the counts come from the file.
        let words = ncps
            .checked_mul(2)
            .and_then(|x| x.checked_add(nknots))
            .and_then(|x| x.checked_add(if rational { ncps } else { 0 }))
            .ok_or("count overflow")?;
        let need = words.checked_mul(8).and_then(|x| x.checked_add(SPLINE_HEADER)).ok_or("size overflow")?;
        if need != rec.len() {
            return Err("spline record size disagrees with its header".into());
        }
        Ok(SplineRec { degree, ncps, nknots, rational, body: &rec[SPLINE_HEADER..] })
    }
    fn to_geom(&self) -> Geom {
        let f = |i: usize| f64::from_le_bytes(self.body[8 * i..8 * i + 8].try_into().unwrap_or([0; 8]));
        let cps = (0..self.ncps).map(|i| [f(2 * i), f(2 * i + 1)]).collect();
        let k0 = 2 * self.ncps;
        let knots = (0..self.nknots).map(|i| f(k0 + i)).collect();
        let weights = self.rational.then(|| (0..self.ncps).map(|i| f(k0 + self.nknots + i)).collect());
        Geom::Spline { degree: self.degree, cps, knots, weights }
    }
}

fn touch_splines(v: &View) -> Result<u64, String> {
    if v.n[5] == 0 {
        return Ok(0);
    }
    let f = FIRST[5];
    let arena = v.sec[f + 2];
    let offs = v.offsets(f + 1, v.n[5], arena.len())?;
    let mut sum = sum_u32(v.sec[f]);
    for w in offs.windows(2) {
        let rec = SplineRec::parse(&arena[w[0]..w[1]])?;
        sum = sum.wrapping_add(rec.degree as u64).wrapping_add(sum_f64(rec.body));
    }
    Ok(sum)
}

fn composite_geom(rec: &[u8]) -> Result<Geom, String> {
    let nparts = rd_u32(rec, 0)? as usize;
    let mut o = 8usize;
    let mut parts = Vec::new();
    for _ in 0..nparts {
        let tag = rd_u32(rec, o)?;
        let k = if tag == 0 {
            4
        } else if tag == 1 {
            5
        } else {
            return Err("bad part tag".into());
        };
        let body = rec.get(o + 8..o + 8 + 8 * k).ok_or("part truncated")?;
        let f = |i: usize| f64::from_le_bytes(body[8 * i..8 * i + 8].try_into().unwrap_or([0; 8]));
        parts.push(if tag == 0 {
            Geom::Line { a: [f(0), f(1)], b: [f(2), f(3)] }
        } else {
            Geom::Arc { c: [f(0), f(1)], r: f(2), a0: f(3), a1: f(4) }
        });
        o += 8 + 8 * k;
    }
    if o != rec.len() {
        return Err("composite record has trailing bytes".into());
    }
    Ok(Geom::Composite { parts })
}

fn touch_composites(v: &View) -> Result<u64, String> {
    if v.n[6] == 0 {
        return Ok(0);
    }
    let f = FIRST[6];
    let arena = v.sec[f + 2];
    let offs = v.offsets(f + 1, v.n[6], arena.len())?;
    let mut sum = sum_u32(v.sec[f]);
    for w in offs.windows(2) {
        let rec = &arena[w[0]..w[1]];
        let nparts = rd_u32(rec, 0)? as usize;
        let mut o = 8usize;
        for _ in 0..nparts {
            let tag = rd_u32(rec, o)?;
            let k = match tag {
                0 => 4,
                1 => 5,
                _ => return Err("bad part tag".into()),
            };
            let body = rec.get(o + 8..o + 8 + 8 * k).ok_or("part truncated")?;
            sum = sum.wrapping_add(tag as u64).wrapping_add(sum_f64(body));
            o += 8 + 8 * k;
        }
        if o != rec.len() {
            return Err("composite record has trailing bytes".into());
        }
    }
    Ok(sum)
}

/// Padding bytes in one spline record: the header pad (the arrays that follow are 8 aligned already).
pub fn spline_padding_bytes_per_curve() -> usize {
    SPLINE_HEADER_PAD
}

/// Padding in one encoded block: bytes that sit between sections or after the last one, as read
/// back from the block's own section table (not estimated).
pub fn section_padding_bytes(block: &[u8]) -> usize {
    let rd = |o: usize| {
        u32::from_le_bytes(block.get(o..o + 4).and_then(|s| s.try_into().ok()).unwrap_or([0; 4])) as usize
    };
    let mut secs: Vec<(usize, usize)> =
        (0..NSEC).map(|i| (rd(TABLE_AT + 8 * i), rd(TABLE_AT + 8 * i + 4))).filter(|s| s.1 > 0).collect();
    secs.sort_unstable();
    let mut end = HEADER_LEN;
    let mut pad = 0;
    for (o, l) in secs {
        pad += o.saturating_sub(end);
        end = o + l;
    }
    pad + block.len().saturating_sub(end)
}

/// Padding bytes inside a composite record: 4 after the part count and 4 after each part tag.
pub fn composite_padding_bytes(parts: usize) -> usize {
    4 + 4 * parts
}
