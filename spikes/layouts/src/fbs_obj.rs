//! Baseline: FlatBuffers, one table per entity (schema/pvec_obj.fbs). This is the shape that panel
//! finding 6 warns against, kept as the thing the type-batched layouts are measured against.

use crate::common::Codec;
use crate::obj_gen::obj;
use flatbuffers::FlatBufferBuilder;
use pvec_fixtures::{Entity, Geom};

/// The per-entity-object baseline.
pub struct FbObj;

fn pt(p: [f64; 2]) -> obj::Pt {
    obj::Pt::new(p[0], p[1])
}

fn geom<'a>(
    fbb: &mut FlatBufferBuilder<'a>,
    g: &Geom,
) -> (obj::Geom, flatbuffers::WIPOffset<flatbuffers::UnionWIPOffset>) {
    match g {
        Geom::Line { a, b } => {
            let (a, b) = (pt(*a), pt(*b));
            (
                obj::Geom::Line,
                obj::Line::create(fbb, &obj::LineArgs { a: Some(&a), b: Some(&b) }).as_union_value(),
            )
        }
        Geom::Circle { c, r } => {
            let c = pt(*c);
            (
                obj::Geom::Circle,
                obj::Circle::create(fbb, &obj::CircleArgs { c: Some(&c), r: *r }).as_union_value(),
            )
        }
        Geom::Arc { c, r, a0, a1 } => {
            let c = pt(*c);
            (
                obj::Geom::Arc,
                obj::Arc::create(fbb, &obj::ArcArgs { c: Some(&c), r: *r, a0: *a0, a1: *a1 })
                    .as_union_value(),
            )
        }
        Geom::Ellipse { c, a, b, rot, t0, t1 } => {
            let c = pt(*c);
            let args = obj::EllipseArgs { c: Some(&c), a: *a, b: *b, rot: *rot, t0: *t0, t1: *t1 };
            (obj::Geom::Ellipse, obj::Ellipse::create(fbb, &args).as_union_value())
        }
        Geom::Polyline { pts } => {
            let v: Vec<obj::Pt> = pts.iter().map(|p| pt(*p)).collect();
            let pts = Some(fbb.create_vector(&v));
            (obj::Geom::Polyline, obj::Polyline::create(fbb, &obj::PolylineArgs { pts }).as_union_value())
        }
        Geom::Spline { degree, cps, knots, weights } => {
            let v: Vec<obj::Pt> = cps.iter().map(|p| pt(*p)).collect();
            let args = obj::SplineArgs {
                degree: *degree,
                cps: Some(fbb.create_vector(&v)),
                knots: Some(fbb.create_vector(knots)),
                weights: weights.as_ref().map(|w| fbb.create_vector(w)),
            };
            (obj::Geom::Spline, obj::Spline::create(fbb, &args).as_union_value())
        }
        Geom::Composite { parts } => {
            let segs: Vec<_> = parts
                .iter()
                .map(|p| match p {
                    Geom::Line { a, b } => {
                        let (a, b) = (pt(*a), pt(*b));
                        let line = obj::Line::create(fbb, &obj::LineArgs { a: Some(&a), b: Some(&b) });
                        obj::Seg::create(fbb, &obj::SegArgs { line: Some(line), arc: None })
                    }
                    Geom::Arc { c, r, a0, a1 } => {
                        let c = pt(*c);
                        let arc =
                            obj::Arc::create(fbb, &obj::ArcArgs { c: Some(&c), r: *r, a0: *a0, a1: *a1 });
                        obj::Seg::create(fbb, &obj::SegArgs { line: None, arc: Some(arc) })
                    }
                    _ => unreachable!("composites hold lines and arcs"),
                })
                .collect();
            let parts = Some(fbb.create_vector(&segs));
            (
                obj::Geom::Composite,
                obj::Composite::create(fbb, &obj::CompositeArgs { parts }).as_union_value(),
            )
        }
    }
}

fn s(a: u64, b: f64) -> u64 {
    a.wrapping_add(b.to_bits())
}

fn touch_line(l: &obj::Line<'_>) -> u64 {
    let z = obj::Pt::new(0.0, 0.0);
    let (a, b) = (*l.a().unwrap_or(&z), *l.b().unwrap_or(&z));
    [a.x(), a.y(), b.x(), b.y()].iter().fold(0, |t, v| s(t, *v))
}

fn touch_arc(a: &obj::Arc<'_>) -> u64 {
    let c = a.c().cloned().unwrap_or_else(|| obj::Pt::new(0.0, 0.0));
    [c.x(), c.y(), a.r(), a.a0(), a.a1()].iter().fold(0, |t, v| s(t, *v))
}

impl Codec for FbObj {
    const NAME: &'static str = "baseline flatbuffers per entity";

    fn encode_block(ents: &[&Entity]) -> Vec<u8> {
        let mut fbb = FlatBufferBuilder::with_capacity(ents.len() * 128);
        let mut offs = Vec::with_capacity(ents.len());
        for e in ents {
            let (t, g) = geom(&mut fbb, &e.geom);
            offs.push(obj::Entity::create(
                &mut fbb,
                &obj::EntityArgs { id: e.id, geom_type: t, geom: Some(g) },
            ));
        }
        let v = fbb.create_vector(&offs);
        let root = obj::Block::create(&mut fbb, &obj::BlockArgs { entities: Some(v) });
        obj::finish_block_buffer(&mut fbb, root);
        fbb.finished_data().to_vec()
    }

    #[inline(never)]
    fn touch(block: &[u8]) -> Result<u64, String> {
        let b = obj::root_as_block(block).map_err(|e| e.to_string())?;
        let mut sum = 0u64;
        for e in b.entities().ok_or("no entities")?.iter() {
            sum = sum.wrapping_add(e.id() as u64);
            sum = sum.wrapping_add(match e.geom_type() {
                obj::Geom::Line => touch_line(&e.geom_as_line().ok_or("line")?),
                obj::Geom::Circle => {
                    let c = e.geom_as_circle().ok_or("circle")?;
                    let p = c.c().cloned().unwrap_or_else(|| obj::Pt::new(0.0, 0.0));
                    [p.x(), p.y(), c.r()].iter().fold(0, |t, v| s(t, *v))
                }
                obj::Geom::Arc => touch_arc(&e.geom_as_arc().ok_or("arc")?),
                obj::Geom::Ellipse => {
                    let x = e.geom_as_ellipse().ok_or("ellipse")?;
                    let p = x.c().cloned().unwrap_or_else(|| obj::Pt::new(0.0, 0.0));
                    [p.x(), p.y(), x.a(), x.b(), x.rot(), x.t0(), x.t1()].iter().fold(0, |t, v| s(t, *v))
                }
                obj::Geom::Polyline => e
                    .geom_as_polyline()
                    .and_then(|p| p.pts())
                    .map_or(0, |v| v.iter().fold(0, |t, q| s(s(t, q.x()), q.y()))),
                obj::Geom::Spline => {
                    let x = e.geom_as_spline().ok_or("spline")?;
                    let mut t = x.degree() as u64;
                    if let Some(v) = x.cps() {
                        t = v.iter().fold(t, |t, q| s(s(t, q.x()), q.y()));
                    }
                    if let Some(v) = x.knots() {
                        t = v.iter().fold(t, s);
                    }
                    if let Some(v) = x.weights() {
                        t = v.iter().fold(t, s);
                    }
                    t
                }
                obj::Geom::Composite => {
                    let mut t = 0u64;
                    for sg in e.geom_as_composite().and_then(|c| c.parts()).ok_or("composite")?.iter() {
                        if let Some(l) = sg.line() {
                            t = t.wrapping_add(touch_line(&l));
                        } else if let Some(a) = sg.arc() {
                            t = t.wrapping_add(touch_arc(&a)).wrapping_add(1);
                        }
                    }
                    t
                }
                _ => return Err("unknown geometry kind".into()),
            });
        }
        Ok(sum)
    }

    fn decode(block: &[u8]) -> Result<Vec<Entity>, String> {
        let b = obj::root_as_block(block).map_err(|e| e.to_string())?;
        let z = obj::Pt::new(0.0, 0.0);
        let p = |q: Option<&obj::Pt>| {
            let q = q.unwrap_or(&z);
            [q.x(), q.y()]
        };
        let mut out = Vec::new();
        for e in b.entities().ok_or("no entities")?.iter() {
            let geom = match e.geom_type() {
                obj::Geom::Line => {
                    let l = e.geom_as_line().ok_or("line")?;
                    Geom::Line { a: p(l.a()), b: p(l.b()) }
                }
                obj::Geom::Circle => {
                    let c = e.geom_as_circle().ok_or("circle")?;
                    Geom::Circle { c: p(c.c()), r: c.r() }
                }
                obj::Geom::Arc => {
                    let a = e.geom_as_arc().ok_or("arc")?;
                    Geom::Arc { c: p(a.c()), r: a.r(), a0: a.a0(), a1: a.a1() }
                }
                obj::Geom::Ellipse => {
                    let x = e.geom_as_ellipse().ok_or("ellipse")?;
                    Geom::Ellipse { c: p(x.c()), a: x.a(), b: x.b(), rot: x.rot(), t0: x.t0(), t1: x.t1() }
                }
                obj::Geom::Polyline => {
                    let x = e.geom_as_polyline().ok_or("polyline")?;
                    Geom::Polyline {
                        pts: x.pts().map(|v| v.iter().map(|q| [q.x(), q.y()]).collect()).unwrap_or_default(),
                    }
                }
                obj::Geom::Spline => {
                    let x = e.geom_as_spline().ok_or("spline")?;
                    Geom::Spline {
                        degree: x.degree(),
                        cps: x.cps().map(|v| v.iter().map(|q| [q.x(), q.y()]).collect()).unwrap_or_default(),
                        knots: x.knots().map(|v| v.iter().collect()).unwrap_or_default(),
                        weights: x.weights().map(|v| v.iter().collect()),
                    }
                }
                obj::Geom::Composite => {
                    let c = e.geom_as_composite().ok_or("composite")?;
                    let mut parts = Vec::new();
                    for sg in c.parts().ok_or("parts")?.iter() {
                        if let Some(l) = sg.line() {
                            parts.push(Geom::Line { a: p(l.a()), b: p(l.b()) });
                        } else if let Some(a) = sg.arc() {
                            parts.push(Geom::Arc { c: p(a.c()), r: a.r(), a0: a.a0(), a1: a.a1() });
                        }
                    }
                    Geom::Composite { parts }
                }
                _ => return Err("unknown geometry kind".into()),
            };
            out.push(Entity { id: e.id(), geom });
        }
        Ok(out)
    }
}
