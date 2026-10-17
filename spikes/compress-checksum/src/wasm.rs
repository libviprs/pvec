//! wasm32 exports for the node harness. The host allocates with `alloc`, copies a packed buffer in
//! and calls one export. Packed buffer: `u32 count`, then `count + 1` little-endian `u32` offsets, then
//! the block bytes; each block's raw length is stored in `lens` (a second buffer of `count` `u32`).
#![allow(unsafe_code)]

use crate::{Codec, checksum, unshuffle8};

#[unsafe(no_mangle)]
pub extern "C" fn alloc(n: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(n);
    let p = v.as_mut_ptr();
    core::mem::forget(v);
    p
}

fn bytes<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    // SAFETY: the host got `ptr` from `alloc(len)` and wrote `len` initialised bytes before calling;
    // nothing frees or mutates the buffer during the call.
    unsafe { core::slice::from_raw_parts(ptr, len) }
}

fn u32s(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// Decompresses every block (codec 0 deflate, 1 lz4), optionally unshuffles it, and returns the sum of
/// the decoded lengths plus an xor of the first byte of each block so the work is observable.
#[unsafe(no_mangle)]
pub extern "C" fn decode_blocks(
    codec: u32,
    shuffled: u32,
    data: *const u8,
    data_len: usize,
    lens: *const u8,
    count: usize,
) -> u64 {
    let (data, lens) = (bytes(data, data_len), u32s(bytes(lens, count * 4)));
    let codec = if codec == 0 { Codec::Deflate1 } else { Codec::Lz4 };
    let offs = u32s(&data[4..4 + 4 * (count + 1)]);
    let base = 4 + 4 * (count + 1);
    let mut total = 0u64;
    let mut x = 0u64;
    for i in 0..count {
        let blk = &data[base + offs[i] as usize..base + offs[i + 1] as usize];
        let Some(mut raw) = codec.decompress(blk, lens[i] as usize) else { return u64::MAX };
        if shuffled == 1 {
            raw = unshuffle8(&raw);
        }
        total += raw.len() as u64;
        x ^= raw[0] as u64;
    }
    total ^ (x << 56)
}

/// Checksums every block with checksum `kind` (index into `CHECKSUMS`) and folds the results.
#[unsafe(no_mangle)]
pub extern "C" fn checksum_blocks(
    kind: u32,
    data: *const u8,
    data_len: usize,
    lens: *const u8,
    count: usize,
) -> u64 {
    let (data, lens) = (bytes(data, data_len), u32s(bytes(lens, count * 4)));
    let mut at = 0usize;
    let mut acc = 0u64;
    for l in lens {
        acc = acc.wrapping_add(checksum(crate::CHECKSUMS[kind as usize], &data[at..at + l as usize]));
        at += l as usize;
    }
    acc
}
