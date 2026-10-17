//! Property tests for the basic codecs (P3.2): random valid curves round trip bit for bit.
#![allow(missing_docs)]

mod common;
use common::*;
use core::f64::consts::TAU;
use pvec_codec::*;
use pvec_entity::EntityId;
use pvec_geometry::*;

const SEED: u64 = 0xBA51C;

fn random_placement(r: &mut Rng) -> Placement2 {
    let a = r.range(0.0, TAU);
    Placement2::new(pt(r.range(-1000.0, 1000.0), r.range(-1000.0, 1000.0)), v(a.cos(), a.sin())).unwrap()
}

fn random_curve(r: &mut Rng) -> Curve {
    loop {
        let made = match r.below(8) {
            0 => Line::new(
                pt(r.range(-50.0, 50.0), r.range(-50.0, 50.0)),
                v(r.range(-1.0, 1.0), r.range(-1.0, 1.0)),
            )
            .map(Curve::Line),
            1 => Circle::new(random_placement(r), r.range(0.01, 50.0)).map(Curve::Circle),
            2 => {
                let a = r.range(0.5, 40.0);
                Ellipse::new(random_placement(r), a, a * r.range(0.2, 1.0)).map(Curve::Ellipse)
            }
            3 => {
                let n = 2 + r.below(30);
                Polyline::new((0..n).map(|i| pt(i as f64 * 3.0 + r.unit(), r.range(-20.0, 20.0))).collect())
                    .map(Curve::Polyline)
            }
            4 => {
                let l = Line::new(
                    pt(r.range(-9.0, 9.0), r.range(-9.0, 9.0)),
                    v(r.range(-1.0, 1.0), r.range(-1.0, 1.0)),
                );
                l.and_then(|l| {
                    let (a, b) = (r.range(-20.0, 20.0), r.range(-20.0, 20.0));
                    Trimmed::new(Curve::Line(l), a, b, a < b).map(Curve::Trimmed)
                })
            }
            5 => Circle::new(random_placement(r), r.range(0.01, 50.0)).and_then(|c| {
                Trimmed::new(Curve::Circle(c), r.range(-12.0, 12.0), r.range(-12.0, 12.0), r.below(2) == 0)
                    .map(Curve::Trimmed)
            }),
            6 => {
                let a = r.range(0.5, 40.0);
                Ellipse::new(random_placement(r), a, a * r.range(0.2, 1.0)).and_then(|e| {
                    Trimmed::new(
                        Curve::Ellipse(e),
                        r.range(-12.0, 12.0),
                        r.range(-12.0, 12.0),
                        r.below(2) == 0,
                    )
                    .map(Curve::Trimmed)
                })
            }
            _ => {
                // a chain of polyline segments, optionally closed back to the start
                let n = 2 + r.below(5);
                let mut at = pt(r.range(-5.0, 5.0), r.range(-5.0, 5.0));
                let start = at;
                let mut segs = Vec::new();
                for k in 0..n {
                    let next = if k + 1 == n && r.below(2) == 0 {
                        start
                    } else {
                        pt(at.x + r.range(1.0, 4.0), at.y + r.range(-4.0, 4.0))
                    };
                    match Polyline::new(vec![at, next]) {
                        Ok(p) => segs.push(Curve::Polyline(p)),
                        Err(_) => break,
                    }
                    at = next;
                }
                Composite::new(segs)
                    .and_then(|c| if at == start && r.below(2) == 0 { c.declare_closed() } else { Ok(c) })
                    .map(Curve::Composite)
            }
        };
        if let Ok(c) = made {
            return c;
        }
    }
}

#[test]
fn random_curves_round_trip_bit_for_bit() {
    let mut r = Rng(SEED);
    for case in 0..300 {
        let n = 1 + r.below(40);
        let mut ents = Vec::new();
        let mut next = 0u32;
        for _ in 0..n {
            next += 1 + r.below(5) as u32;
            ents.push((EntityId::new(next).unwrap(), random_curve(&mut r)));
        }
        let enc = encode(&ents).unwrap();
        let back = decode(&enc.bytes).unwrap_or_else(|e| panic!("seed {SEED:#x} case {case}: {e}"));
        assert_eq!(back.len(), ents.len());
        for (eid, c) in &ents {
            let (_, got) = back.iter().find(|(i, _)| i == eid).unwrap();
            assert_bit_equal(&format!("seed {SEED:#x} case {case} {eid}"), c, got);
        }
        // and the encoding of the decoded set is the same bytes: the format is canonical
        let again = encode(&back).unwrap();
        assert_eq!(again.bytes, enc.bytes, "seed {SEED:#x} case {case}: re-encoding changed the bytes");
    }
}

#[test]
fn the_same_entities_in_any_input_order_decode_to_the_same_set() {
    let mut r = Rng(SEED + 1);
    for _ in 0..50 {
        let n = 2 + r.below(20);
        let ents: Vec<(EntityId, Curve)> =
            (0..n).map(|i| (EntityId::new(i as u32).unwrap(), random_curve(&mut r))).collect();
        let mut shuffled = ents.clone();
        for i in (1..shuffled.len()).rev() {
            shuffled.swap(i, r.below(i + 1));
        }
        let a = decode(&encode(&ents).unwrap().bytes).unwrap();
        let b = decode(&encode(&shuffled).unwrap().bytes).unwrap();
        for (eid, c) in &ents {
            let ga = &a.iter().find(|(i, _)| i == eid).unwrap().1;
            let gb = &b.iter().find(|(i, _)| i == eid).unwrap().1;
            assert_bit_equal("order independence", c, ga);
            assert_bit_equal("order independence", c, gb);
        }
    }
}
