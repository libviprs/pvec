//! Well-known binary for the three geometry types the export writes, the GeoPackage binary header
//! around it, and the envelope of what was written.
//!
//! The ISO type codes are 2 (`LINESTRING`), 8 (`CIRCULARSTRING`) and 9 (`COMPOUNDCURVE`), all two
//! dimensional, all little endian. The envelope is computed from the written points, and for an arc
//! from the circle through them, so a feature whose arc bulges past its end points still has a box that
//! contains it.

use super::GeometryType;
use core::f64::consts::{FRAC_PI_2, TAU};

pub(super) type Pt = [f64; 2];

/// One component of a curve geometry.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Piece {
    /// Straight chords through the points, at least two.
    Line(Vec<Pt>),
    /// Circular arcs: an odd number of points, at least three, each arc taking start, a point on it
    /// and end, and the next arc starting where the last ended.
    Arcs(Vec<Pt>),
}

impl Piece {
    pub(super) fn points(&self) -> &[Pt] {
        match self {
            Piece::Line(p) | Piece::Arcs(p) => p,
        }
    }

    pub(super) fn points_mut(&mut self) -> &mut Vec<Pt> {
        match self {
            Piece::Line(p) | Piece::Arcs(p) => p,
        }
    }

    pub(super) fn kind(&self) -> GeometryType {
        match self {
            Piece::Line(_) => GeometryType::LineString,
            Piece::Arcs(_) => GeometryType::CircularString,
        }
    }

    pub(super) fn last(&self) -> Pt {
        *self.points().last().unwrap()
    }
}

fn put_header(out: &mut Vec<u8>, ty: u32, count: usize) {
    out.push(1);
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(count as u32).to_le_bytes());
}

fn put_piece(out: &mut Vec<u8>, piece: &Piece) {
    put_header(out, piece.kind().wkb_code(), piece.points().len());
    for p in piece.points() {
        out.extend_from_slice(&p[0].to_le_bytes());
        out.extend_from_slice(&p[1].to_le_bytes());
    }
}

/// The WKB of one piece on its own, or of several as a compound curve.
pub(super) fn wkb(pieces: &[Piece], compound: bool) -> (GeometryType, Vec<u8>) {
    let mut out = Vec::new();
    if compound {
        put_header(&mut out, GeometryType::CompoundCurve.wkb_code(), pieces.len());
        for p in pieces {
            put_piece(&mut out, p);
        }
        (GeometryType::CompoundCurve, out)
    } else {
        put_piece(&mut out, &pieces[0]);
        (pieces[0].kind(), out)
    }
}

/// The GeoPackageBinary form: magic, version 0, flags (little endian, XY envelope), the SRS id, the
/// envelope as `[min x, max x, min y, max y]`, then the WKB.
pub(super) fn gpkg_blob(srs_id: i32, envelope: [f64; 4], wkb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(40 + wkb.len());
    out.extend_from_slice(b"GP");
    out.push(0);
    out.push(0b0000_0011);
    out.extend_from_slice(&srs_id.to_le_bytes());
    for v in envelope {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(wkb);
    out
}

/// Centre and radius of the circle through three points, `None` when they are collinear.
pub(super) fn circumcircle(a: Pt, b: Pt, c: Pt) -> Option<(Pt, f64)> {
    // Work relative to `a` so large coordinates do not cost the small differences their digits.
    let (bx, by, cx, cy) = (b[0] - a[0], b[1] - a[1], c[0] - a[0], c[1] - a[1]);
    let d = 2.0 * (bx * cy - by * cx);
    if d == 0.0 || !d.is_finite() {
        return None;
    }
    let (b2, c2) = (bx * bx + by * by, cx * cx + cy * cy);
    let ux = (cy * b2 - by * c2) / d;
    let uy = (bx * c2 - cx * b2) / d;
    let r = ux.hypot(uy);
    (r.is_finite() && r > 0.0).then_some(([a[0] + ux, a[1] + uy], r))
}

fn grow(b: &mut [f64; 4], p: Pt) {
    b[0] = b[0].min(p[0]);
    b[1] = b[1].max(p[0]);
    b[2] = b[2].min(p[1]);
    b[3] = b[3].max(p[1]);
}

fn empty_box() -> [f64; 4] {
    [f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY]
}

/// Whether the arc `a -> b -> c` runs counter-clockwise.
fn counter_clockwise(a: Pt, b: Pt, c: Pt) -> bool {
    (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) > 0.0
}

/// The box of one three-point arc.
pub(super) fn arc_box(a: Pt, b: Pt, c: Pt) -> [f64; 4] {
    let mut out = empty_box();
    for p in [a, b, c] {
        grow(&mut out, p);
    }
    let Some((centre, r)) = circumcircle(a, b, c) else { return out };
    let angle = |p: Pt| (p[1] - centre[1]).atan2(p[0] - centre[0]);
    let (a0, a2) = (angle(a), angle(c));
    let ccw = counter_clockwise(a, b, c);
    let sweep = if ccw { (a2 - a0).rem_euclid(TAU) } else { (a0 - a2).rem_euclid(TAU) };
    for k in 0..4 {
        let q = f64::from(k) * FRAC_PI_2;
        let from_start = if ccw { (q - a0).rem_euclid(TAU) } else { (a0 - q).rem_euclid(TAU) };
        if from_start <= sweep {
            let p = match k {
                0 => [centre[0] + r, centre[1]],
                1 => [centre[0], centre[1] + r],
                2 => [centre[0] - r, centre[1]],
                _ => [centre[0], centre[1] - r],
            };
            grow(&mut out, p);
        }
    }
    out
}

/// The box of a piece: `[min x, max x, min y, max y]`.
pub(super) fn piece_box(piece: &Piece) -> [f64; 4] {
    match piece {
        Piece::Line(pts) => {
            let mut b = empty_box();
            for p in pts {
                grow(&mut b, *p);
            }
            b
        }
        Piece::Arcs(pts) => {
            let mut b = empty_box();
            for arc in pts.windows(3).step_by(2) {
                union(&mut b, arc_box(arc[0], arc[1], arc[2]));
            }
            b
        }
    }
}

pub(super) fn union(into: &mut [f64; 4], other: [f64; 4]) {
    into[0] = into[0].min(other[0]);
    into[1] = into[1].max(other[1]);
    into[2] = into[2].min(other[2]);
    into[3] = into[3].max(other[3]);
}

pub(super) fn pieces_box(pieces: &[Piece]) -> [f64; 4] {
    let mut b = empty_box();
    for p in pieces {
        union(&mut b, piece_box(p));
    }
    b
}

/// Distance from `q` to the segment `a b`.
pub(super) fn segment_distance(a: Pt, b: Pt, q: Pt) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    (q[0] - a[0] - t * dx).hypot(q[1] - a[1] - t * dy)
}

/// Distance from `q` to the three-point arc.
pub(super) fn arc_distance(a: Pt, b: Pt, c: Pt, q: Pt) -> f64 {
    let ends = |q: Pt| segment_distance(a, a, q).min(segment_distance(c, c, q));
    let Some((centre, r)) = circumcircle(a, b, c) else {
        return segment_distance(a, c, q);
    };
    let angle = |p: Pt| (p[1] - centre[1]).atan2(p[0] - centre[0]);
    let ccw = counter_clockwise(a, b, c);
    let (a0, a2, aq) = (angle(a), angle(c), angle(q));
    let (sweep, from_start) = if ccw {
        ((a2 - a0).rem_euclid(TAU), (aq - a0).rem_euclid(TAU))
    } else {
        ((a0 - a2).rem_euclid(TAU), (a0 - aq).rem_euclid(TAU))
    };
    if from_start <= sweep { ((q[0] - centre[0]).hypot(q[1] - centre[1]) - r).abs() } else { ends(q) }
}

/// Distance from `q` to a piece.
pub(super) fn piece_distance(piece: &Piece, q: Pt) -> f64 {
    match piece {
        Piece::Line(pts) => {
            pts.windows(2).map(|w| segment_distance(w[0], w[1], q)).fold(f64::INFINITY, f64::min)
        }
        Piece::Arcs(pts) => {
            pts.windows(3).step_by(2).map(|w| arc_distance(w[0], w[1], w[2], q)).fold(f64::INFINITY, f64::min)
        }
    }
}
