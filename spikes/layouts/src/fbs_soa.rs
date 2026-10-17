//! Candidate A: FlatBuffers block, type-batched structure of arrays (schema/pvec_soa.fbs).
//! Reads go through the generated verifier, so hostile input is checked by the library.

use crate::common::Codec;
use crate::soa_gen::soa;
use flatbuffers::{FlatBufferBuilder, Vector, WIPOffset};
use pvec_fixtures::{Entity, Geom};

type Fb<'a> = FlatBufferBuilder<'a>;

fn v32<'a>(b: &mut Fb<'a>, v: &[u32]) -> Option<WIPOffset<Vector<'a, u32>>> {
    Some(b.create_vector(v))
}
fn v64<'a>(b: &mut Fb<'a>, v: &[f64]) -> Option<WIPOffset<Vector<'a, f64>>> {
    Some(b.create_vector(v))
}
fn v8<'a>(b: &mut Fb<'a>, v: &[u8]) -> Option<WIPOffset<Vector<'a, u8>>> {
    Some(b.create_vector(v))
}

/// Candidate A, reading with the idiomatic generated iterators.
pub struct FbSoa;
/// Candidate A, reading the same bytes through the vectors' raw byte view.
pub struct FbSoaRaw;

fn encode(ents: &[&Entity]) -> Vec<u8> {
    let mut fbb = FlatBufferBuilder::with_capacity(ents.len() * 64);
    let pick = |t: u8| ents.iter().filter(move |e| e.geom.tag() == t).copied().collect::<Vec<_>>();
    let ids = |es: &[&Entity]| es.iter().map(|e| e.id).collect::<Vec<u32>>();
    let (l, c, a, e, p, s, k) = (pick(0), pick(1), pick(2), pick(3), pick(4), pick(5), pick(6));

    let lines = (!l.is_empty()).then(|| {
        let col = |i: usize| {
            l.iter()
                .map(|e| match &e.geom {
                    Geom::Line { a, b } => [a[0], a[1], b[0], b[1]][i],
                    _ => 0.0,
                })
                .collect::<Vec<f64>>()
        };
        let (idv, ax, ay, bx, by) = (ids(&l), col(0), col(1), col(2), col(3));
        let args = soa::LineBatchArgs {
            ids: v32(&mut fbb, &idv),
            ax: v64(&mut fbb, &ax),
            ay: v64(&mut fbb, &ay),
            bx: v64(&mut fbb, &bx),
            by: v64(&mut fbb, &by),
        };
        soa::LineBatch::create(&mut fbb, &args)
    });
    let circles = (!c.is_empty()).then(|| {
        let col = |i: usize| {
            c.iter()
                .map(|e| match &e.geom {
                    Geom::Circle { c, r } => [c[0], c[1], *r][i],
                    _ => 0.0,
                })
                .collect::<Vec<f64>>()
        };
        let (idv, cx, cy, r) = (ids(&c), col(0), col(1), col(2));
        let args = soa::CircleBatchArgs {
            ids: v32(&mut fbb, &idv),
            cx: v64(&mut fbb, &cx),
            cy: v64(&mut fbb, &cy),
            r: v64(&mut fbb, &r),
        };
        soa::CircleBatch::create(&mut fbb, &args)
    });
    let arcs = (!a.is_empty()).then(|| {
        let col = |i: usize| {
            a.iter()
                .map(|e| match &e.geom {
                    Geom::Arc { c, r, a0, a1 } => [c[0], c[1], *r, *a0, *a1][i],
                    _ => 0.0,
                })
                .collect::<Vec<f64>>()
        };
        let (idv, cx, cy, r, a0, a1) = (ids(&a), col(0), col(1), col(2), col(3), col(4));
        let args = soa::ArcBatchArgs {
            ids: v32(&mut fbb, &idv),
            cx: v64(&mut fbb, &cx),
            cy: v64(&mut fbb, &cy),
            r: v64(&mut fbb, &r),
            a0: v64(&mut fbb, &a0),
            a1: v64(&mut fbb, &a1),
        };
        soa::ArcBatch::create(&mut fbb, &args)
    });
    let ellipses = (!e.is_empty()).then(|| {
        let col = |i: usize| {
            e.iter()
                .map(|e| match &e.geom {
                    Geom::Ellipse { c, a, b, rot, t0, t1 } => [c[0], c[1], *a, *b, *rot, *t0, *t1][i],
                    _ => 0.0,
                })
                .collect::<Vec<f64>>()
        };
        let idv = ids(&e);
        let cols: Vec<Vec<f64>> = (0..7).map(col).collect();
        let args = soa::EllipseBatchArgs {
            ids: v32(&mut fbb, &idv),
            cx: v64(&mut fbb, &cols[0]),
            cy: v64(&mut fbb, &cols[1]),
            a: v64(&mut fbb, &cols[2]),
            b: v64(&mut fbb, &cols[3]),
            rot: v64(&mut fbb, &cols[4]),
            t0: v64(&mut fbb, &cols[5]),
            t1: v64(&mut fbb, &cols[6]),
        };
        soa::EllipseBatch::create(&mut fbb, &args)
    });
    let polylines = (!p.is_empty()).then(|| {
        let (mut voff, mut xs, mut ys) = (vec![0u32], vec![], vec![]);
        for e in &p {
            if let Geom::Polyline { pts } = &e.geom {
                for q in pts {
                    xs.push(q[0]);
                    ys.push(q[1]);
                }
            }
            voff.push(xs.len() as u32);
        }
        let idv = ids(&p);
        let args = soa::PolylineBatchArgs {
            ids: v32(&mut fbb, &idv),
            voff: v32(&mut fbb, &voff),
            xs: v64(&mut fbb, &xs),
            ys: v64(&mut fbb, &ys),
        };
        soa::PolylineBatch::create(&mut fbb, &args)
    });
    let splines = (!s.is_empty()).then(|| {
        let (mut degree, mut cp_off, mut knot_off, mut w_off) = (vec![], vec![0u32], vec![0u32], vec![0u32]);
        let (mut cps, mut knots, mut weights) = (vec![], vec![], vec![]);
        for e in &s {
            if let Geom::Spline { degree: d, cps: c, knots: k, weights: w } = &e.geom {
                degree.push(*d);
                for q in c {
                    cps.push(q[0]);
                    cps.push(q[1]);
                }
                knots.extend_from_slice(k);
                weights.extend(w.iter().flatten());
            }
            cp_off.push((cps.len() / 2) as u32);
            knot_off.push(knots.len() as u32);
            w_off.push(weights.len() as u32);
        }
        let idv = ids(&s);
        let args = soa::SplineBatchArgs {
            ids: v32(&mut fbb, &idv),
            degree: v8(&mut fbb, &degree),
            cp_off: v32(&mut fbb, &cp_off),
            knot_off: v32(&mut fbb, &knot_off),
            w_off: v32(&mut fbb, &w_off),
            cps: v64(&mut fbb, &cps),
            knots: v64(&mut fbb, &knots),
            weights: v64(&mut fbb, &weights),
        };
        soa::SplineBatch::create(&mut fbb, &args)
    });
    let composites = (!k.is_empty()).then(|| {
        let (mut part_off, mut data_off, mut tags, mut data) = (vec![0u32], vec![0u32], vec![], vec![]);
        for e in &k {
            if let Geom::Composite { parts } = &e.geom {
                for q in parts {
                    match q {
                        Geom::Line { a, b } => {
                            tags.push(0u8);
                            data.extend_from_slice(&[a[0], a[1], b[0], b[1]]);
                        }
                        Geom::Arc { c, r, a0, a1 } => {
                            tags.push(1u8);
                            data.extend_from_slice(&[c[0], c[1], *r, *a0, *a1]);
                        }
                        _ => {}
                    }
                }
            }
            part_off.push(tags.len() as u32);
            data_off.push(data.len() as u32);
        }
        let idv = ids(&k);
        let args = soa::CompositeBatchArgs {
            ids: v32(&mut fbb, &idv),
            part_off: v32(&mut fbb, &part_off),
            data_off: v32(&mut fbb, &data_off),
            tags: v8(&mut fbb, &tags),
            data: v64(&mut fbb, &data),
        };
        soa::CompositeBatch::create(&mut fbb, &args)
    });
    let root = soa::Block::create(
        &mut fbb,
        &soa::BlockArgs { lines, circles, arcs, ellipses, polylines, splines, composites },
    );
    soa::finish_block_buffer(&mut fbb, root);
    fbb.finished_data().to_vec()
}

#[inline(never)]
fn sum_iter(v: Option<Vector<'_, f64>>) -> u64 {
    v.map_or(0, |v| v.iter().fold(0u64, |a, x| a.wrapping_add(x.to_bits())))
}
#[inline(never)]
fn sum_ids(v: Option<Vector<'_, u32>>) -> u64 {
    v.map_or(0, |v| v.iter().fold(0u64, |a, x| a.wrapping_add(x as u64)))
}
#[inline(never)]
fn sum_raw(v: Option<Vector<'_, f64>>) -> u64 {
    v.map_or(0, |v| {
        v.bytes()
            .chunks_exact(8)
            .fold(0u64, |a, c| a.wrapping_add(u64::from_le_bytes(c.try_into().unwrap_or([0; 8]))))
    })
}

#[inline(never)]
fn touch_with(block: &[u8], f64s: fn(Option<Vector<'_, f64>>) -> u64) -> Result<u64, String> {
    let b = soa::root_as_block(block).map_err(|e| e.to_string())?;
    let mut s = 0u64;
    if let Some(x) = b.lines() {
        s = s
            .wrapping_add(sum_ids(x.ids()))
            .wrapping_add(f64s(x.ax()))
            .wrapping_add(f64s(x.ay()))
            .wrapping_add(f64s(x.bx()))
            .wrapping_add(f64s(x.by()));
    }
    if let Some(x) = b.circles() {
        s = s
            .wrapping_add(sum_ids(x.ids()))
            .wrapping_add(f64s(x.cx()))
            .wrapping_add(f64s(x.cy()))
            .wrapping_add(f64s(x.r()));
    }
    if let Some(x) = b.arcs() {
        s = s
            .wrapping_add(sum_ids(x.ids()))
            .wrapping_add(f64s(x.cx()))
            .wrapping_add(f64s(x.cy()))
            .wrapping_add(f64s(x.r()))
            .wrapping_add(f64s(x.a0()))
            .wrapping_add(f64s(x.a1()));
    }
    if let Some(x) = b.ellipses() {
        s = s
            .wrapping_add(sum_ids(x.ids()))
            .wrapping_add(f64s(x.cx()))
            .wrapping_add(f64s(x.cy()))
            .wrapping_add(f64s(x.a()))
            .wrapping_add(f64s(x.b()))
            .wrapping_add(f64s(x.rot()))
            .wrapping_add(f64s(x.t0()))
            .wrapping_add(f64s(x.t1()));
    }
    if let Some(x) = b.polylines() {
        s = s.wrapping_add(sum_ids(x.ids())).wrapping_add(f64s(x.xs())).wrapping_add(f64s(x.ys()));
    }
    if let Some(x) = b.splines() {
        s = s
            .wrapping_add(sum_ids(x.ids()))
            .wrapping_add(f64s(x.cps()))
            .wrapping_add(f64s(x.knots()))
            .wrapping_add(f64s(x.weights()));
        if let Some(d) = x.degree() {
            s = s.wrapping_add(d.iter().map(|x| x as u64).sum::<u64>());
        }
    }
    if let Some(x) = b.composites() {
        s = s.wrapping_add(sum_ids(x.ids())).wrapping_add(f64s(x.data()));
        if let Some(t) = x.tags() {
            s = s.wrapping_add(t.iter().map(|x| x as u64).sum::<u64>());
        }
    }
    Ok(s)
}

fn offs(v: Option<Vector<'_, u32>>, n: usize, limit: usize) -> Result<Vec<usize>, String> {
    let v = v.ok_or("missing offsets")?;
    if v.len() != n + 1 {
        return Err("offset table length".into());
    }
    let o: Vec<usize> = v.iter().map(|x| x as usize).collect();
    if o[0] != 0 || o[n] != limit || o.windows(2).any(|w| w[0] > w[1]) {
        return Err("offset table not monotone or not covering".into());
    }
    Ok(o)
}

fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
    let b = soa::root_as_block(block).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let get = |v: Option<Vector<'_, f64>>, n: usize| -> Result<Vec<f64>, String> {
        let v = v.ok_or("missing column")?;
        if v.len() != n {
            return Err("column length".into());
        }
        Ok(v.iter().collect())
    };
    let ids = |v: Option<Vector<'_, u32>>| -> Vec<u32> { v.map(|v| v.iter().collect()).unwrap_or_default() };
    if let Some(x) = b.lines() {
        let id = ids(x.ids());
        let n = id.len();
        let (ax, ay, bx, by) = (get(x.ax(), n)?, get(x.ay(), n)?, get(x.bx(), n)?, get(x.by(), n)?);
        for i in 0..n {
            out.push(Entity { id: id[i], geom: Geom::Line { a: [ax[i], ay[i]], b: [bx[i], by[i]] } });
        }
    }
    if let Some(x) = b.circles() {
        let id = ids(x.ids());
        let n = id.len();
        let (cx, cy, r) = (get(x.cx(), n)?, get(x.cy(), n)?, get(x.r(), n)?);
        for i in 0..n {
            out.push(Entity { id: id[i], geom: Geom::Circle { c: [cx[i], cy[i]], r: r[i] } });
        }
    }
    if let Some(x) = b.arcs() {
        let id = ids(x.ids());
        let n = id.len();
        let (cx, cy, r, a0, a1) =
            (get(x.cx(), n)?, get(x.cy(), n)?, get(x.r(), n)?, get(x.a0(), n)?, get(x.a1(), n)?);
        for i in 0..n {
            out.push(Entity {
                id: id[i],
                geom: Geom::Arc { c: [cx[i], cy[i]], r: r[i], a0: a0[i], a1: a1[i] },
            });
        }
    }
    if let Some(x) = b.ellipses() {
        let id = ids(x.ids());
        let n = id.len();
        let c = [
            get(x.cx(), n)?,
            get(x.cy(), n)?,
            get(x.a(), n)?,
            get(x.b(), n)?,
            get(x.rot(), n)?,
            get(x.t0(), n)?,
            get(x.t1(), n)?,
        ];
        for i in 0..n {
            out.push(Entity {
                id: id[i],
                geom: Geom::Ellipse {
                    c: [c[0][i], c[1][i]],
                    a: c[2][i],
                    b: c[3][i],
                    rot: c[4][i],
                    t0: c[5][i],
                    t1: c[6][i],
                },
            });
        }
    }
    if let Some(x) = b.polylines() {
        let id = ids(x.ids());
        let n = id.len();
        let xs: Vec<f64> = x.xs().map(|v| v.iter().collect()).unwrap_or_default();
        let ys: Vec<f64> = x.ys().map(|v| v.iter().collect()).unwrap_or_default();
        if xs.len() != ys.len() {
            return Err("xs/ys length".into());
        }
        let vo = offs(x.voff(), n, xs.len())?;
        for i in 0..n {
            out.push(Entity {
                id: id[i],
                geom: Geom::Polyline { pts: (vo[i]..vo[i + 1]).map(|j| [xs[j], ys[j]]).collect() },
            });
        }
    }
    if let Some(x) = b.splines() {
        let id = ids(x.ids());
        let n = id.len();
        let cps: Vec<f64> = x.cps().map(|v| v.iter().collect()).unwrap_or_default();
        let knots: Vec<f64> = x.knots().map(|v| v.iter().collect()).unwrap_or_default();
        let wts: Vec<f64> = x.weights().map(|v| v.iter().collect()).unwrap_or_default();
        let deg: Vec<u8> = x.degree().map(|v| v.iter().collect()).unwrap_or_default();
        if deg.len() != n || cps.len() % 2 != 0 {
            return Err("spline columns".into());
        }
        let co = offs(x.cp_off(), n, cps.len() / 2)?;
        let ko = offs(x.knot_off(), n, knots.len())?;
        let wo = offs(x.w_off(), n, wts.len())?;
        for i in 0..n {
            let weights = (wo[i + 1] > wo[i]).then(|| wts[wo[i]..wo[i + 1]].to_vec());
            out.push(Entity {
                id: id[i],
                geom: Geom::Spline {
                    degree: deg[i],
                    cps: (co[i]..co[i + 1]).map(|j| [cps[2 * j], cps[2 * j + 1]]).collect(),
                    knots: knots[ko[i]..ko[i + 1]].to_vec(),
                    weights,
                },
            });
        }
    }
    if let Some(x) = b.composites() {
        let id = ids(x.ids());
        let n = id.len();
        let tags: Vec<u8> = x.tags().map(|v| v.iter().collect()).unwrap_or_default();
        let data: Vec<f64> = x.data().map(|v| v.iter().collect()).unwrap_or_default();
        let po = offs(x.part_off(), n, tags.len())?;
        let dof = offs(x.data_off(), n, data.len())?;
        for i in 0..n {
            let mut d = dof[i];
            let mut parts = Vec::new();
            for &t in &tags[po[i]..po[i + 1]] {
                let k = if t == 0 { 4 } else { 5 };
                let v = data.get(d..d + k).ok_or("composite data truncated")?;
                parts.push(if t == 0 {
                    Geom::Line { a: [v[0], v[1]], b: [v[2], v[3]] }
                } else {
                    Geom::Arc { c: [v[0], v[1]], r: v[2], a0: v[3], a1: v[4] }
                });
                d += k;
            }
            out.push(Entity { id: id[i], geom: Geom::Composite { parts } });
        }
    }
    Ok(out)
}

impl Codec for FbSoa {
    const NAME: &'static str = "A flatbuffers SoA";
    fn encode_block(ents: &[&Entity]) -> Vec<u8> {
        encode(ents)
    }
    fn touch(block: &[u8]) -> Result<u64, String> {
        touch_with(block, sum_iter)
    }
    fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
        decode(block)
    }
}

impl Codec for FbSoaRaw {
    const NAME: &'static str = "A flatbuffers SoA (raw byte view)";
    fn encode_block(ents: &[&Entity]) -> Vec<u8> {
        encode(ents)
    }
    fn touch(block: &[u8]) -> Result<u64, String> {
        touch_with(block, sum_raw)
    }
    fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
        decode(block)
    }
}
