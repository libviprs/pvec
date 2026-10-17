//! The encoder: entities in, one canonical block out.

use crate::error::EncodeError;
use crate::records::{
    basis_tag_of, trimmed_spline_kind, write_composite, write_spline, write_trimmed_spline,
};
use crate::rows::fixed_row;
use crate::section::Section;
use crate::wire::Writer;
use crate::{FORMAT_VERSION, MAGIC, capabilities_of};
use pvec_capabilities::CapabilityId;
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Curve2d};
use pvec_numeric::MAX_ENTITIES;
use std::collections::{BTreeMap, BTreeSet};

/// A block and what it needs from a reader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    /// The block payload.
    pub bytes: Vec<u8>,
    /// The capabilities a reader needs to understand every entity in the block, sorted by id, with the
    /// bases of trimmed curves and the segments of composites included.
    pub capabilities: Vec<CapabilityId>,
}

fn section_of(c: &Curve) -> Result<Section, EncodeError> {
    if let Some(row) = fixed_row(c) {
        return Ok(row.section);
    }
    match c {
        Curve::Polyline(_) => Ok(Section::Polyline),
        Curve::Composite(_) => Ok(Section::Composite),
        Curve::Bezier(_) => Ok(Section::Bezier),
        Curve::BSpline(_) => Ok(Section::BSpline),
        Curve::RationalBSpline(_) => Ok(Section::RationalBSpline),
        Curve::Trimmed(_) if trimmed_spline_kind(c).is_some() => Ok(Section::TrimmedSpline),
        other => Err(EncodeError::Unsupported(other.kind())),
    }
}

/// Encodes entities into one block. The result is canonical: sections in ascending kind order, the
/// input order kept within a kind, all padding zero, so the same input always gives the same bytes.
pub fn encode(entities: &[(EntityId, Curve)]) -> Result<Encoded, EncodeError> {
    if entities.len() > MAX_ENTITIES {
        return Err(EncodeError::TooMany);
    }
    let mut seen: Vec<EntityId> = entities.iter().map(|(i, _)| *i).collect();
    seen.sort_unstable();
    if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
        return Err(EncodeError::DuplicateId(w[0]));
    }

    let mut groups: BTreeMap<Section, Vec<&(EntityId, Curve)>> = BTreeMap::new();
    for e in entities {
        groups.entry(section_of(&e.1)?).or_default().push(e);
    }

    let mut w = Writer::default();
    w.buf.extend_from_slice(&MAGIC);
    w.u16(FORMAT_VERSION);
    w.u16(0); // flags
    w.u32(0); // total length, patched below
    w.u32(entities.len() as u32);
    w.u16(groups.len() as u16);
    w.buf.extend_from_slice(&[0; 6]); // reserved
    for (section, rows) in &groups {
        w.u16(section.id());
        w.u16(0);
        w.u32(rows.len() as u32);
    }

    for (section, rows) in &groups {
        w.u32s(rows.iter().map(|(i, _)| i.index()));
        match section {
            Section::Polyline => {
                let polys: Vec<&pvec_geometry::Polyline> = rows
                    .iter()
                    .filter_map(|(_, c)| if let Curve::Polyline(p) = c { Some(p) } else { None })
                    .collect();
                let mut offs = Vec::with_capacity(polys.len() + 1);
                let mut total: u32 = 0;
                offs.push(0);
                for p in &polys {
                    total = total.checked_add(p.vertices().len() as u32).ok_or(EncodeError::TooLarge)?;
                    offs.push(total);
                }
                w.u32s(offs);
                w.f64s(polys.iter().flat_map(|p| p.vertices().iter().map(|q| q.x)));
                w.f64s(polys.iter().flat_map(|p| p.vertices().iter().map(|q| q.y)));
            }
            Section::Bezier | Section::BSpline | Section::RationalBSpline => {
                arena_section(&mut w, rows, write_spline)?;
            }
            Section::TrimmedSpline => {
                arena_section(&mut w, rows, |a, c| match (trimmed_spline_kind(c), c) {
                    (Some(k), Curve::Trimmed(t)) => write_trimmed_spline(a, t, basis_tag_of(k)),
                    _ => Err(EncodeError::Unsupported(c.kind())),
                })?;
            }
            Section::Composite => {
                let comps: Vec<&pvec_geometry::Composite> = rows
                    .iter()
                    .filter_map(|(_, c)| if let Curve::Composite(k) = c { Some(k) } else { None })
                    .collect();
                w.u8s(comps.iter().map(|c| u8::from(c.is_declared_closed())));
                let mut arena = Writer::default();
                let mut offs: Vec<u32> = vec![0];
                for c in &comps {
                    write_composite(&mut arena, c)?;
                    offs.push(u32::try_from(arena.buf.len()).map_err(|_| EncodeError::TooLarge)?);
                }
                w.u32s(offs);
                w.buf.extend_from_slice(&arena.buf);
                w.pad8();
            }
            fixed => {
                let rows: Vec<_> = rows.iter().filter_map(|(_, c)| fixed_row(c)).collect();
                let (width, has_flags) = fixed.fixed_shape().ok_or(EncodeError::TooLarge)?;
                for j in 0..width {
                    w.f64s(rows.iter().map(|r| r.vals[j]));
                }
                if has_flags {
                    w.u8s(rows.iter().map(|r| u8::from(r.reversed)));
                }
            }
        }
    }

    let total = u32::try_from(w.buf.len()).map_err(|_| EncodeError::TooLarge)?;
    w.buf[8..12].copy_from_slice(&total.to_le_bytes());

    let mut caps: BTreeSet<CapabilityId> = BTreeSet::new();
    for (_, c) in entities {
        caps.extend(capabilities_of(c));
    }
    Ok(Encoded { bytes: w.buf, capabilities: caps.into_iter().collect() })
}

/// Writes the offsets and the arena of a section whose entities are variable-length records.
fn arena_section(
    w: &mut Writer,
    rows: &[&(EntityId, Curve)],
    mut write: impl FnMut(&mut Writer, &Curve) -> Result<(), EncodeError>,
) -> Result<(), EncodeError> {
    let mut arena = Writer::default();
    let mut offs: Vec<u32> = vec![0];
    for (_, c) in rows {
        write(&mut arena, c)?;
        offs.push(u32::try_from(arena.buf.len()).map_err(|_| EncodeError::TooLarge)?);
    }
    w.u32s(offs);
    w.buf.extend_from_slice(&arena.buf);
    w.pad8();
    Ok(())
}
