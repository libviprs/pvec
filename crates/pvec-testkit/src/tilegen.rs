//! Header-valid tiles for the sniffers (issue #219): the first bytes of a PNG, a JPEG or a WebP, as a
//! libviprs encoder writes them, with a payload that is not a pixel. Nothing here decodes anything; a tile
//! from this module passes a header sniffer and is not an image a decoder would show.
//!
//! The module is plain `std`, so the tests of `pvec-container`, which cannot depend on this crate, include it
//! by path.
#![allow(dead_code)]

fn crc32(bytes: &[u8]) -> u32 {
    let mut c = !0u32;
    for b in bytes {
        c ^= u32::from(*b);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let from = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[from..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Bytes that are not a pixel and not the same from one tile to the next.
pub fn payload(len: usize, seed: u8) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}

/// A PNG of `width` by `height` with the given IHDR fields and `data` bytes in one IDAT (several when `idats` is
/// more than 1): the chunk framing is right and the pixel data is not.
pub fn png_with(
    width: u32,
    height: u32,
    depth: u8,
    colour: u8,
    interlace: u8,
    data: &[u8],
    idats: usize,
) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[depth, colour, 0, 0, interlace]);
    chunk(&mut out, b"IHDR", &ihdr);
    let step = data.len().div_ceil(idats.max(1)).max(1);
    for part in data.chunks(step) {
        chunk(&mut out, b"IDAT", part);
    }
    if data.is_empty() {
        chunk(&mut out, b"IDAT", &[]);
    }
    chunk(&mut out, b"IEND", &[]);
    out
}

/// A PNG tile of `edge` by `edge` for `channels` (1 gray, 2 gray and alpha, 3 RGB, 4 RGBA) at `depth`.
pub fn png(edge: u32, channels: u8, depth: u8, data: &[u8]) -> Vec<u8> {
    let colour = match channels {
        1 => 0,
        2 => 4,
        3 => 2,
        _ => 6,
    };
    png_with(edge, edge, depth, colour, 0, data, 1)
}

/// A JPEG of `width` by `height` with `nf` components: SOI, a JFIF segment, a quantisation table, `pad` bytes of
/// comment (to push the frame header out), the frame header (SOF2 when `progressive`), a Huffman table, the scan
/// header, `data` bytes of entropy-coded data and, when `eoi`, EOI.
pub fn jpeg_with(
    width: u32,
    height: u32,
    nf: u8,
    progressive: bool,
    pad: usize,
    data: &[u8],
    eoi: bool,
) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    out.extend_from_slice(&[0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xDB, 0, 67, 0]);
    out.extend((1..=64u8).collect::<Vec<_>>());
    // a comment segment is at most 65,533 bytes, so a longer pad is several
    let mut left = pad;
    while left > 0 {
        let n = left.clamp(4, 60_000);
        out.extend_from_slice(&[0xFF, 0xFE]);
        out.extend_from_slice(&((n) as u16).to_be_bytes());
        out.extend(std::iter::repeat_n(b'.', n - 2));
        left = left.saturating_sub(n + 2);
    }
    out.extend_from_slice(&[0xFF, if progressive { 0xC2 } else { 0xC0 }]);
    out.extend_from_slice(&(8 + 3 * u16::from(nf)).to_be_bytes());
    out.push(8);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.push(nf);
    for c in 0..nf {
        out.extend_from_slice(&[c + 1, 0x11, 0]);
    }
    out.extend_from_slice(&[0xFF, 0xC4, 0, 19, 0]);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&[0xFF, 0xDA, 0, 6 + 2 * u16::from(nf) as u8, nf]);
    for c in 0..nf {
        out.extend_from_slice(&[c + 1, 0]);
    }
    out.extend_from_slice(&[0, 63, 0]);
    // entropy-coded data has no marker in it: an FF is followed by 00
    out.extend(data.iter().flat_map(|b| if *b == 0xFF { vec![0xFF, 0] } else { vec![*b] }));
    if eoi {
        out.extend_from_slice(&[0xFF, 0xD9]);
    }
    out
}

/// A JPEG tile of `edge` by `edge` for gray (`nf` 1) or RGB (`nf` 3).
pub fn jpeg(edge: u32, nf: u8, progressive: bool, data: &[u8]) -> Vec<u8> {
    jpeg_with(edge, edge, nf, progressive, 0, data, true)
}

fn riff(chunks: &[u8]) -> Vec<u8> {
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&((4 + chunks.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(chunks);
    out
}

fn webp_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A lossy WebP (`VP8 `) of `width` by `height`, keyframe, with `data` bytes after the frame header.
pub fn webp_vp8(width: u16, height: u16, data: &[u8]) -> Vec<u8> {
    let mut frame = vec![0x10, 0x02, 0x00, 0x9D, 0x01, 0x2A];
    frame.extend_from_slice(&width.to_le_bytes());
    frame.extend_from_slice(&height.to_le_bytes());
    frame.extend_from_slice(data);
    riff(&webp_chunk(b"VP8 ", &frame))
}

/// A lossless WebP (`VP8L`) of `width` by `height`, with `data` bytes after the header.
pub fn webp_vp8l(width: u32, height: u32, data: &[u8]) -> Vec<u8> {
    let bits = (width - 1) | ((height - 1) << 14);
    let mut frame = vec![0x2F];
    frame.extend_from_slice(&bits.to_le_bytes());
    frame.extend_from_slice(data);
    riff(&webp_chunk(b"VP8L", &frame))
}

/// An extended WebP (`VP8X`) with a canvas of `width` by `height`, the animation flag when `animated`, and an
/// image chunk (`VP8L` when `lossless`, else `VP8 `) of the same size, or an `ANIM` and `ANMF` pair when animated.
pub fn webp_vp8x(width: u32, height: u32, animated: bool, lossless: bool, data: &[u8]) -> Vec<u8> {
    let mut head = vec![if animated { 0x02 } else { 0x10 }, 0, 0, 0];
    head.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    head.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
    let mut chunks = webp_chunk(b"VP8X", &head);
    if animated {
        chunks.extend(webp_chunk(b"ANIM", &[0; 6]));
        chunks.extend(webp_chunk(b"ANMF", data));
    } else {
        let inner = if lossless {
            webp_vp8l(width, height, data)
        } else {
            webp_vp8(width as u16, height as u16, data)
        };
        chunks.extend_from_slice(&inner[12..]);
    }
    riff(&chunks)
}
