//! H1: one HDF5 table per EXPRESS entity type, one row per instance, references as row indices,
//! lists as variable-length arrays. This follows the shape of the ISO/TS 10303-26 mapping as I
//! understand it, built from the standard's published description of the idea and not from its
//! text (the Technical Specification is not in this repository and I did not have it). It is an
//! "HDF5-backed representation of the fixture set", which is what the issue asks me to measure; no
//! conformance to Part 26 is claimed.
#![allow(missing_docs)]

use crate::schema::*;
use crate::util::*;
use hdf5::types::VarLenArray;
use hdf5::{Dataset, File, H5Type};
use hdf5_metno as hdf5;
use pvec_fixtures::{Drawing, Entity, Geom, overlaps};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Tables {
    pub points: Vec<Point>,
    pub dirs: Vec<Direction>,
    pub vecs: Vec<Vector>,
    pub places: Vec<Placement>,
    pub lines: Vec<LineRow>,
    pub circles: Vec<CircleRow>,
    pub ellipses: Vec<EllipseRow>,
    pub trims: Vec<TrimRow>,
    pub polys: Vec<PolyRow>,
    pub splines: Vec<SplineRow>,
    pub segs: Vec<SegRow>,
    pub comps: Vec<CompRow>,
    pub roots: Vec<Root>,
    pub bbox: Vec<f64>,
}

fn pt(t: &mut Tables, p: [f64; 2]) -> u32 {
    t.points.push(Point { x: p[0], y: p[1] });
    (t.points.len() - 1) as u32
}

fn dir(t: &mut Tables, d: [f64; 2]) -> u32 {
    t.dirs.push(Direction { dx: d[0], dy: d[1] });
    (t.dirs.len() - 1) as u32
}

fn placement(t: &mut Tables, c: [f64; 2], rot: f64) -> u32 {
    let (loc, d) = (pt(t, c), dir(t, [rot.cos(), rot.sin()]));
    t.places.push(Placement { loc, dir: d });
    (t.places.len() - 1) as u32
}

fn trim(t: &mut Tables, basis_tag: u8, basis: u32, t1: f64, t2: f64) -> u32 {
    t.trims.push(TrimRow { basis_tag, basis, t1, t2, sense: 1 });
    (t.trims.len() - 1) as u32
}

/// Adds the curve for `g` and returns (root tag, row).
fn curve(t: &mut Tables, g: &Geom) -> (u8, u32) {
    match g {
        Geom::Line { a, b } => {
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let p = pt(t, *a);
            let d = dir(t, [(b[0] - a[0]) / len, (b[1] - a[1]) / len]);
            t.vecs.push(Vector { dir: d, mag: 1.0 });
            t.lines.push(LineRow { pnt: p, vec: (t.vecs.len() - 1) as u32 });
            let l = (t.lines.len() - 1) as u32;
            (tag::TRIM, trim(t, tag::LINE, l, 0.0, len))
        }
        Geom::Circle { c, r } => {
            let pl = placement(t, *c, 0.0);
            t.circles.push(CircleRow { pl, r: *r });
            (tag::CIRCLE, (t.circles.len() - 1) as u32)
        }
        Geom::Arc { c, r, a0, a1 } => {
            let pl = placement(t, *c, 0.0);
            t.circles.push(CircleRow { pl, r: *r });
            let ci = (t.circles.len() - 1) as u32;
            (tag::TRIM, trim(t, tag::CIRCLE, ci, *a0, *a1))
        }
        Geom::Ellipse { c, a, b, rot, t0, t1 } => {
            let pl = placement(t, *c, *rot);
            t.ellipses.push(EllipseRow { pl, a: *a, b: *b });
            let e = (t.ellipses.len() - 1) as u32;
            (tag::TRIM, trim(t, tag::ELLIPSE, e, *t0, *t1))
        }
        Geom::Polyline { pts } => {
            let refs: Vec<u32> = pts.iter().map(|p| pt(t, *p)).collect();
            t.polys.push(PolyRow { pts: VarLenArray::from_slice(&refs) });
            (tag::POLY, (t.polys.len() - 1) as u32)
        }
        Geom::Spline { degree, cps, knots, weights } => {
            let refs: Vec<u32> = cps.iter().map(|p| pt(t, *p)).collect();
            let (mut mults, mut distinct) = (Vec::<u32>::new(), Vec::<f64>::new());
            for &k in knots {
                if distinct.last() == Some(&k) {
                    *mults.last_mut().unwrap() += 1;
                } else {
                    distinct.push(k);
                    mults.push(1);
                }
            }
            let w = weights.clone().unwrap_or_default();
            t.splines.push(SplineRow {
                degree: *degree,
                rational: weights.is_some() as u8,
                cps: VarLenArray::from_slice(&refs),
                mults: VarLenArray::from_slice(&mults),
                knots: VarLenArray::from_slice(&distinct),
                weights: VarLenArray::from_slice(&w),
            });
            (tag::SPLINE, (t.splines.len() - 1) as u32)
        }
        Geom::Composite { parts } => {
            let mut segs = Vec::new();
            for p in parts {
                let (pt_tag, row) = curve(t, p);
                t.segs.push(SegRow { parent_tag: pt_tag, parent: row, same_sense: 1, transition: 1 });
                segs.push((t.segs.len() - 1) as u32);
            }
            t.comps.push(CompRow { segs: VarLenArray::from_slice(&segs) });
            (tag::COMP, (t.comps.len() - 1) as u32)
        }
    }
}

pub fn build(d: &Drawing) -> Tables {
    let mut t = Tables::default();
    for e in &d.entities {
        let (tg, row) = curve(&mut t, &e.geom);
        t.roots.push(Root { tag: tg, row });
        t.bbox.extend(e.geom.bbox());
    }
    t
}

pub fn write(d: &Drawing, path: &str, cfg: &Cfg) -> hdf5::Result<()> {
    let t = build(d);
    let f = create_file(path, cfg)?;
    let g = f.create_group("data")?;
    put1(&g, "CARTESIAN_POINT", &t.points, cfg)?;
    put1(&g, "DIRECTION", &t.dirs, cfg)?;
    put1(&g, "VECTOR", &t.vecs, cfg)?;
    put1(&g, "AXIS2_PLACEMENT_2D", &t.places, cfg)?;
    put1(&g, "LINE", &t.lines, cfg)?;
    put1(&g, "CIRCLE", &t.circles, cfg)?;
    put1(&g, "ELLIPSE", &t.ellipses, cfg)?;
    put1(&g, "TRIMMED_CURVE", &t.trims, cfg)?;
    put1(&g, "POLYLINE", &t.polys, cfg)?;
    put1(&g, "B_SPLINE_CURVE_WITH_KNOTS", &t.splines, cfg)?;
    put1(&g, "COMPOSITE_CURVE_SEGMENT", &t.segs, cfg)?;
    put1(&g, "COMPOSITE_CURVE", &t.comps, cfg)?;
    put1(&g, "GEOMETRIC_CURVE_SET", &t.roots, cfg)?;
    // Not part of the entity mapping: a bounds table, so a viewport query has something to scan.
    let ext = f.create_group("ext")?;
    put2(&ext, "bbox", 4, &t.bbox, cfg)?;
    f.flush()?;
    Ok(())
}

/// Logical payload bytes (fixed-size parts as laid out in memory, plus the elements the
/// variable-length arrays point at), for the overhead fraction.
pub fn payload_bytes(d: &Drawing) -> u64 {
    let t = build(d);
    let n = |len: usize, sz: usize| (len * sz) as u64;
    let mut b = n(t.points.len(), 16)
        + n(t.dirs.len(), 16)
        + n(t.vecs.len(), 16)
        + n(t.places.len(), 8)
        + n(t.lines.len(), 8)
        + n(t.circles.len(), 16);
    b += n(t.ellipses.len(), 24)
        + n(t.trims.len(), 32)
        + n(t.segs.len(), 12)
        + n(t.roots.len(), 8)
        + n(t.bbox.len(), 8);
    for p in &t.polys {
        b += p.pts.len() as u64 * 4;
    }
    for s in &t.splines {
        b += 2
            + s.cps.len() as u64 * 4
            + s.mults.len() as u64 * 4
            + (s.knots.len() + s.weights.len()) as u64 * 8;
    }
    for c in &t.comps {
        b += c.segs.len() as u64 * 4;
    }
    b
}

// ------------------------------------------------------------------------------------- reading

/// Rows of one table: either every row (dense) or the rows asked for (sparse).
pub struct Tab<T> {
    dense: Vec<T>,
    sparse: BTreeMap<u32, T>,
}

impl<T> Tab<T> {
    fn new() -> Self {
        Tab { dense: Vec::new(), sparse: BTreeMap::new() }
    }
    fn get(&self, i: u32) -> &T {
        if self.dense.is_empty() { &self.sparse[&i] } else { &self.dense[i as usize] }
    }
}

fn ds(f: &File, name: &str) -> Option<Dataset> {
    f.dataset(&format!("data/{name}")).ok()
}

fn read_rows<T: H5Type>(d: &Option<Dataset>, rows: &[u32]) -> Tab<T> {
    let mut tab = Tab::new();
    let Some(d) = d else { return tab };
    for (a, b) in crate::h2::runs(rows) {
        let v = d.read_slice_1d::<T, _>(a..b).expect("read");
        for (i, row) in v.into_iter().enumerate() {
            tab.sparse.insert((a + i) as u32, row);
        }
    }
    tab
}

fn read_dense<T: H5Type>(d: &Option<Dataset>) -> Tab<T> {
    let mut tab = Tab::new();
    if let Some(d) = d {
        tab.dense = d.read_raw::<T>().expect("read");
    }
    tab
}

struct Store {
    points: Tab<Point>,
    dirs: Tab<Direction>,
    vecs: Tab<Vector>,
    places: Tab<Placement>,
    lines: Tab<LineRow>,
    circles: Tab<CircleRow>,
    ellipses: Tab<EllipseRow>,
    trims: Tab<TrimRow>,
    polys: Tab<PolyRow>,
    splines: Tab<SplineRow>,
    segs: Tab<SegRow>,
    comps: Tab<CompRow>,
}

fn rebuild(s: &Store, root: Root) -> Geom {
    match root.tag {
        tag::CIRCLE => {
            let c = s.circles.get(root.row);
            let p = s.places.get(c.pl);
            let q = s.points.get(p.loc);
            Geom::Circle { c: [q.x, q.y], r: c.r }
        }
        tag::TRIM => {
            let t = s.trims.get(root.row);
            match t.basis_tag {
                tag::LINE => {
                    let l = s.lines.get(t.basis);
                    let (a, v) = (s.points.get(l.pnt), s.vecs.get(l.vec));
                    let d = s.dirs.get(v.dir);
                    let (x, y) = (v.mag * d.dx, v.mag * d.dy);
                    Geom::Line { a: [a.x + t.t1 * x, a.y + t.t1 * y], b: [a.x + t.t2 * x, a.y + t.t2 * y] }
                }
                tag::CIRCLE => {
                    let c = s.circles.get(t.basis);
                    let q = s.points.get(s.places.get(c.pl).loc);
                    Geom::Arc { c: [q.x, q.y], r: c.r, a0: t.t1, a1: t.t2 }
                }
                _ => {
                    let e = s.ellipses.get(t.basis);
                    let p = s.places.get(e.pl);
                    let (q, d) = (s.points.get(p.loc), s.dirs.get(p.dir));
                    Geom::Ellipse { c: [q.x, q.y], a: e.a, b: e.b, rot: d.dy.atan2(d.dx), t0: t.t1, t1: t.t2 }
                }
            }
        }
        tag::POLY => {
            let p = s.polys.get(root.row);
            Geom::Polyline { pts: p.pts.iter().map(|&i| [s.points.get(i).x, s.points.get(i).y]).collect() }
        }
        tag::SPLINE => {
            let sp = s.splines.get(root.row);
            let cps = sp.cps.iter().map(|&i| [s.points.get(i).x, s.points.get(i).y]).collect();
            let mut knots = Vec::new();
            for (m, k) in sp.mults.iter().zip(sp.knots.iter()) {
                knots.extend(std::iter::repeat_n(*k, *m as usize));
            }
            Geom::Spline {
                degree: sp.degree,
                cps,
                knots,
                weights: (sp.rational == 1).then(|| sp.weights.to_vec()),
            }
        }
        _ => {
            let c = s.comps.get(root.row);
            Geom::Composite {
                parts: c
                    .segs
                    .iter()
                    .map(|&i| {
                        let sg = s.segs.get(i);
                        rebuild(s, Root { tag: sg.parent_tag, row: sg.parent })
                    })
                    .collect(),
            }
        }
    }
}

fn dense_store(f: &File) -> Store {
    Store {
        points: read_dense(&ds(f, "CARTESIAN_POINT")),
        dirs: read_dense(&ds(f, "DIRECTION")),
        vecs: read_dense(&ds(f, "VECTOR")),
        places: read_dense(&ds(f, "AXIS2_PLACEMENT_2D")),
        lines: read_dense(&ds(f, "LINE")),
        circles: read_dense(&ds(f, "CIRCLE")),
        ellipses: read_dense(&ds(f, "ELLIPSE")),
        trims: read_dense(&ds(f, "TRIMMED_CURVE")),
        polys: read_dense(&ds(f, "POLYLINE")),
        splines: read_dense(&ds(f, "B_SPLINE_CURVE_WITH_KNOTS")),
        segs: read_dense(&ds(f, "COMPOSITE_CURVE_SEGMENT")),
        comps: read_dense(&ds(f, "COMPOSITE_CURVE")),
    }
}

/// Reads every entity.
pub fn read_all(path: &str) -> hdf5::Result<Vec<Entity>> {
    let f = File::open(path)?;
    let roots: Vec<Root> = f.dataset("data/GEOMETRIC_CURVE_SET")?.read_raw()?;
    let s = dense_store(&f);
    Ok(roots.iter().enumerate().map(|(i, r)| Entity { id: i as u32, geom: rebuild(&s, *r) }).collect())
}

fn sorted(v: &mut Vec<u32>) {
    v.sort_unstable();
    v.dedup();
}

/// Reads the entities whose bounds overlap `rect`: scan the bounds table, then follow references
/// level by level, reading only the rows each level needs.
pub fn read_viewport(path: &str, log: Option<&str>, rect: [f64; 4]) -> hdf5::Result<Vec<Entity>> {
    let f = open_logged(path, log)?;
    let roots: Vec<Root> = f.dataset("data/GEOMETRIC_CURVE_SET")?.read_raw()?;
    let bbox: Vec<f64> = f.dataset("ext/bbox")?.read_raw()?;
    let ids: Vec<u32> = bbox
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, b)| overlaps(&[b[0], b[1], b[2], b[3]], &rect))
        .map(|(i, _)| i as u32)
        .collect();

    let mut need = [const { Vec::<u32>::new() }; 8];
    // Slots: 0 comp, 1 seg, 2 trim, 3 line, 4 circle, 5 ellipse, 6 vec, 7 places (points/dirs below).
    let (mut polys, mut splines, mut points, mut dirs) = (vec![], vec![], vec![], vec![]);
    for &i in &ids {
        let r = roots[i as usize];
        match r.tag {
            tag::TRIM => need[2].push(r.row),
            tag::CIRCLE => need[4].push(r.row),
            tag::POLY => polys.push(r.row),
            tag::SPLINE => splines.push(r.row),
            _ => need[0].push(r.row),
        }
    }
    for v in need.iter_mut() {
        sorted(v);
    }
    sorted(&mut polys);
    sorted(&mut splines);

    let comps: Tab<CompRow> = read_rows(&ds(&f, "COMPOSITE_CURVE"), &need[0]);
    for r in need[0].clone() {
        need[1].extend(comps.get(r).segs.iter());
    }
    sorted(&mut need[1]);
    let segs: Tab<SegRow> = read_rows(&ds(&f, "COMPOSITE_CURVE_SEGMENT"), &need[1]);
    for r in need[1].clone() {
        let sg = segs.get(r);
        match sg.parent_tag {
            tag::TRIM => need[2].push(sg.parent),
            tag::CIRCLE => need[4].push(sg.parent),
            _ => {}
        }
    }
    sorted(&mut need[2]);
    let trims: Tab<TrimRow> = read_rows(&ds(&f, "TRIMMED_CURVE"), &need[2]);
    for r in need[2].clone() {
        let t = trims.get(r);
        match t.basis_tag {
            tag::LINE => need[3].push(t.basis),
            tag::CIRCLE => need[4].push(t.basis),
            _ => need[5].push(t.basis),
        }
    }
    for k in [3, 4, 5] {
        sorted(&mut need[k]);
    }
    let lines: Tab<LineRow> = read_rows(&ds(&f, "LINE"), &need[3]);
    let circles: Tab<CircleRow> = read_rows(&ds(&f, "CIRCLE"), &need[4]);
    let ellipses: Tab<EllipseRow> = read_rows(&ds(&f, "ELLIPSE"), &need[5]);
    let mut places_need = Vec::new();
    for r in need[3].clone() {
        let l = lines.get(r);
        points.push(l.pnt);
        need[6].push(l.vec);
    }
    for r in need[4].clone() {
        places_need.push(circles.get(r).pl);
    }
    for r in need[5].clone() {
        places_need.push(ellipses.get(r).pl);
    }
    sorted(&mut need[6]);
    sorted(&mut places_need);
    let vecs: Tab<Vector> = read_rows(&ds(&f, "VECTOR"), &need[6]);
    for r in need[6].clone() {
        dirs.push(vecs.get(r).dir);
    }
    let places: Tab<Placement> = read_rows(&ds(&f, "AXIS2_PLACEMENT_2D"), &places_need);
    for &r in &places_need {
        points.push(places.get(r).loc);
        dirs.push(places.get(r).dir);
    }
    let pol: Tab<PolyRow> = read_rows(&ds(&f, "POLYLINE"), &polys);
    let spl: Tab<SplineRow> = read_rows(&ds(&f, "B_SPLINE_CURVE_WITH_KNOTS"), &splines);
    for &r in &polys {
        points.extend(pol.get(r).pts.iter());
    }
    for &r in &splines {
        points.extend(spl.get(r).cps.iter());
    }
    sorted(&mut points);
    sorted(&mut dirs);
    let s = Store {
        points: read_rows(&ds(&f, "CARTESIAN_POINT"), &points),
        dirs: read_rows(&ds(&f, "DIRECTION"), &dirs),
        vecs,
        places,
        lines,
        circles,
        ellipses,
        trims,
        polys: pol,
        splines: spl,
        segs,
        comps,
    };
    Ok(ids.iter().map(|&i| Entity { id: i, geom: rebuild(&s, roots[i as usize]) }).collect())
}
