//! Variable-length records: the composite record and its segments. The same segment encoding is used
//! for every kind of composite, so P3.3 adds segment tags here without touching the sections.

use crate::error::{DecodeError, EncodeError};
use crate::rows::{build_fixed, fixed_row};
use crate::section::Section;
use crate::wire::{Reader, Res, Writer};
use pvec_entity::EntityId;
use pvec_geometry::{Composite, Curve, Curve2d, Point2, Polyline};
use pvec_numeric::{MAX_COMPOSITE_SEGMENTS, MAX_POLYLINE_VERTICES};

/// Segment tags. Zero is not a tag.
const TAG_TRIMMED_LINE: u32 = 1;
const TAG_TRIMMED_CIRCLE: u32 = 2;
const TAG_TRIMMED_ELLIPSE: u32 = 3;
const TAG_POLYLINE: u32 = 4;

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
