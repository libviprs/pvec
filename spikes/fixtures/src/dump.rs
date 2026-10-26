//! The shape dump the phase 8 benchmarks hand to the spikes (`crates/pvec-bench/src/dump.rs` writes it, `PVSH1`):
//! the benchmark corpus entry as the phase 1 `Drawing`, so the spike models run on exactly the geometry the
//! `.pvec` arm holds.

use crate::{Drawing, Entity, Geom};
use std::f64::consts::TAU;

const MAGIC: &[u8; 8] = b"PVSH1\0\0\0";
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

struct Reader<'a>(&'a [u8], usize);

impl Reader<'_> {
    fn bytes(&mut self, n: usize) -> &[u8] {
        let s = &self.0[self.1..self.1 + n];
        self.1 += n;
        s
    }
    fn u8(&mut self) -> u8 {
        self.bytes(1)[0]
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes(4).try_into().expect("4 bytes"))
    }
    fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.bytes(8).try_into().expect("8 bytes"))
    }
    fn f(&mut self) -> f64 {
        f64::from_bits(self.u64())
    }
    fn fs(&mut self, n: usize) -> Vec<f64> {
        (0..n).map(|_| self.f()).collect()
    }
}

/// The drawing in a shape dump. Fails if the digest in the header is not the digest of the body.
pub fn load(name: &str, bytes: &[u8]) -> Result<Drawing, String> {
    if bytes.len() < 56 || &bytes[..8] != MAGIC {
        return Err("not a PVSH1 dump".into());
    }
    let mut h = Reader(bytes, 8);
    let count = h.u64() as usize;
    let digest = h.u64();
    let extent = [h.f(), h.f(), h.f(), h.f()];
    if fnv(&bytes[h.1..]) != digest {
        return Err("the digest in the header is not the digest of the body".into());
    }
    let mut entities = Vec::with_capacity(count);
    for id in 0..count {
        let geom = match h.u8() {
            0 => Geom::Line { a: [h.f(), h.f()], b: [h.f(), h.f()] },
            1 => {
                let n = h.u32() as usize;
                Geom::Polyline { pts: (0..n).map(|_| [h.f(), h.f()]).collect() }
            }
            2 => {
                let (c, r, start, sweep) = ([h.f(), h.f()], h.f(), h.f(), h.f());
                if sweep.abs() == TAU {
                    Geom::Circle { c, r }
                } else {
                    let (a0, a1) = if sweep > 0.0 { (start, start + sweep) } else { (start + sweep, start) };
                    Geom::Arc { c, r, a0, a1 }
                }
            }
            3 => {
                let (c, a, b, rot, start, sweep) = ([h.f(), h.f()], h.f(), h.f(), h.f(), h.f(), h.f());
                let (t0, t1) = if sweep.abs() == TAU {
                    (0.0, TAU)
                } else if sweep > 0.0 {
                    (start, start + sweep)
                } else {
                    (start + sweep, start)
                };
                Geom::Ellipse { c, a, b, rot, t0, t1 }
            }
            4 => {
                let (degree, weighted) = (h.u8(), h.u8() == 1);
                let (n, k) = (h.u32() as usize, h.u32() as usize);
                let (x, y, knots) = (h.fs(n), h.fs(n), h.fs(k));
                let weights = weighted.then(|| h.fs(n));
                Geom::Spline {
                    degree,
                    cps: x.iter().zip(&y).map(|(x, y)| [*x, *y]).collect(),
                    knots,
                    weights,
                }
            }
            t => return Err(format!("unknown tag {t}")),
        };
        entities.push(Entity { id: id as u32, geom });
    }
    Ok(Drawing { name: Box::leak(name.to_string().into_boxed_str()), extent, entities })
}
