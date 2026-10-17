//! Variable-length records: the composite record and its segments. The same segment encoding is used
//! for every kind of composite, so P3.3 adds segment tags here without touching the sections.

use crate::error::{DecodeError, EncodeError};
use crate::rows::{build_fixed, fixed_row};
use crate::section::Section;
use crate::wire::{Reader, Res, Writer};
use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Bezier, Composite, Curve, Curve2d, Point2, Polyline, RationalBSpline, Trim, Trimmed,
};
use pvec_numeric::{
    MAX_COMPOSITE_SEGMENTS, MAX_CONTROL_POINTS, MAX_DEGREE, MAX_KNOTS, MAX_POLYLINE_VERTICES,
};

/// Segment tags. Zero is not a tag.
const TAG_TRIMMED_LINE: u32 = 1;
const TAG_TRIMMED_CIRCLE: u32 = 2;
const TAG_TRIMMED_ELLIPSE: u32 = 3;
const TAG_POLYLINE: u32 = 4;
/// Bezier, B-spline and rational B-spline segments are tags 5, 6 and 7 (this base plus the basis tag
/// 1, 2 or 3), and their trimmed forms are tags 8, 9 and 10.
const TAG_SPLINE_BASE: u32 = 4;
const TAG_TRIMMED_SPLINE_BASE: u32 = 7;

fn fixed_tag(s: Section) -> Option<u32> {
    match s {
        Section::TrimmedLine => Some(TAG_TRIMMED_LINE),
        Section::TrimmedCircle => Some(TAG_TRIMMED_CIRCLE),
        Section::TrimmedEllipse => Some(TAG_TRIMMED_ELLIPSE),
        _ => None,
    }
}

fn tag_section(tag: u32) -> Option<Section> {
    match tag {
        TAG_TRIMMED_LINE => Some(Section::TrimmedLine),
        TAG_TRIMMED_CIRCLE => Some(Section::TrimmedCircle),
        TAG_TRIMMED_ELLIPSE => Some(Section::TrimmedEllipse),
        _ => None,
    }
}

/// Appends one composite record: the segment count, then each segment as a tag and its payload.
pub(crate) fn write_composite(w: &mut Writer, c: &Composite) -> Result<(), EncodeError> {
    w.u32(c.segments().len() as u32);
    w.u32(0);
    for seg in c.segments() {
        if let Some(row) = fixed_row(seg) {
            let Some(tag) = fixed_tag(row.section) else { return Err(EncodeError::Unsupported(seg.kind())) };
            w.u32(tag);
            w.u32(u32::from(row.reversed));
            w.f64s(row.vals[..row.n].iter().copied());
        } else if let Curve::Polyline(p) = seg {
            w.u32(TAG_POLYLINE);
            w.u32(p.vertices().len() as u32);
            w.f64s(p.vertices().iter().flat_map(|q| [q.x, q.y]));
        } else if let Some(k) = spline_kind(seg) {
            w.u32(TAG_SPLINE_BASE + basis_tag_of(k));
            w.u32(0);
            write_spline(w, seg)?;
        } else if let (Some(k), Curve::Trimmed(t)) = (trimmed_spline_kind(seg), seg) {
            write_trimmed_spline(w, t, TAG_TRIMMED_SPLINE_BASE + basis_tag_of(k))?;
        } else {
            return Err(EncodeError::Unsupported(seg.kind()));
        }
    }
    Ok(())
}

/// Reads one composite record from `record`, which must be consumed exactly.
pub(crate) fn read_composite(record: &[u8], id: EntityId, closed: bool) -> Res<Composite> {
    let geo = |error| DecodeError::Geometry { id, error };
    let mut r = Reader::new(record);
    let n = r.u32()? as usize;
    if r.u32()? != 0 {
        return Err(DecodeError::ReservedNotZero);
    }
    if n == 0 || n > MAX_COMPOSITE_SEGMENTS {
        return Err(DecodeError::BadRecord);
    }
    // Each segment takes at least 8 bytes (tag and one more word); a count the record cannot hold is a
    // lie, and it is caught before the vector is sized.
    if n > r.remaining() / 8 {
        return Err(DecodeError::BadRecord);
    }
    let mut segs = Vec::with_capacity(n);
    for _ in 0..n {
        let tag = r.u32()?;
        let aux = r.u32()?;
        let curve = if let Some(section) = tag_section(tag) {
            if aux > 1 {
                return Err(DecodeError::BadFlags);
            }
            let (width, _) = section.fixed_shape().ok_or(DecodeError::BadTag(tag))?;
            let vals = r.f64s(width)?;
            build_fixed(section, &vals, aux == 1).map_err(geo)?
        } else if (TAG_SPLINE_BASE + 1..=TAG_SPLINE_BASE + 3).contains(&tag) {
            if aux != 0 {
                return Err(DecodeError::ReservedNotZero);
            }
            read_spline(&mut r, kind_from_basis_tag(tag - TAG_SPLINE_BASE)?, id)?
        } else if (TAG_TRIMMED_SPLINE_BASE + 1..=TAG_TRIMMED_SPLINE_BASE + 3).contains(&tag) {
            read_trimmed_spline(&mut r, kind_from_basis_tag(tag - TAG_TRIMMED_SPLINE_BASE)?, id, aux)?
        } else if tag == TAG_POLYLINE {
            let nv = aux as usize;
            if !(2..=MAX_POLYLINE_VERTICES).contains(&nv) || nv > r.remaining() / 16 {
                return Err(DecodeError::BadRecord);
            }
            let xy = r.f64s(2 * nv)?;
            let pts = xy.chunks_exact(2).map(|p| Point2::new(p[0], p[1])).collect();
            Curve::Polyline(Polyline::new(pts).map_err(geo)?)
        } else {
            return Err(DecodeError::BadTag(tag));
        };
        segs.push(curve);
    }
    if r.remaining() != 0 {
        return Err(DecodeError::BadRecord);
    }
    let c = Composite::new(segs).map_err(geo)?;
    if closed { c.declare_closed().map_err(geo) } else { Ok(c) }
}

// ---------------------------------------------------------------- spline records

/// Which spline kind a record holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SplineKind {
    Bezier,
    BSpline,
    Rational,
}

impl SplineKind {
    /// The basis tag of a trimmed spline: 1, 2 or 3.
    fn basis_tag(self) -> u32 {
        match self {
            SplineKind::Bezier => 1,
            SplineKind::BSpline => 2,
            SplineKind::Rational => 3,
        }
    }

    fn from_basis_tag(tag: u32) -> Option<SplineKind> {
        match tag {
            1 => Some(SplineKind::Bezier),
            2 => Some(SplineKind::BSpline),
            3 => Some(SplineKind::Rational),
            _ => None,
        }
    }
}

/// The spline kind of a curve, if it is one.
pub(crate) fn spline_kind(c: &Curve) -> Option<SplineKind> {
    match c {
        Curve::Bezier(_) => Some(SplineKind::Bezier),
        Curve::BSpline(_) => Some(SplineKind::BSpline),
        Curve::RationalBSpline(_) => Some(SplineKind::Rational),
        _ => None,
    }
}

/// Appends a spline record: degree, control point count, knot count and flags (bit 0: declared
/// closed) as four u32, then the control points, the knots and, for a rational curve, the weights.
/// A Bezier curve has no knot array and its degree is its point count minus one.
pub(crate) fn write_spline(w: &mut Writer, c: &Curve) -> Result<(), EncodeError> {
    let (degree, cps, knots, weights, closed): (usize, &[Point2], &[f64], &[f64], bool) = match c {
        Curve::Bezier(b) => (b.degree(), b.control_points(), &[], &[], b.is_declared_closed()),
        Curve::BSpline(s) => (s.degree(), s.control_points(), s.knots(), &[], s.is_declared_closed()),
        Curve::RationalBSpline(s) => {
            (s.degree(), s.control_points(), s.knots(), s.weights(), s.is_declared_closed())
        }
        other => return Err(EncodeError::Unsupported(other.kind())),
    };
    w.u32(degree as u32);
    w.u32(cps.len() as u32);
    w.u32(knots.len() as u32);
    w.u32(u32::from(closed));
    w.f64s(cps.iter().flat_map(|p| [p.x, p.y]));
    w.f64s(knots.iter().copied());
    w.f64s(weights.iter().copied());
    Ok(())
}

/// Reads a spline record of the given kind. Every count is checked against the bytes left before the
/// arrays are read, and the arrays go through the constructors, which check the knot vector (NP-14),
/// the weights (NP-15), the hull (NP-08) and the values (NP-01, NP-02).
pub(crate) fn read_spline(r: &mut Reader<'_>, kind: SplineKind, id: EntityId) -> Res<Curve> {
    let geo = |error| DecodeError::Geometry { id, error };
    let degree = r.u32()? as usize;
    let ncps = r.u32()? as usize;
    let nknots = r.u32()? as usize;
    let flags = r.u32()?;
    if flags > 1 {
        return Err(DecodeError::BadFlags);
    }
    let max_cps = if kind == SplineKind::Bezier { MAX_DEGREE + 1 } else { MAX_CONTROL_POINTS };
    if ncps < 2 || ncps > max_cps || nknots > MAX_KNOTS {
        return Err(DecodeError::BadRecord);
    }
    if kind == SplineKind::Bezier && (nknots != 0 || degree + 1 != ncps) {
        return Err(DecodeError::BadRecord);
    }
    let xy = r.f64s(2 * ncps)?;
    let knots = r.f64s(nknots)?;
    let weights = if kind == SplineKind::Rational { r.f64s(ncps)? } else { Vec::new() };
    let pts: Vec<Point2> = xy.chunks_exact(2).map(|p| Point2::new(p[0], p[1])).collect();
    let curve = match kind {
        SplineKind::Bezier => {
            let b = Bezier::new(pts).map_err(geo)?;
            Curve::Bezier(if flags == 1 { b.declare_closed().map_err(geo)? } else { b })
        }
        SplineKind::BSpline => {
            let s = BSpline::new(degree, pts, knots).map_err(geo)?;
            Curve::BSpline(if flags == 1 { s.declare_closed().map_err(geo)? } else { s })
        }
        SplineKind::Rational => {
            let s = RationalBSpline::new(degree, pts, knots, weights).map_err(geo)?;
            Curve::RationalBSpline(if flags == 1 { s.declare_closed().map_err(geo)? } else { s })
        }
    };
    Ok(curve)
}

/// Appends the body of a trimmed spline: a first word (the basis tag, or the segment tag in a
/// composite), the reversed flag, the trim start and span, then the basis as a spline record.
pub(crate) fn write_trimmed_spline(w: &mut Writer, t: &Trimmed, first_word: u32) -> Result<(), EncodeError> {
    let trim = t.trim();
    w.u32(first_word);
    w.u32(u32::from(trim.reversed));
    w.f64(trim.start);
    w.f64(trim.span);
    write_spline(w, t.basis())
}

/// The basis kind of a trimmed spline, if its basis is a spline.
pub(crate) fn trimmed_spline_kind(c: &Curve) -> Option<SplineKind> {
    match c {
        Curve::Trimmed(t) => spline_kind(t.basis()),
        _ => None,
    }
}

/// The basis tag to write for a trimmed spline.
pub(crate) fn basis_tag_of(k: SplineKind) -> u32 {
    k.basis_tag()
}

/// Reads the rest of a trimmed spline whose basis kind and reversed word have been read: the trim
/// start and span, then the basis as a spline record.
pub(crate) fn read_trimmed_spline(
    r: &mut Reader<'_>,
    kind: SplineKind,
    id: EntityId,
    reversed_word: u32,
) -> Res<Curve> {
    let reversed = match reversed_word {
        0 => false,
        1 => true,
        _ => return Err(DecodeError::BadFlags),
    };
    let start = r.f64()?;
    let span = r.f64()?;
    let basis = read_spline(r, kind, id)?;
    let t = Trimmed::from_normalised(basis, Trim { start, span, reversed })
        .map_err(|error| DecodeError::Geometry { id, error })?;
    Ok(Curve::Trimmed(t))
}

/// Resolves the first word of a trimmed spline record in a standalone section.
pub(crate) fn kind_from_basis_tag(tag: u32) -> Res<SplineKind> {
    SplineKind::from_basis_tag(tag).ok_or(DecodeError::BadTag(tag))
}
