//! A Part 21 writer for the fixtures, used only to size the baseline. It writes the exact forms of
//! `docs/ISO_PROFILE.md` (cartesian point, direction, vector, line, circle, ellipse, polyline,
//! trimmed curve, composite curve, B-spline with knots and its rational complex instance) and one
//! `GEOMETRIC_CURVE_SET` that refers to every curve. It does not write the AP242 product and
//! context boilerplate, a few kilobytes that are the same for every layout.
//!
//! No independent STEP reader has parsed this output in this spike. P2.2 and P6.3 do that. The byte
//! count is the baseline for ratio columns, nothing more.

use crate::{Drawing, Geom, Pt};
use std::io::{self, Write};

struct Counting<'a, W: Write> {
    w: &'a mut W,
    n: u64,
}

impl<W: Write> Counting<'_, W> {
    fn put(&mut self, s: &str) -> io::Result<()> {
        self.n += s.len() as u64;
        self.w.write_all(s.as_bytes())
    }
}

/// Part 21 real: shortest round-trip digits, always with a decimal point, `E` for the exponent.
pub fn real(x: f64) -> String {
    let s = format!("{x:?}");
    match s.split_once('e') {
        Some((m, e)) => {
            let m = if m.contains('.') { m.to_string() } else { format!("{m}.") };
            format!("{m}E{e}")
        }
        None => {
            if s.contains('.') {
                s
            } else {
                format!("{s}.")
            }
        }
    }
}

struct Ids {
    next: u64,
}

impl Ids {
    fn id(&mut self) -> u64 {
        self.next += 1;
        self.next
    }
}

fn point<W: Write>(o: &mut Counting<W>, ids: &mut Ids, p: Pt) -> io::Result<u64> {
    let id = ids.id();
    o.put(&format!("#{id}=CARTESIAN_POINT('',({},{}));\n", real(p[0]), real(p[1])))?;
    Ok(id)
}

fn dir<W: Write>(o: &mut Counting<W>, ids: &mut Ids, d: Pt) -> io::Result<u64> {
    let id = ids.id();
    o.put(&format!("#{id}=DIRECTION('',({},{}));\n", real(d[0]), real(d[1])))?;
    Ok(id)
}

fn placement<W: Write>(o: &mut Counting<W>, ids: &mut Ids, c: Pt, rot: f64) -> io::Result<u64> {
    let p = point(o, ids, c)?;
    let d = dir(o, ids, [rot.cos(), rot.sin()])?;
    let id = ids.id();
    o.put(&format!("#{id}=AXIS2_PLACEMENT_2D('',#{p},#{d});\n"))?;
    Ok(id)
}

fn trimmed<W: Write>(o: &mut Counting<W>, ids: &mut Ids, basis: u64, t0: f64, t1: f64) -> io::Result<u64> {
    let id = ids.id();
    o.put(&format!(
        "#{id}=TRIMMED_CURVE('',#{basis},(PARAMETER_VALUE({})),(PARAMETER_VALUE({})),.T.,.PARAMETER.);\n",
        real(t0),
        real(t1)
    ))?;
    Ok(id)
}

fn curve<W: Write>(o: &mut Counting<W>, ids: &mut Ids, g: &Geom) -> io::Result<u64> {
    match g {
        Geom::Line { a, b } => {
            let len = (b[0] - a[0]).hypot(b[1] - a[1]);
            let p = point(o, ids, *a)?;
            let d = dir(o, ids, [(b[0] - a[0]) / len, (b[1] - a[1]) / len])?;
            let v = ids.id();
            o.put(&format!("#{v}=VECTOR('',#{d},1.);\n"))?;
            let l = ids.id();
            o.put(&format!("#{l}=LINE('',#{p},#{v});\n"))?;
            trimmed(o, ids, l, 0.0, len)
        }
        Geom::Circle { c, r } => {
            let pl = placement(o, ids, *c, 0.0)?;
            let id = ids.id();
            o.put(&format!("#{id}=CIRCLE('',#{pl},{});\n", real(*r)))?;
            Ok(id)
        }
        Geom::Arc { c, r, a0, a1 } => {
            let pl = placement(o, ids, *c, 0.0)?;
            let id = ids.id();
            o.put(&format!("#{id}=CIRCLE('',#{pl},{});\n", real(*r)))?;
            trimmed(o, ids, id, *a0, *a1)
        }
        Geom::Ellipse { c, a, b, rot, t0, t1 } => {
            let pl = placement(o, ids, *c, *rot)?;
            let id = ids.id();
            o.put(&format!("#{id}=ELLIPSE('',#{pl},{},{});\n", real(*a), real(*b)))?;
            trimmed(o, ids, id, *t0, *t1)
        }
        Geom::Polyline { pts } => {
            let refs = pts.iter().map(|p| point(o, ids, *p)).collect::<io::Result<Vec<_>>>()?;
            let id = ids.id();
            let list: Vec<String> = refs.iter().map(|r| format!("#{r}")).collect();
            o.put(&format!("#{id}=POLYLINE('',({}));\n", list.join(",")))?;
            Ok(id)
        }
        Geom::Spline { degree, cps, knots, weights } => {
            let refs = cps.iter().map(|p| point(o, ids, *p)).collect::<io::Result<Vec<_>>>()?;
            let list: Vec<String> = refs.iter().map(|r| format!("#{r}")).collect();
            // Distinct knots with multiplicities.
            let mut mults: Vec<usize> = Vec::new();
            let mut distinct: Vec<f64> = Vec::new();
            for &k in knots {
                if distinct.last() == Some(&k) {
                    *mults.last_mut().unwrap() += 1;
                } else {
                    distinct.push(k);
                    mults.push(1);
                }
            }
            let m: Vec<String> = mults.iter().map(|x| x.to_string()).collect();
            let k: Vec<String> = distinct.iter().map(|x| real(*x)).collect();
            let id = ids.id();
            match weights {
                None => o.put(&format!(
                    "#{id}=B_SPLINE_CURVE_WITH_KNOTS('',{degree},({}),.UNSPECIFIED.,.F.,.U.,({}),({}),.UNSPECIFIED.);\n",
                    list.join(","),
                    m.join(","),
                    k.join(",")
                ))?,
                Some(w) => {
                    let w: Vec<String> = w.iter().map(|x| real(*x)).collect();
                    o.put(&format!(
                        "#{id}=(BOUNDED_CURVE() B_SPLINE_CURVE({degree},({}),.UNSPECIFIED.,.F.,.U.) B_SPLINE_CURVE_WITH_KNOTS(({}),({}),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE(({})) REPRESENTATION_ITEM(''));\n",
                        list.join(","),
                        m.join(","),
                        k.join(","),
                        w.join(",")
                    ))?
                }
            }
            Ok(id)
        }
        Geom::Composite { parts } => {
            let mut segs = Vec::new();
            for p in parts {
                let c = curve(o, ids, p)?;
                let s = ids.id();
                o.put(&format!("#{s}=COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#{c});\n"))?;
                segs.push(format!("#{s}"));
            }
            let id = ids.id();
            o.put(&format!("#{id}=COMPOSITE_CURVE('',({}),.F.);\n", segs.join(",")))?;
            Ok(id)
        }
    }
}

/// Writes the drawing as Part 21 text and returns the byte count.
pub fn write_part21<W: Write>(d: &Drawing, w: &mut W) -> io::Result<u64> {
    let mut o = Counting { w, n: 0 };
    o.put("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('pvec phase 1 fixture'),'2;1');\n")?;
    o.put(&format!(
        "FILE_NAME('{}.stp','',(''),(''),'','','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN'));\nENDSEC;\nDATA;\n",
        d.name
    ))?;
    let mut ids = Ids { next: 0 };
    let mut roots = Vec::with_capacity(d.entities.len());
    for e in &d.entities {
        roots.push(curve(&mut o, &mut ids, &e.geom)?);
    }
    let set = ids.id();
    let list: Vec<String> = roots.iter().map(|r| format!("#{r}")).collect();
    o.put(&format!("#{set}=GEOMETRIC_CURVE_SET('',({}));\nENDSEC;\nEND-ISO-10303-21;\n", list.join(",")))?;
    Ok(o.n)
}
