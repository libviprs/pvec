//! The shape dump the STEP-HDF arm reads. The HDF toolchain is a separate workspace (`spikes/step-hdf`) that
//! cannot depend on this crate, so the three arms meet at this file: the `.pvec` arm and the Part 21 arm are
//! built from the same shape list in this process, and the HDF arm from the dump of that list.
//!
//! Little endian throughout. Header: the eight bytes `PVSH1\0\0\0`, the shape count (`u64`), the FNV-1a 64
//! digest of the body (`u64`), the extent (four `f64`). Body, per shape, a tag byte and then
//!
//! | tag | shape | payload |
//! |---|---|---|
//! | 0 | line | `ax ay bx by` |
//! | 1 | polyline | `n: u32`, then `n` pairs `x y` |
//! | 2 | circle or arc | `cx cy r start sweep` |
//! | 3 | ellipse or arc | `cx cy a b rotation start sweep` |
//! | 4 | B-spline | `degree: u8`, `weighted: u8`, `n: u32`, `k: u32`, `n` x, `n` y, `k` knots, `n` weights if weighted |

use crate::shape::Shape;

/// The magic bytes.
pub const MAGIC: [u8; 8] = *b"PVSH1\0\0\0";

/// FNV-1a over bytes. Std only, and the same on every platform.
pub fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

fn body(shapes: &[Shape]) -> Vec<u8> {
    let mut out = Vec::new();
    let f = |out: &mut Vec<u8>, vs: &[f64]| vs.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    for s in shapes {
        match s {
            Shape::Line(a, b) => {
                out.push(0);
                f(&mut out, &[a[0], a[1], b[0], b[1]]);
            }
            Shape::Polyline(p) => {
                out.push(1);
                out.extend_from_slice(&(p.len() as u32).to_le_bytes());
                p.iter().for_each(|q| f(&mut out, q));
            }
            Shape::Arc { centre, radius, start_sweep } => {
                out.push(2);
                f(&mut out, &[centre[0], centre[1], *radius, start_sweep[0], start_sweep[1]]);
            }
            Shape::Ellipse { centre, radii, angles } => {
                out.push(3);
                f(&mut out, &[centre[0], centre[1], radii[0], radii[1], angles[0], angles[1], angles[2]]);
            }
            Shape::Spline(sp) => {
                out.push(4);
                out.push(sp.degree as u8);
                out.push(u8::from(sp.weights.is_some()));
                out.extend_from_slice(&(sp.x.len() as u32).to_le_bytes());
                out.extend_from_slice(&(sp.knots.len() as u32).to_le_bytes());
                f(&mut out, &sp.x);
                f(&mut out, &sp.y);
                f(&mut out, &sp.knots);
                if let Some(w) = &sp.weights {
                    f(&mut out, w);
                }
            }
        }
    }
    out
}

/// The 16 hex digits that name a shape list: the digest of its dump body.
pub fn digest(shapes: &[Shape]) -> String {
    format!("{:016x}", fnv(&body(shapes)))
}

/// The dump of a shape list with the extent the drawing declares.
pub fn dump(shapes: &[Shape], extent: [f64; 4]) -> Vec<u8> {
    let body = body(shapes);
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(shapes.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv(&body).to_le_bytes());
    extent.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()));
    out.extend_from_slice(&body);
    out
}
