//! Candidate C: rkyv zero-copy archive of type-batched structure-of-arrays structs.
//!
//! Why this one. It is the Rust-native alternative to FlatBuffers: no schema compiler, no generated
//! code, the layout is the in-memory layout of the archived types, and `access` validates the buffer
//! with bytecheck before handing out references. It tests whether a Rust-first stack gains anything
//! over FlatBuffers for the same SoA shape. Its cost is that the format is defined by a Rust crate
//! version and not by a language-neutral schema, which matters for the browser reader (EPIC M).

use crate::common::Codec;
use pvec_fixtures::{Entity, Geom};
use rkyv::rancor::Error as RErr;
use rkyv::vec::ArchivedVec;
use rkyv::{Archive, Archived, Deserialize, Serialize};

#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CLines {
    ids: Vec<u32>,
    ax: Vec<f64>,
    ay: Vec<f64>,
    bx: Vec<f64>,
    by: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CCircles {
    ids: Vec<u32>,
    cx: Vec<f64>,
    cy: Vec<f64>,
    r: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CArcs {
    ids: Vec<u32>,
    cx: Vec<f64>,
    cy: Vec<f64>,
    r: Vec<f64>,
    a0: Vec<f64>,
    a1: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CEllipses {
    ids: Vec<u32>,
    cols: Vec<Vec<f64>>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CPolylines {
    ids: Vec<u32>,
    voff: Vec<u32>,
    xs: Vec<f64>,
    ys: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CSplines {
    ids: Vec<u32>,
    degree: Vec<u8>,
    cp_off: Vec<u32>,
    knot_off: Vec<u32>,
    w_off: Vec<u32>,
    cps: Vec<f64>,
    knots: Vec<f64>,
    weights: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CComposites {
    ids: Vec<u32>,
    part_off: Vec<u32>,
    data_off: Vec<u32>,
    tags: Vec<u8>,
    data: Vec<f64>,
}
#[derive(Archive, Serialize, Deserialize, Default)]
pub struct CBlock {
    lines: CLines,
    circles: CCircles,
    arcs: CArcs,
    ellipses: CEllipses,
    polylines: CPolylines,
    splines: CSplines,
    composites: CComposites,
}

/// Candidate C.
pub struct Rkyv;

fn to_cblock(ents: &[&Entity]) -> CBlock {
    let mut b = CBlock::default();
    b.ellipses.cols = vec![vec![]; 7];
    b.polylines.voff.push(0);
    b.splines.cp_off.push(0);
    b.splines.knot_off.push(0);
    b.splines.w_off.push(0);
    b.composites.part_off.push(0);
    b.composites.data_off.push(0);
    for e in ents {
        match &e.geom {
            Geom::Line { a, b: q } => {
                b.lines.ids.push(e.id);
                b.lines.ax.push(a[0]);
                b.lines.ay.push(a[1]);
                b.lines.bx.push(q[0]);
                b.lines.by.push(q[1]);
            }
            Geom::Circle { c, r } => {
                b.circles.ids.push(e.id);
                b.circles.cx.push(c[0]);
                b.circles.cy.push(c[1]);
                b.circles.r.push(*r);
            }
            Geom::Arc { c, r, a0, a1 } => {
                b.arcs.ids.push(e.id);
                b.arcs.cx.push(c[0]);
                b.arcs.cy.push(c[1]);
                b.arcs.r.push(*r);
                b.arcs.a0.push(*a0);
                b.arcs.a1.push(*a1);
            }
            Geom::Ellipse { c, a, b: bb, rot, t0, t1 } => {
                b.ellipses.ids.push(e.id);
                for (i, v) in [c[0], c[1], *a, *bb, *rot, *t0, *t1].into_iter().enumerate() {
                    b.ellipses.cols[i].push(v);
                }
            }
            Geom::Polyline { pts } => {
                b.polylines.ids.push(e.id);
                for p in pts {
                    b.polylines.xs.push(p[0]);
                    b.polylines.ys.push(p[1]);
                }
                b.polylines.voff.push(b.polylines.xs.len() as u32);
            }
            Geom::Spline { degree, cps, knots, weights } => {
                b.splines.ids.push(e.id);
                b.splines.degree.push(*degree);
                for p in cps {
                    b.splines.cps.push(p[0]);
                    b.splines.cps.push(p[1]);
                }
                b.splines.knots.extend_from_slice(knots);
                b.splines.weights.extend(weights.iter().flatten());
                b.splines.cp_off.push((b.splines.cps.len() / 2) as u32);
                b.splines.knot_off.push(b.splines.knots.len() as u32);
                b.splines.w_off.push(b.splines.weights.len() as u32);
            }
            Geom::Composite { parts } => {
                b.composites.ids.push(e.id);
                for p in parts {
                    match p {
                        Geom::Line { a, b: q } => {
                            b.composites.tags.push(0);
                            b.composites.data.extend_from_slice(&[a[0], a[1], q[0], q[1]]);
                        }
                        Geom::Arc { c, r, a0, a1 } => {
                            b.composites.tags.push(1);
                            b.composites.data.extend_from_slice(&[c[0], c[1], *r, *a0, *a1]);
                        }
                        _ => {}
                    }
                }
                b.composites.part_off.push(b.composites.tags.len() as u32);
                b.composites.data_off.push(b.composites.data.len() as u32);
            }
        }
    }
    b
}

fn offs(o: &[u32], n: usize, limit: usize) -> Result<Vec<usize>, String> {
    if o.len() != n + 1 {
        return Err("offset table length".into());
    }
    let v: Vec<usize> = o.iter().map(|&x| x as usize).collect();
    if v[0] != 0 || v[n] != limit || v.windows(2).any(|w| w[0] > w[1]) {
        return Err("offset table not monotone or not covering".into());
    }
    Ok(v)
}

fn same(n: usize, cols: &[&[f64]]) -> Result<(), String> {
    if cols.iter().all(|c| c.len() == n) { Ok(()) } else { Err("column length".into()) }
}

fn from_cblock(b: &CBlock) -> Result<Vec<Entity>, String> {
    let mut out = Vec::new();
    let l = &b.lines;
    same(l.ids.len(), &[&l.ax, &l.ay, &l.bx, &l.by])?;
    for i in 0..l.ids.len() {
        out.push(Entity { id: l.ids[i], geom: Geom::Line { a: [l.ax[i], l.ay[i]], b: [l.bx[i], l.by[i]] } });
    }
    let c = &b.circles;
    same(c.ids.len(), &[&c.cx, &c.cy, &c.r])?;
    for i in 0..c.ids.len() {
        out.push(Entity { id: c.ids[i], geom: Geom::Circle { c: [c.cx[i], c.cy[i]], r: c.r[i] } });
    }
    let a = &b.arcs;
    same(a.ids.len(), &[&a.cx, &a.cy, &a.r, &a.a0, &a.a1])?;
    for i in 0..a.ids.len() {
        out.push(Entity {
            id: a.ids[i],
            geom: Geom::Arc { c: [a.cx[i], a.cy[i]], r: a.r[i], a0: a.a0[i], a1: a.a1[i] },
        });
    }
    let e = &b.ellipses;
    if !e.ids.is_empty() {
        if e.cols.len() != 7 {
            return Err("ellipse columns".into());
        }
        let cols: Vec<&[f64]> = e.cols.iter().map(|v| v.as_slice()).collect();
        same(e.ids.len(), &cols)?;
        for i in 0..e.ids.len() {
            out.push(Entity {
                id: e.ids[i],
                geom: Geom::Ellipse {
                    c: [cols[0][i], cols[1][i]],
                    a: cols[2][i],
                    b: cols[3][i],
                    rot: cols[4][i],
                    t0: cols[5][i],
                    t1: cols[6][i],
                },
            });
        }
    }
    let p = &b.polylines;
    if !p.ids.is_empty() {
        same(p.xs.len(), &[&p.ys])?;
        let vo = offs(&p.voff, p.ids.len(), p.xs.len())?;
        for i in 0..p.ids.len() {
            out.push(Entity {
                id: p.ids[i],
                geom: Geom::Polyline { pts: (vo[i]..vo[i + 1]).map(|j| [p.xs[j], p.ys[j]]).collect() },
            });
        }
    }
    let s = &b.splines;
    if !s.ids.is_empty() {
        let n = s.ids.len();
        if s.degree.len() != n || s.cps.len() % 2 != 0 {
            return Err("spline columns".into());
        }
        let co = offs(&s.cp_off, n, s.cps.len() / 2)?;
        let ko = offs(&s.knot_off, n, s.knots.len())?;
        let wo = offs(&s.w_off, n, s.weights.len())?;
        for i in 0..n {
            out.push(Entity {
                id: s.ids[i],
                geom: Geom::Spline {
                    degree: s.degree[i],
                    cps: (co[i]..co[i + 1]).map(|j| [s.cps[2 * j], s.cps[2 * j + 1]]).collect(),
                    knots: s.knots[ko[i]..ko[i + 1]].to_vec(),
                    weights: (wo[i + 1] > wo[i]).then(|| s.weights[wo[i]..wo[i + 1]].to_vec()),
                },
            });
        }
    }
    let k = &b.composites;
    if !k.ids.is_empty() {
        let n = k.ids.len();
        let po = offs(&k.part_off, n, k.tags.len())?;
        let dof = offs(&k.data_off, n, k.data.len())?;
        for i in 0..n {
            let mut d = dof[i];
            let mut parts = Vec::new();
            for &t in &k.tags[po[i]..po[i + 1]] {
                let w = if t == 0 { 4 } else { 5 };
                let v = k.data.get(d..d + w).ok_or("composite data truncated")?;
                parts.push(if t == 0 {
                    Geom::Line { a: [v[0], v[1]], b: [v[2], v[3]] }
                } else {
                    Geom::Arc { c: [v[0], v[1]], r: v[2], a0: v[3], a1: v[4] }
                });
                d += w;
            }
            out.push(Entity { id: k.ids[i], geom: Geom::Composite { parts } });
        }
    }
    Ok(out)
}

#[inline(never)]
fn sf(v: &ArchivedVec<Archived<f64>>) -> u64 {
    v.iter().fold(0u64, |a, x| a.wrapping_add(x.to_native().to_bits()))
}
#[inline(never)]
fn si(v: &ArchivedVec<Archived<u32>>) -> u64 {
    v.iter().fold(0u64, |a, x| a.wrapping_add(x.to_native() as u64))
}
#[inline(never)]
fn sb(v: &ArchivedVec<u8>) -> u64 {
    v.iter().fold(0u64, |a, x| a.wrapping_add(*x as u64))
}

impl Codec for Rkyv {
    const NAME: &'static str = "C rkyv SoA";

    fn encode_block(ents: &[&Entity]) -> Vec<u8> {
        match rkyv::to_bytes::<RErr>(&to_cblock(ents)) {
            Ok(v) => v.to_vec(),
            Err(_) => Vec::new(),
        }
    }

    #[inline(never)]
    fn touch(block: &[u8]) -> Result<u64, String> {
        let a = rkyv::access::<ArchivedCBlock, RErr>(block).map_err(|e| e.to_string())?;
        let mut s = 0u64;
        let l = &a.lines;
        s = s
            .wrapping_add(si(&l.ids))
            .wrapping_add(sf(&l.ax))
            .wrapping_add(sf(&l.ay))
            .wrapping_add(sf(&l.bx))
            .wrapping_add(sf(&l.by));
        let c = &a.circles;
        s = s.wrapping_add(si(&c.ids)).wrapping_add(sf(&c.cx)).wrapping_add(sf(&c.cy)).wrapping_add(sf(&c.r));
        let r = &a.arcs;
        s = s
            .wrapping_add(si(&r.ids))
            .wrapping_add(sf(&r.cx))
            .wrapping_add(sf(&r.cy))
            .wrapping_add(sf(&r.r))
            .wrapping_add(sf(&r.a0))
            .wrapping_add(sf(&r.a1));
        let e = &a.ellipses;
        s = s.wrapping_add(si(&e.ids));
        for col in e.cols.iter() {
            s = s.wrapping_add(sf(col));
        }
        let p = &a.polylines;
        s = s.wrapping_add(si(&p.ids)).wrapping_add(sf(&p.xs)).wrapping_add(sf(&p.ys));
        let q = &a.splines;
        s = s
            .wrapping_add(si(&q.ids))
            .wrapping_add(sb(&q.degree))
            .wrapping_add(sf(&q.cps))
            .wrapping_add(sf(&q.knots))
            .wrapping_add(sf(&q.weights));
        let k = &a.composites;
        s = s.wrapping_add(si(&k.ids)).wrapping_add(sb(&k.tags)).wrapping_add(sf(&k.data));
        Ok(s)
    }

    fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
        let a = rkyv::access::<ArchivedCBlock, RErr>(block).map_err(|e| e.to_string())?;
        let b: CBlock = rkyv::deserialize::<CBlock, RErr>(a).map_err(|e| e.to_string())?;
        from_cblock(&b)
    }
}
