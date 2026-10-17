//! H2: HDF5 as a plain container for a type-batched, structure-of-arrays layout with a flat
//! bounding-box dataset. No STEP entity tables; offsets instead of variable-length types.
#![allow(missing_docs)]

use crate::schema::Root;
use crate::util::*;
use hdf5::{Dataset, File};
use hdf5_metno as hdf5;
use pvec_fixtures::{Drawing, Entity, Geom, overlaps};
use std::collections::BTreeMap;

#[derive(Default)]
struct Soa {
    roots: Vec<Root>,
    bbox: Vec<f64>,
    line: Vec<f64>,
    circle: Vec<f64>,
    arc: Vec<f64>,
    ellipse: Vec<f64>,
    poly_off: Vec<u32>,
    poly_pts: Vec<f64>,
    sp_degree: Vec<u8>,
    sp_rat: Vec<u8>,
    sp_cp_off: Vec<u32>,
    sp_knot_off: Vec<u32>,
    sp_w_off: Vec<u32>,
    sp_cps: Vec<f64>,
    sp_knots: Vec<f64>,
    sp_w: Vec<f64>,
    comp_off: Vec<u32>,
    comp_tag: Vec<u8>,
    comp_vals: Vec<f64>,
}

fn comp_part(s: &mut Soa, g: &Geom) {
    match g {
        Geom::Line { a, b } => {
            s.comp_tag.push(0);
            s.comp_vals.extend([a[0], a[1], b[0], b[1], 0.0]);
        }
        Geom::Arc { c, r, a0, a1 } => {
            s.comp_tag.push(2);
            s.comp_vals.extend([c[0], c[1], *r, *a0, *a1]);
        }
        other => panic!("composite part of kind {} is not in the fixtures", other.tag()),
    }
}

fn build(d: &Drawing) -> Soa {
    let mut s = Soa {
        poly_off: vec![0],
        sp_cp_off: vec![0],
        sp_knot_off: vec![0],
        sp_w_off: vec![0],
        comp_off: vec![0],
        ..Default::default()
    };
    for e in &d.entities {
        let bb = e.geom.bbox();
        s.bbox.extend(bb);
        let row;
        match &e.geom {
            Geom::Line { a, b } => {
                row = s.line.len() / 4;
                s.line.extend([a[0], a[1], b[0], b[1]]);
            }
            Geom::Circle { c, r } => {
                row = s.circle.len() / 3;
                s.circle.extend([c[0], c[1], *r]);
            }
            Geom::Arc { c, r, a0, a1 } => {
                row = s.arc.len() / 5;
                s.arc.extend([c[0], c[1], *r, *a0, *a1]);
            }
            Geom::Ellipse { c, a, b, rot, t0, t1 } => {
                row = s.ellipse.len() / 7;
                s.ellipse.extend([c[0], c[1], *a, *b, *rot, *t0, *t1]);
            }
            Geom::Polyline { pts } => {
                row = s.poly_off.len() - 1;
                for p in pts {
                    s.poly_pts.extend(*p);
                }
                s.poly_off.push((s.poly_pts.len() / 2) as u32);
            }
            Geom::Spline { degree, cps, knots, weights } => {
                row = s.sp_degree.len();
                s.sp_degree.push(*degree);
                s.sp_rat.push(weights.is_some() as u8);
                for p in cps {
                    s.sp_cps.extend(*p);
                }
                s.sp_knots.extend(knots);
                if let Some(w) = weights {
                    s.sp_w.extend(w);
                }
                s.sp_cp_off.push((s.sp_cps.len() / 2) as u32);
                s.sp_knot_off.push(s.sp_knots.len() as u32);
                s.sp_w_off.push(s.sp_w.len() as u32);
            }
            Geom::Composite { parts } => {
                row = s.comp_off.len() - 1;
                for p in parts {
                    comp_part(&mut s, p);
                }
                s.comp_off.push(s.comp_tag.len() as u32);
            }
        }
        s.roots.push(Root { tag: e.geom.tag(), row: row as u32 });
    }
    s
}

pub fn write(d: &Drawing, path: &str, cfg: &Cfg) -> hdf5::Result<()> {
    let s = build(d);
    let f = create_file(path, cfg)?;
    let g = f.create_group("pvec_h2")?;
    put1(&g, "roots", &s.roots, cfg)?;
    put2(&g, "bbox", 4, &s.bbox, cfg)?;
    put2(&g, "line", 4, &s.line, cfg)?;
    put2(&g, "circle", 3, &s.circle, cfg)?;
    put2(&g, "arc", 5, &s.arc, cfg)?;
    put2(&g, "ellipse", 7, &s.ellipse, cfg)?;
    put1(&g, "poly_off", &s.poly_off, cfg)?;
    put2(&g, "poly_pts", 2, &s.poly_pts, cfg)?;
    put1(&g, "sp_degree", &s.sp_degree, cfg)?;
    put1(&g, "sp_rat", &s.sp_rat, cfg)?;
    put1(&g, "sp_cp_off", &s.sp_cp_off, cfg)?;
    put1(&g, "sp_knot_off", &s.sp_knot_off, cfg)?;
    put1(&g, "sp_w_off", &s.sp_w_off, cfg)?;
    put2(&g, "sp_cps", 2, &s.sp_cps, cfg)?;
    put1(&g, "sp_knots", &s.sp_knots, cfg)?;
    put1(&g, "sp_w", &s.sp_w, cfg)?;
    put1(&g, "comp_off", &s.comp_off, cfg)?;
    put1(&g, "comp_tag", &s.comp_tag, cfg)?;
    put2(&g, "comp_vals", 5, &s.comp_vals, cfg)?;
    f.flush()?;
    Ok(())
}

/// Logical (uncompressed) payload bytes of everything written for `d`, for the overhead fraction.
pub fn payload_bytes(d: &Drawing) -> u64 {
    let s = build(d);
    let f64s = s.bbox.len()
        + s.line.len()
        + s.circle.len()
        + s.arc.len()
        + s.ellipse.len()
        + s.poly_pts.len()
        + s.sp_cps.len()
        + s.sp_knots.len()
        + s.sp_w.len()
        + s.comp_vals.len();
    let u32s =
        s.poly_off.len() + s.sp_cp_off.len() + s.sp_knot_off.len() + s.sp_w_off.len() + s.comp_off.len();
    let u8s = s.sp_degree.len() + s.sp_rat.len() + s.comp_tag.len();
    (f64s * 8 + u32s * 4 + u8s + s.roots.len() * 8) as u64
}

// ------------------------------------------------------------------------------------- reading

fn ds(f: &File, name: &str) -> Option<Dataset> {
    f.dataset(&format!("pvec_h2/{name}")).ok()
}

fn f64s(d: &Option<Dataset>, cols: usize, a: usize, b: usize) -> Vec<f64> {
    match d {
        Some(d) if b > a => d.read_slice_2d::<f64, _>((a..b, 0..cols)).expect("read").into_iter().collect(),
        _ => Vec::new(),
    }
}

fn f64s1(d: &Option<Dataset>, a: usize, b: usize) -> Vec<f64> {
    match d {
        Some(d) if b > a => d.read_slice_1d::<f64, _>(a..b).expect("read").into_iter().collect(),
        _ => Vec::new(),
    }
}

fn u32s(d: &Option<Dataset>, a: usize, b: usize) -> Vec<u32> {
    match d {
        Some(d) if b > a => d.read_slice_1d::<u32, _>(a..b).expect("read").into_iter().collect(),
        _ => Vec::new(),
    }
}

fn u8s(d: &Option<Dataset>, a: usize, b: usize) -> Vec<u8> {
    match d {
        Some(d) if b > a => d.read_slice_1d::<u8, _>(a..b).expect("read").into_iter().collect(),
        _ => Vec::new(),
    }
}

/// Merges sorted row numbers into half-open runs of consecutive rows.
pub fn runs(rows: &[u32]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for &r in rows {
        let r = r as usize;
        match out.last_mut() {
            Some(l) if l.1 == r => l.1 = r + 1,
            _ => out.push((r, r + 1)),
        }
    }
    out
}

struct Tables {
    line: Option<Dataset>,
    circle: Option<Dataset>,
    arc: Option<Dataset>,
    ellipse: Option<Dataset>,
    poly_off: Option<Dataset>,
    poly_pts: Option<Dataset>,
    sp_degree: Option<Dataset>,
    sp_rat: Option<Dataset>,
    sp_cp_off: Option<Dataset>,
    sp_knot_off: Option<Dataset>,
    sp_w_off: Option<Dataset>,
    sp_cps: Option<Dataset>,
    sp_knots: Option<Dataset>,
    sp_w: Option<Dataset>,
    comp_off: Option<Dataset>,
    comp_tag: Option<Dataset>,
    comp_vals: Option<Dataset>,
}

fn open_tables(f: &File) -> Tables {
    Tables {
        line: ds(f, "line"),
        circle: ds(f, "circle"),
        arc: ds(f, "arc"),
        ellipse: ds(f, "ellipse"),
        poly_off: ds(f, "poly_off"),
        poly_pts: ds(f, "poly_pts"),
        sp_degree: ds(f, "sp_degree"),
        sp_rat: ds(f, "sp_rat"),
        sp_cp_off: ds(f, "sp_cp_off"),
        sp_knot_off: ds(f, "sp_knot_off"),
        sp_w_off: ds(f, "sp_w_off"),
        sp_cps: ds(f, "sp_cps"),
        sp_knots: ds(f, "sp_knots"),
        sp_w: ds(f, "sp_w"),
        comp_off: ds(f, "comp_off"),
        comp_tag: ds(f, "comp_tag"),
        comp_vals: ds(f, "comp_vals"),
    }
}

fn pts_of(v: &[f64]) -> Vec<[f64; 2]> {
    v.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
}

fn part(tag: u8, v: &[f64]) -> Geom {
    if tag == 0 {
        Geom::Line { a: [v[0], v[1]], b: [v[2], v[3]] }
    } else {
        Geom::Arc { c: [v[0], v[1]], r: v[2], a0: v[3], a1: v[4] }
    }
}

/// Reads the geometry for the given rows of one kind. Rows must be sorted and unique.
fn read_kind(t: &Tables, tag: u8, rows: &[u32], out: &mut dyn FnMut(u8, u32, Geom)) {
    match tag {
        0..=3 => {
            let (d, cols) = match tag {
                0 => (&t.line, 4),
                1 => (&t.circle, 3),
                2 => (&t.arc, 5),
                _ => (&t.ellipse, 7),
            };
            for (a, b) in runs(rows) {
                let v = f64s(d, cols, a, b);
                for (i, c) in v.chunks_exact(cols).enumerate() {
                    let g = match tag {
                        0 => Geom::Line { a: [c[0], c[1]], b: [c[2], c[3]] },
                        1 => Geom::Circle { c: [c[0], c[1]], r: c[2] },
                        2 => Geom::Arc { c: [c[0], c[1]], r: c[2], a0: c[3], a1: c[4] },
                        _ => {
                            Geom::Ellipse { c: [c[0], c[1]], a: c[2], b: c[3], rot: c[4], t0: c[5], t1: c[6] }
                        }
                    };
                    out(tag, (a + i) as u32, g);
                }
            }
        }
        4 => {
            for (a, b) in runs(rows) {
                let off = u32s(&t.poly_off, a, b + 1);
                let pts = f64s(&t.poly_pts, 2, off[0] as usize, off[off.len() - 1] as usize);
                for i in 0..(b - a) {
                    let (s, e) = ((off[i] - off[0]) as usize, (off[i + 1] - off[0]) as usize);
                    out(tag, (a + i) as u32, Geom::Polyline { pts: pts_of(&pts[2 * s..2 * e]) });
                }
            }
        }
        5 => {
            for (a, b) in runs(rows) {
                let (deg, rat) = (u8s(&t.sp_degree, a, b), u8s(&t.sp_rat, a, b));
                let (co, ko, wo) = (
                    u32s(&t.sp_cp_off, a, b + 1),
                    u32s(&t.sp_knot_off, a, b + 1),
                    u32s(&t.sp_w_off, a, b + 1),
                );
                let cps = f64s(&t.sp_cps, 2, co[0] as usize, co[co.len() - 1] as usize);
                let kn = f64s1(&t.sp_knots, ko[0] as usize, ko[ko.len() - 1] as usize);
                let w = f64s1(&t.sp_w, wo[0] as usize, wo[wo.len() - 1] as usize);
                for i in 0..(b - a) {
                    let c = pts_of(&cps[2 * (co[i] - co[0]) as usize..2 * (co[i + 1] - co[0]) as usize]);
                    let k = kn[(ko[i] - ko[0]) as usize..(ko[i + 1] - ko[0]) as usize].to_vec();
                    let weights = (rat[i] == 1)
                        .then(|| w[(wo[i] - wo[0]) as usize..(wo[i + 1] - wo[0]) as usize].to_vec());
                    out(tag, (a + i) as u32, Geom::Spline { degree: deg[i], cps: c, knots: k, weights });
                }
            }
        }
        _ => {
            for (a, b) in runs(rows) {
                let off = u32s(&t.comp_off, a, b + 1);
                let (p0, p1) = (off[0] as usize, off[off.len() - 1] as usize);
                let tags = u8s(&t.comp_tag, p0, p1);
                let vals = f64s(&t.comp_vals, 5, p0, p1);
                for i in 0..(b - a) {
                    let (s, e) = ((off[i] as usize - p0), (off[i + 1] as usize - p0));
                    let parts = (s..e).map(|j| part(tags[j], &vals[5 * j..5 * j + 5])).collect();
                    out(tag, (a + i) as u32, Geom::Composite { parts });
                }
            }
        }
    }
}

fn gather(f: &File, ids: &[u32], roots: &[Root]) -> Vec<Entity> {
    let t = open_tables(f);
    let mut need: [Vec<u32>; 7] = Default::default();
    for &i in ids {
        let r = roots[i as usize];
        need[r.tag as usize].push(r.row);
    }
    // Rows come back in increasing order per kind, so each kind keeps a map from row to its slot.
    let mut got: [Vec<Option<Geom>>; 7] = Default::default();
    let mut slot: [BTreeMap<u32, usize>; 7] = Default::default();
    for (tag, rows) in need.iter_mut().enumerate() {
        rows.sort_unstable();
        rows.dedup();
        got[tag] = vec![None; rows.len()];
        for (k, r) in rows.iter().enumerate() {
            slot[tag].insert(*r, k);
        }
        let (g, sl) = (&mut got[tag], &slot[tag]);
        read_kind(&t, tag as u8, rows, &mut |tg, row, geom| {
            debug_assert_eq!(tg as usize, tag);
            g[sl[&row]] = Some(geom);
        });
    }
    ids.iter()
        .map(|&i| {
            let r = roots[i as usize];
            let k = slot[r.tag as usize][&r.row];
            Entity { id: i, geom: got[r.tag as usize][k].clone().expect("row was read") }
        })
        .collect()
}

/// Dense variant for `read_all`: every row of every kind, no per-row map.
fn gather_all(f: &File, roots: &[Root]) -> Vec<Entity> {
    let t = open_tables(f);
    let mut count = [0u32; 7];
    for r in roots {
        count[r.tag as usize] = count[r.tag as usize].max(r.row + 1);
    }
    let mut got: [Vec<Option<Geom>>; 7] = Default::default();
    for tag in 0..7 {
        let rows: Vec<u32> = (0..count[tag]).collect();
        got[tag] = vec![None; rows.len()];
        let g = &mut got[tag];
        read_kind(&t, tag as u8, &rows, &mut |_, row, geom| g[row as usize] = Some(geom));
    }
    roots
        .iter()
        .enumerate()
        .map(|(i, r)| Entity {
            id: i as u32,
            geom: got[r.tag as usize][r.row as usize].take().expect("row was read"),
        })
        .collect()
}

/// Reads every entity.
pub fn read_all(path: &str) -> hdf5::Result<Vec<Entity>> {
    let f = File::open(path)?;
    let roots: Vec<Root> = f.dataset("pvec_h2/roots")?.read_raw()?;
    Ok(gather_all(&f, &roots))
}

/// Reads the entities whose bounds overlap `rect`, the way a viewport query would: scan the
/// flat bounds dataset, then gather the matching rows.
pub fn read_viewport(path: &str, log: Option<&str>, rect: [f64; 4]) -> hdf5::Result<Vec<Entity>> {
    let f = open_logged(path, log)?;
    let roots: Vec<Root> = f.dataset("pvec_h2/roots")?.read_raw()?;
    let bbox: Vec<f64> = f.dataset("pvec_h2/bbox")?.read_raw()?;
    let ids: Vec<u32> = bbox
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, b)| overlaps(&[b[0], b[1], b[2], b[3]], &rect))
        .map(|(i, _)| i as u32)
        .collect();
    Ok(gather(&f, &ids, &roots))
}
