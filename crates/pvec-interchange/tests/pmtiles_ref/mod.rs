//! An independent PMTiles v3 reader, written from the specification. It shares no code with the writer in
//! `src/pmtiles.rs`. Used by `pmtiles.rs` (vector tiles) and `issue_222_raster_pmtiles.rs` (raster tiles).
#![allow(dead_code)]

pub struct Hdr {
    pub root: (u64, u64),
    pub meta: (u64, u64),
    pub leaves: (u64, u64),
    pub data: (u64, u64),
    pub addressed: u64,
    pub entries: u64,
    pub contents: u64,
    pub clustered: u8,
    pub internal_compression: u8,
    pub tile_compression: u8,
    pub tile_type: u8,
    pub min_zoom: u8,
    pub max_zoom: u8,
}

pub fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

pub fn header(b: &[u8]) -> Hdr {
    assert!(b.len() >= 127, "shorter than the header");
    assert_eq!(&b[0..7], b"PMTiles");
    assert_eq!(b[7], 3, "version");
    Hdr {
        root: (u64_at(b, 8), u64_at(b, 16)),
        meta: (u64_at(b, 24), u64_at(b, 32)),
        leaves: (u64_at(b, 40), u64_at(b, 48)),
        data: (u64_at(b, 56), u64_at(b, 64)),
        addressed: u64_at(b, 72),
        entries: u64_at(b, 80),
        contents: u64_at(b, 88),
        clustered: b[96],
        internal_compression: b[97],
        tile_compression: b[98],
        tile_type: b[99],
        min_zoom: b[100],
        max_zoom: b[101],
    }
}

pub fn varint(b: &[u8], i: &mut usize) -> u64 {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let byte = b[*i];
        *i += 1;
        v |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return v;
        }
        shift += 7;
        assert!(shift < 64);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub id: u64,
    pub offset: u64,
    pub length: u64,
    pub run: u64,
}

pub fn directory(b: &[u8]) -> Vec<Entry> {
    let mut i = 0;
    let n = varint(b, &mut i) as usize;
    let mut es = vec![Entry { id: 0, offset: 0, length: 0, run: 0 }; n];
    let mut last = 0;
    for e in es.iter_mut() {
        last += varint(b, &mut i);
        e.id = last;
    }
    for e in es.iter_mut() {
        e.run = varint(b, &mut i);
    }
    for e in es.iter_mut() {
        e.length = varint(b, &mut i);
    }
    for k in 0..n {
        let v = varint(b, &mut i);
        es[k].offset = if v == 0 && k > 0 { es[k - 1].offset + es[k - 1].length } else { v - 1 };
    }
    assert_eq!(i, b.len(), "trailing bytes in a directory");
    es
}

pub fn range(a: &[u8], (off, len): (u64, u64)) -> &[u8] {
    &a[off as usize..(off + len) as usize]
}

/// Looks one tile id up the way a client does: root first, then a leaf if the root points at one.
pub fn find(a: &[u8], id: u64) -> Option<&[u8]> {
    let h = header(a);
    let mut dir = directory(range(a, h.root));
    for _depth in 0..4 {
        let pos = dir.partition_point(|e| e.id <= id);
        if pos == 0 {
            return None;
        }
        let e = dir[pos - 1];
        if e.run > 0 {
            return (id < e.id + e.run).then(|| range(a, (h.data.0 + e.offset, e.length)));
        }
        dir = directory(range(a, (h.leaves.0 + e.offset, e.length)));
    }
    panic!("directories nested too deep");
}

/// Every (id, bytes) of the archive, walking root and leaves.
pub fn all_tiles(a: &[u8]) -> Vec<(u64, Vec<u8>)> {
    let h = header(a);
    let mut out = Vec::new();
    let mut stack = vec![directory(range(a, h.root))];
    while let Some(dir) = stack.pop() {
        for e in dir {
            if e.run == 0 {
                stack.push(directory(range(a, (h.leaves.0 + e.offset, e.length))));
            } else {
                for k in 0..e.run {
                    out.push((e.id + k, range(a, (h.data.0 + e.offset, e.length)).to_vec()));
                }
            }
        }
    }
    out.sort();
    out
}

/// The tile id of the PMTiles specification, from its reference algorithm (the Hilbert curve of
/// Wikipedia's `xy2d`, stacked by zoom).
pub fn ref_tile_id(z: u32, x: u64, y: u64) -> u64 {
    let acc: u64 = (0..z).map(|i| 4u64.pow(i)).sum();
    let n = 1u64 << z;
    let (mut x, mut y, mut d) = (x as i64, y as i64, 0u64);
    let mut s = n / 2;
    while s > 0 {
        let rx = i64::from((x & s as i64) > 0);
        let ry = i64::from((y & s as i64) > 0);
        d += s * s * ((3 * rx) ^ ry) as u64;
        if ry == 0 {
            if rx == 1 {
                x = n as i64 - 1 - x;
                y = n as i64 - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    acc + d
}
