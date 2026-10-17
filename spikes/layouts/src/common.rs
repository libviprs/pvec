//! What every candidate shares: the block-addressed file container, the Morton block assignment,
//! the order-independent "touch" workload, and a 16-byte aligned buffer.
//!
//! Only the block payloads differ between candidates. The framing, the directory and the block
//! assignment are identical, so every difference in the tables is the layout and nothing else.

use pvec_fixtures::{Drawing, Entity, Geom, overlaps};

/// Entities per block. A parameter of the spike, not a recommendation (P1.6 and P4.4 own it).
pub const BLOCK: usize = 512;

/// A buffer whose start is aligned to 16 bytes, so zero-copy readers see aligned data wherever a
/// block starts on a 16-byte boundary within the file.
pub struct Aligned {
    buf: Vec<u128>,
    len: usize,
}

impl Aligned {
    /// Copies `bytes` into a fresh aligned buffer.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        let mut a = Aligned { buf: vec![0u128; bytes.len().div_ceil(16)], len: bytes.len() };
        a.as_mut_bytes().copy_from_slice(bytes);
        a
    }
    /// A zeroed buffer of `len` bytes.
    pub fn zeroed(len: usize) -> Self {
        Aligned { buf: vec![0u128; len.div_ceil(16)], len }
    }
    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `buf` owns `buf.len() * 16 >= len` initialised bytes, `u8` has alignment 1, and
        // the returned slice borrows `self`, so the allocation outlives it.
        unsafe { core::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>(), self.len) }
    }
    /// The bytes, mutably.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: as in `as_bytes`, and `&mut self` guarantees exclusive access.
        unsafe { core::slice::from_raw_parts_mut(self.buf.as_mut_ptr().cast::<u8>(), self.len) }
    }
}

/// One candidate's block codec.
pub trait Codec {
    /// Name used in tables.
    const NAME: &'static str;
    /// Encodes the entities of one block.
    fn encode_block(ents: &[&Entity]) -> Vec<u8>;
    /// Walks every value of a block and returns the order-independent touch sum.
    /// Untrusted bytes: must never panic and must bounds-check every offset.
    fn touch(block: &[u8]) -> Result<u64, String>;
    /// Materialises every entity of a block.
    fn decode(block: &[u8]) -> Result<Vec<Entity>, String>;
}

/// Order-independent touch of one geometry: wrapping sum of the bit patterns of every float, plus
/// the spline degree and a 0 or 1 per composite part. Each layout must reproduce it by walking its
/// own representation.
pub fn touch_geom(g: &Geom) -> u64 {
    let f = |x: &f64| x.to_bits();
    let s = |v: &[f64]| v.iter().fold(0u64, |a, x| a.wrapping_add(f(x)));
    let p = |v: &[[f64; 2]]| v.iter().fold(0u64, |a, q| a.wrapping_add(f(&q[0])).wrapping_add(f(&q[1])));
    match g {
        Geom::Line { a, b } => p(&[*a, *b]),
        Geom::Circle { c, r } => p(&[*c]).wrapping_add(f(r)),
        Geom::Arc { c, r, a0, a1 } => p(&[*c]).wrapping_add(s(&[*r, *a0, *a1])),
        Geom::Ellipse { c, a, b, rot, t0, t1 } => p(&[*c]).wrapping_add(s(&[*a, *b, *rot, *t0, *t1])),
        Geom::Polyline { pts } => p(pts),
        Geom::Spline { degree, cps, knots, weights } => p(cps)
            .wrapping_add(s(knots))
            .wrapping_add(weights.as_deref().map_or(0, s))
            .wrapping_add(*degree as u64),
        Geom::Composite { parts } => parts.iter().fold(0u64, |a, q| {
            a.wrapping_add(touch_geom(q)).wrapping_add(u64::from(matches!(q, Geom::Arc { .. })))
        }),
    }
}

/// Touch of one entity: its id plus its geometry.
pub fn touch_entity(e: &Entity) -> u64 {
    (e.id as u64).wrapping_add(touch_geom(&e.geom))
}

/// Morton (Z-order) key of the bbox centre, 16 bits per axis over the drawing extent.
fn morton(d: &Drawing, e: &Entity) -> u32 {
    let b = e.geom.bbox();
    let [x0, y0, x1, y1] = d.extent;
    let q = |v: f64, lo: f64, hi: f64| (((v - lo) / (hi - lo)).clamp(0.0, 1.0) * 65535.0) as u32;
    let (mut x, mut y) = (q((b[0] + b[2]) / 2.0, x0, x1), q((b[1] + b[3]) / 2.0, y0, y1));
    let spread = |mut v: u32| {
        v = (v | (v << 8)) & 0x00ff_00ff;
        v = (v | (v << 4)) & 0x0f0f_0f0f;
        v = (v | (v << 2)) & 0x3333_3333;
        (v | (v << 1)) & 0x5555_5555
    };
    x = spread(x);
    y = spread(y);
    x | (y << 1)
}

/// Entities in Morton order, chunked into blocks of [`BLOCK`].
pub fn blocks(d: &Drawing) -> Vec<Vec<&Entity>> {
    let mut order: Vec<(u32, u32)> = d.entities.iter().map(|e| (morton(d, e), e.id)).collect();
    order.sort_unstable();
    order.chunks(BLOCK).map(|c| c.iter().map(|&(_, id)| &d.entities[id as usize]).collect()).collect()
}

/// One directory entry: where a block sits and what it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirEntry {
    /// Byte offset of the block in the file.
    pub off: u64,
    /// Block length in bytes (without alignment padding).
    pub len: u64,
    /// Entities in the block.
    pub nent: u32,
    /// Union of the entities' source-space bounds.
    pub bbox: [f64; 4],
}

/// Header bytes: magic, version, block count, directory offset and length.
pub const HEADER: usize = 32;
/// Bytes per directory entry.
pub const DIR_ENTRY: usize = 56;
const MAGIC: &[u8; 8] = b"PVECLAY1";

fn align16(n: usize) -> usize {
    n.div_ceil(16) * 16
}

/// Builds a whole file: header, 16-byte aligned blocks, directory.
pub fn build_file<C: Codec>(blocks: &[Vec<&Entity>]) -> Vec<u8> {
    let mut out = vec![0u8; HEADER];
    let mut dir = Vec::with_capacity(blocks.len());
    for b in blocks {
        let enc = C::encode_block(b);
        let off = align16(out.len());
        out.resize(off, 0);
        let mut bbox = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for e in b {
            let q = e.geom.bbox();
            bbox = [bbox[0].min(q[0]), bbox[1].min(q[1]), bbox[2].max(q[2]), bbox[3].max(q[3])];
        }
        dir.push(DirEntry { off: off as u64, len: enc.len() as u64, nent: b.len() as u32, bbox });
        out.extend_from_slice(&enc);
    }
    let dir_off = align16(out.len());
    out.resize(dir_off, 0);
    for e in &dir {
        out.extend_from_slice(&e.off.to_le_bytes());
        out.extend_from_slice(&e.len.to_le_bytes());
        out.extend_from_slice(&e.nent.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        for v in e.bbox {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    let dir_len = (dir.len() * DIR_ENTRY) as u64;
    out[0..8].copy_from_slice(MAGIC);
    out[8..12].copy_from_slice(&1u32.to_le_bytes());
    out[12..16].copy_from_slice(&(dir.len() as u32).to_le_bytes());
    out[16..24].copy_from_slice(&(dir_off as u64).to_le_bytes());
    out[24..32].copy_from_slice(&dir_len.to_le_bytes());
    out
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Reads and validates the header and directory. Every offset and length is checked against the
/// file size before it is used to slice, and the block count is capped before it sizes an allocation.
pub fn parse_dir(file: &[u8]) -> Result<Vec<DirEntry>, String> {
    if file.len() < HEADER || &file[0..8] != MAGIC {
        return Err("bad header".into());
    }
    let n = u32_at(file, 12).ok_or("short")? as usize;
    let dir_off = u64_at(file, 16).ok_or("short")?;
    let dir_len = u64_at(file, 24).ok_or("short")?;
    if dir_len != (n as u64).checked_mul(DIR_ENTRY as u64).ok_or("overflow")? {
        return Err("directory length does not match block count".into());
    }
    let end = dir_off.checked_add(dir_len).ok_or("overflow")?;
    if end > file.len() as u64 {
        return Err("directory outside file".into());
    }
    let mut out = Vec::with_capacity(n.min(file.len() / DIR_ENTRY));
    for i in 0..n {
        let o = dir_off as usize + i * DIR_ENTRY;
        let off = u64_at(file, o).ok_or("short")?;
        let len = u64_at(file, o + 8).ok_or("short")?;
        let nent = u32_at(file, o + 16).ok_or("short")?;
        let mut bbox = [0.0; 4];
        for (k, v) in bbox.iter_mut().enumerate() {
            *v = f64::from_bits(u64_at(file, o + 24 + 8 * k).ok_or("short")?);
        }
        if off.checked_add(len).ok_or("overflow")? > file.len() as u64 || off % 16 != 0 {
            return Err(format!("block {i} outside file or misaligned"));
        }
        out.push(DirEntry { off, len, nent, bbox });
    }
    Ok(out)
}

/// The slice of one block.
pub fn block_bytes<'a>(file: &'a [u8], e: &DirEntry) -> &'a [u8] {
    &file[e.off as usize..(e.off + e.len) as usize]
}

/// Touch of the whole file.
pub fn touch_file<C: Codec>(file: &[u8]) -> Result<(u64, u64), String> {
    let dir = parse_dir(file)?;
    let mut sum = 0u64;
    let mut n = 0u64;
    for e in &dir {
        sum = sum.wrapping_add(C::touch(block_bytes(file, e))?);
        n += e.nent as u64;
    }
    Ok((sum, n))
}

/// Materialises every entity of a file, in id order.
pub fn decode_file<C: Codec>(file: &[u8]) -> Result<Vec<Entity>, String> {
    let dir = parse_dir(file)?;
    let mut all = Vec::new();
    for e in &dir {
        all.extend(C::decode(block_bytes(file, e))?);
    }
    all.sort_by_key(|e| e.id);
    Ok(all)
}

/// Blocks a viewport rectangle touches.
pub fn blocks_hit(dir: &[DirEntry], rect: &[f64; 4]) -> Vec<usize> {
    (0..dir.len()).filter(|&i| overlaps(&dir[i].bbox, rect)).collect()
}

/// Coalesces the hit blocks into byte ranges: neighbours whose gap is only alignment padding merge.
pub fn ranges(dir: &[DirEntry], hit: &[usize]) -> Vec<(u64, u64)> {
    let mut sorted: Vec<&DirEntry> = hit.iter().map(|&i| &dir[i]).collect();
    sorted.sort_by_key(|e| e.off);
    let mut out: Vec<(u64, u64)> = Vec::new();
    for e in sorted {
        match out.last_mut() {
            Some(last) if e.off <= last.1 + 16 => last.1 = e.off + e.len,
            _ => out.push((e.off, e.off + e.len)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_directory_is_rejected_not_trusted() {
        assert!(parse_dir(&[]).is_err());
        let mut f = vec![0u8; 64];
        f[0..8].copy_from_slice(MAGIC);
        // Claims four billion blocks in a 64-byte file.
        f[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        f[24..32].copy_from_slice(&(u32::MAX as u64 * DIR_ENTRY as u64).to_le_bytes());
        assert!(parse_dir(&f).is_err());
    }

    #[test]
    fn aligned_buffer_is_aligned() {
        let a = Aligned::from_bytes(&[1, 2, 3]);
        assert_eq!(a.as_bytes().as_ptr() as usize % 16, 0);
        assert_eq!(a.as_bytes(), &[1, 2, 3]);
    }
}
