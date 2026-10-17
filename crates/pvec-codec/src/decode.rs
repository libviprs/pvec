//! The decoder. It trusts nothing in the block: the header is checked first, every count is compared
//! with the bytes that are really left before it can size an allocation, offset tables are checked for
//! shape, and every curve is rebuilt through the validating constructors of `pvec-geometry`, so a
//! malformed block cannot produce an unvalidated curve.

use crate::error::DecodeError;
use crate::records::read_composite;
use crate::rows::build_fixed;
use crate::section::Section;
use crate::wire::{Reader, Res};
use crate::{FORMAT_VERSION, MAGIC};
use pvec_entity::EntityId;
use pvec_geometry::{Curve, Point2, Polyline};
use pvec_numeric::{MAX_ENTITIES, MAX_POLYLINE_VERTICES};

struct Header {
    entity_count: u32,
    kinds: Vec<(Section, usize)>,
}

fn parse_header(bytes: &[u8]) -> Res<(Header, usize)> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = r.u16()?;
    if version != FORMAT_VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    if r.u16()? != 0 {
        return Err(DecodeError::ReservedNotZero);
    }
    let declared = r.u32()? as usize;
    let entity_count = r.u32()?;
    let kind_count = r.u16()? as usize;
    if r.take(6)?.iter().any(|b| *b != 0) {
        return Err(DecodeError::ReservedNotZero);
    }
    if declared != bytes.len() {
        return Err(DecodeError::LengthMismatch { declared, actual: bytes.len() });
    }
    if entity_count as usize >= MAX_ENTITIES {
        return Err(DecodeError::TooMany);
    }
    // The directory has to fit in the block before anything is read from it.
    if kind_count.checked_mul(8).is_none_or(|n| n > r.remaining()) {
        return Err(DecodeError::Truncated);
    }
    let mut kinds: Vec<(Section, usize)> = Vec::with_capacity(kind_count);
    let mut sum: u64 = 0;
    for _ in 0..kind_count {
        let kind = r.u16()?;
        if r.u16()? != 0 {
            return Err(DecodeError::ReservedNotZero);
        }
        let count = r.u32()? as usize;
        let section = Section::from_id(kind).ok_or(DecodeError::UnknownKind(kind))?;
        if count == 0 || kinds.last().is_some_and(|(prev, _)| *prev >= section) {
            return Err(DecodeError::BadKindDirectory);
        }
        if count >= MAX_ENTITIES {
            return Err(DecodeError::TooMany);
        }
        sum += count as u64;
        kinds.push((section, count));
    }
    if sum != u64::from(entity_count) {
        return Err(DecodeError::CountMismatch { header: entity_count, sum });
    }
    Ok((Header { entity_count, kinds }, r.pos()))
}

/// The sections present in a block and how many entities each holds, in kind order. Only the header
/// and the directory are read and checked.
pub fn describe(bytes: &[u8]) -> Result<Vec<(Section, usize)>, DecodeError> {
    parse_header(bytes).map(|(h, _)| h.kinds)
}

fn ids(r: &mut Reader<'_>, n: usize) -> Res<Vec<EntityId>> {
    r.u32s(n)?.into_iter().map(|i| EntityId::new(i).map_err(|_| DecodeError::BadId(i))).collect()
}

/// A flag byte holds only bit 0.
fn flag_bits(bytes: &[u8]) -> Res<Vec<bool>> {
    bytes
        .iter()
        .map(|b| match b {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::BadFlags),
        })
        .collect()
}

/// Checks an offset table of `n + 1` entries that starts at zero, never decreases and stays inside
/// `limit`; `step` is the smallest legal difference and `align` the required alignment of each entry.
fn check_offsets(offs: &[u32], limit: usize, step: u32, align: u32) -> Res<()> {
    if offs.first() != Some(&0) {
        return Err(DecodeError::BadOffsets);
    }
    for w in offs.windows(2) {
        if w[1] < w[0] || w[1] - w[0] < step {
            return Err(DecodeError::BadOffsets);
        }
    }
    if offs.iter().any(|o| o % align != 0 || *o as usize > limit) {
        return Err(DecodeError::BadOffsets);
    }
    Ok(())
}

/// Decodes one block into its entities, grouped by section kind in ascending order and in storage
/// order within a kind. Identity is the [`EntityId`], not the position.
pub fn decode(bytes: &[u8]) -> Result<Vec<(EntityId, Curve)>, DecodeError> {
    let (header, data_start) = parse_header(bytes)?;
    let mut r = Reader::new(bytes);
    r.take(data_start)?;
    let mut out: Vec<(EntityId, Curve)> = Vec::new();
    for (section, n) in &header.kinds {
        let n = *n;
        // Nothing is allocated from `n` before the id column has been read, and that read is checked
        // against the bytes left.
        let id_col = ids(&mut r, n)?;
        out.reserve(id_col.len());
        match section {
            Section::Polyline => {
                let voff = r.u32s(n + 1)?;
                let total = *voff.last().unwrap_or(&0) as usize;
                if total > r.remaining() / 16 {
                    // Either the offsets lie or the block is short; both are a bad table.
                    check_offsets(&voff, usize::MAX, 2, 1)?;
                    return Err(DecodeError::Truncated);
                }
                check_offsets(&voff, total, 2, 1)?;
                if voff.windows(2).any(|w| (w[1] - w[0]) as usize > MAX_POLYLINE_VERTICES) {
                    return Err(DecodeError::BadOffsets);
                }
                let xs = r.f64s(total)?;
                let ys = r.f64s(total)?;
                for (k, id) in id_col.iter().enumerate() {
                    let (a, b) = (voff[k] as usize, voff[k + 1] as usize);
                    let pts = (a..b).map(|i| Point2::new(xs[i], ys[i])).collect();
                    let p = Polyline::new(pts).map_err(|error| DecodeError::Geometry { id: *id, error })?;
                    out.push((*id, Curve::Polyline(p)));
                }
            }
            Section::Composite => {
                let closed = flag_bits(r.u8s(n)?)?;
                let offs = r.u32s(n + 1)?;
                let arena_len = *offs.last().unwrap_or(&0) as usize;
                if arena_len > r.remaining() {
                    return Err(DecodeError::Truncated);
                }
                check_offsets(&offs, arena_len, 8, 8)?;
                let arena = r.take(arena_len)?;
                for (k, id) in id_col.iter().enumerate() {
                    let record = &arena[offs[k] as usize..offs[k + 1] as usize];
                    let c = read_composite(record, *id, closed[k])?;
                    out.push((*id, Curve::Composite(c)));
                }
            }
            fixed => {
                let (width, has_flags) = fixed.fixed_shape().ok_or(DecodeError::UnknownKind(fixed.id()))?;
                let mut cols = Vec::with_capacity(width);
                for _ in 0..width {
                    cols.push(r.f64s(n)?);
                }
                let flags = if has_flags { flag_bits(r.u8s(n)?)? } else { vec![false; n] };
                let mut row = [0.0f64; 8];
                for (k, id) in id_col.iter().enumerate() {
                    for (j, c) in cols.iter().enumerate() {
                        row[j] = c[k];
                    }
                    let curve = build_fixed(*fixed, &row[..width], flags[k])
                        .map_err(|error| DecodeError::Geometry { id: *id, error })?;
                    out.push((*id, curve));
                }
            }
        }
    }
    if r.remaining() != 0 {
        return Err(DecodeError::LengthMismatch { declared: bytes.len(), actual: r.pos() });
    }
    debug_assert_eq!(out.len(), header.entity_count as usize);
    let mut seen: Vec<EntityId> = out.iter().map(|(i, _)| *i).collect();
    seen.sort_unstable();
    if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
        return Err(DecodeError::DuplicateId(w[0]));
    }
    Ok(out)
}
