//! A test-only tiler (issue #218): cuts a synthetic image into the tiles of a raster pyramid and encodes each
//! one as a valid PNG whose deflate stream is stored, not compressed.
//!
//! No image codec enters any crate: a PNG with stored blocks is a few dozen lines of framing, a CRC and an
//! Adler sum, and every decoder reads it. It is what the raster tests, the fuzz seeds and the measurement of
//! issue #220 draw from, and what the WASM exports and the MapLibre layer draw in their own tests. The image
//! is a function of the pixel, so two runs give the same bytes, and a tile can be compared with the pixels it
//! was cut from.
//!
//! The pyramid is the one the placement fixes: `levels` levels, the last at full size, each one above it
//! halved with a ceiling, the pyramid stopping at the first level of one tile, tiles anchored at pixel (0, 0)
//! with the edge tiles padded with zero.

use pvec_container::blocks::raster::{TileIn, grid_of};

/// The shape of a synthetic raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    /// Pixels across the full-resolution level.
    pub width: u32,
    /// Pixels down the full-resolution level.
    pub height: u32,
    /// The tile edge, a power of two from 256 to 1024.
    pub tile_edge: u32,
    /// 1 gray, 2 gray and alpha, 3 RGB, 4 RGBA.
    pub channels: u8,
    /// 1 (gray only) or 8.
    pub bit_depth: u8,
}

impl Shape {
    /// The number of levels: the pyramid stops at the first level of one tile.
    pub fn levels(&self) -> u32 {
        let mut n = 1u32;
        while grid_of(self.width, self.height, self.tile_edge, n, 0) != (1, 1) {
            n += 1;
        }
        n
    }

    /// `(cols, rows)` of each level, the coarsest first.
    pub fn grid(&self) -> Vec<(u32, u32)> {
        let n = self.levels();
        (0..n).map(|l| grid_of(self.width, self.height, self.tile_edge, n, l)).collect()
    }
}

/// The pixel at `(x, y)` of the full-resolution image, as `channels` bytes: a gradient with a coarse grid on
/// top, so no two tiles are alike and a tile moved to the wrong place is seen.
pub fn pixel(x: u32, y: u32, channels: u8, bit_depth: u8) -> [u8; 4] {
    let g = ((x / 8 + y / 8) % 2) as u8 * 96 + (x / 3 % 128) as u8;
    let v = [g, (y / 5 % 251) as u8, (x ^ y) as u8, 255 - (x / 64 % 200) as u8];
    if bit_depth == 1 {
        let bit = u8::from((x / 16 + y / 16) % 2 == 0 || x % 7 == 0);
        return [bit, 0, 0, 0];
    }
    match channels {
        1 => [v[0], 0, 0, 0],
        2 => [v[0], v[3], 0, 0],
        3 => [v[0], v[1], v[2], 0],
        _ => v,
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, slot) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    !bytes.iter().fold(!0u32, |c, b| table[((c ^ u32::from(*b)) & 0xFF) as usize] ^ (c >> 8))
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for x in bytes {
        a = (a + u32::from(*x)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let from = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[from..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A PNG of `edge` by `edge` pixels whose pixel `(x, y)` is `at(x, y)` (the first `channels` bytes of it; for a
/// bit depth of 1 the first byte, 0 or 1), the deflate stream stored in blocks of at most 65,535 bytes.
pub fn png(edge: u32, channels: u8, bit_depth: u8, at: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let colour = match channels {
        1 => 0u8,
        2 => 4,
        3 => 2,
        _ => 6,
    };
    let row_bytes = if bit_depth == 1 {
        (edge as usize).div_ceil(8)
    } else {
        edge as usize * usize::from(channels)
    };
    let mut raw = Vec::with_capacity((row_bytes + 1) * edge as usize);
    for y in 0..edge {
        raw.push(0);
        if bit_depth == 1 {
            let mut byte = 0u8;
            for x in 0..edge {
                byte = (byte << 1) | (at(x, y)[0] & 1);
                if x % 8 == 7 {
                    raw.push(byte);
                    byte = 0;
                }
            }
            if edge % 8 != 0 {
                raw.push(byte << (8 - edge % 8));
            }
        } else {
            for x in 0..edge {
                raw.extend_from_slice(&at(x, y)[..usize::from(channels)]);
            }
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(65_535).peekable();
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(b.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(b.len() as u16)).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&edge.to_be_bytes());
    ihdr.extend_from_slice(&edge.to_be_bytes());
    ihdr.extend_from_slice(&[bit_depth, colour, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// The tiles of a pyramid, in the order a producer might emit them (level by level, row by row). `keep` says
/// which `(level, col, row)` are stored; the others are tiles with no bytes, which is how a sparse pyramid is
/// handed over.
pub fn tiles(shape: Shape, keep: impl Fn(u32, u32, u32) -> bool) -> Vec<TileIn> {
    let n = shape.levels();
    let mut out = Vec::new();
    for (l, (cols, rows)) in shape.grid().into_iter().enumerate() {
        let l = l as u32;
        // a pixel of level `l` is `scale` pixels of the full-resolution image
        let scale = 1u32 << (n - 1 - l).min(31);
        let (lw, lh) = (shape.width.div_ceil(scale), shape.height.div_ceil(scale));
        for row in 0..rows {
            for col in 0..cols {
                let bytes = if keep(l, col, row) {
                    png(shape.tile_edge, shape.channels, shape.bit_depth, |x, y| {
                        let (px, py) = (col * shape.tile_edge + x, row * shape.tile_edge + y);
                        if px < lw && py < lh {
                            pixel(px * scale, py * scale, shape.channels, shape.bit_depth)
                        } else {
                            [0; 4]
                        }
                    })
                } else {
                    Vec::new()
                };
                out.push(TileIn { level: l, col, row, bytes });
            }
        }
    }
    out
}
