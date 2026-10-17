//! Codecs, a byte-shuffle pre-filter and checksums, as the container would apply them per block.
#![allow(missing_docs)]

use std::io::{Read, Write};

#[cfg(target_arch = "wasm32")]
pub mod wasm;

/// Groups the bytes of each 8-byte word by position: byte 0 of every word, then byte 1, and so on.
/// The block's `f64` columns then become eight smooth byte planes. Trailing bytes that do not fill a
/// word are copied through.
pub fn shuffle8(src: &[u8]) -> Vec<u8> {
    let n = src.len() / 8;
    let mut out = vec![0u8; src.len()];
    for (i, w) in src.chunks_exact(8).enumerate() {
        for (b, v) in w.iter().enumerate() {
            out[b * n + i] = *v;
        }
    }
    out[n * 8..].copy_from_slice(&src[n * 8..]);
    out
}

pub fn unshuffle8(src: &[u8]) -> Vec<u8> {
    let n = src.len() / 8;
    let mut out = vec![0u8; src.len()];
    for i in 0..n {
        for b in 0..8 {
            out[i * 8 + b] = src[b * n + i];
        }
    }
    out[n * 8..].copy_from_slice(&src[n * 8..]);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codec {
    Deflate1,
    Deflate6,
    Lz4,
    #[cfg(not(target_arch = "wasm32"))]
    Zstd1,
    #[cfg(not(target_arch = "wasm32"))]
    Zstd3,
}

impl Codec {
    pub fn name(self) -> &'static str {
        match self {
            Codec::Deflate1 => "deflate level 1",
            Codec::Deflate6 => "deflate level 6",
            Codec::Lz4 => "LZ4 block",
            #[cfg(not(target_arch = "wasm32"))]
            Codec::Zstd1 => "zstd level 1",
            #[cfg(not(target_arch = "wasm32"))]
            Codec::Zstd3 => "zstd level 3",
        }
    }

    pub fn compress(self, data: &[u8]) -> Vec<u8> {
        match self {
            Codec::Deflate1 | Codec::Deflate6 => {
                let level = if self == Codec::Deflate1 { 1 } else { 6 };
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level));
                e.write_all(data).expect("deflate");
                e.finish().expect("deflate")
            }
            Codec::Lz4 => lz4_flex::block::compress(data),
            #[cfg(not(target_arch = "wasm32"))]
            Codec::Zstd1 => zstd::bulk::compress(data, 1).expect("zstd"),
            #[cfg(not(target_arch = "wasm32"))]
            Codec::Zstd3 => zstd::bulk::compress(data, 3).expect("zstd"),
        }
    }

    /// Decompresses to exactly `len` bytes, or returns `None`. The expected length is part of the
    /// block header in a real container, which is what bounds decompression expansion (P4.8).
    pub fn decompress(self, data: &[u8], len: usize) -> Option<Vec<u8>> {
        match self {
            Codec::Deflate1 | Codec::Deflate6 => {
                let mut out = Vec::with_capacity(len);
                // Read at most one byte more than expected, so a bomb is refused, not expanded.
                flate2::read::DeflateDecoder::new(data).take(len as u64 + 1).read_to_end(&mut out).ok()?;
                (out.len() == len).then_some(out)
            }
            Codec::Lz4 => lz4_flex::block::decompress(data, len).ok().filter(|o| o.len() == len),
            #[cfg(not(target_arch = "wasm32"))]
            Codec::Zstd1 | Codec::Zstd3 => zstd::bulk::decompress(data, len).ok().filter(|o| o.len() == len),
        }
    }
}

pub fn fnv1a64(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// The checksums compared. Each returns a word so the call cannot be optimised away.
pub fn checksum(kind: &str, data: &[u8]) -> u64 {
    match kind {
        "crc32 (crc32fast)" => crc32fast::hash(data) as u64,
        "crc32c" => crc32c::crc32c(data) as u64,
        "xxh3-64" => xxhash_rust::xxh3::xxh3_64(data),
        "fnv-1a 64" => fnv1a64(data),
        "sha-256 (first 8 bytes)" => {
            use sha2::{Digest, Sha256};
            u64::from_le_bytes(Sha256::digest(data)[..8].try_into().unwrap())
        }
        other => panic!("unknown checksum {other}"),
    }
}

pub const CHECKSUMS: [&str; 5] =
    ["crc32 (crc32fast)", "crc32c", "xxh3-64", "fnv-1a 64", "sha-256 (first 8 bytes)"];
