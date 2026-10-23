//! A byte buffer whose first byte is 8-aligned, and the two casts that read it in place.
//!
//! This is the only module of the crate that contains `unsafe`. Borrowing a column as `&[f64]` or
//! `&[u32]` out of the bytes of a block is a reinterpretation that safe Rust has no spelling for, and
//! the alternative (copying every column out) is what the hot block exists to avoid. Each block below
//! carries the argument for why it is sound, and `pvec-checks` fails the build if an `unsafe` appears
//! anywhere else or without a `SAFETY:` comment.
#![allow(unsafe_code)]

/// Owned bytes that start on an 8-byte boundary, so a block read from a file can be decoded in place.
///
/// The storage is a run of `u64` words; `as_bytes` is the same memory as bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlignedBytes {
    words: Vec<u64>,
    len: usize,
}

impl AlignedBytes {
    /// A copy of `bytes` in aligned storage.
    pub fn from_bytes(bytes: &[u8]) -> AlignedBytes {
        let mut words = vec![0u64; bytes.len().div_ceil(8)];
        for (w, chunk) in words.iter_mut().zip(bytes.chunks(8)) {
            let mut b = [0u8; 8];
            b[..chunk.len()].copy_from_slice(chunk);
            *w = u64::from_ne_bytes(b);
        }
        AlignedBytes { words, len: bytes.len() }
    }

    /// The bytes, starting on an 8-byte boundary.
    pub fn as_bytes(&self) -> &[u8] {
        // SAFETY: `words` owns `words.len() * 8 >= len` initialised bytes, `u8` has alignment 1 and no
        // invalid bit pattern, and the returned slice borrows `self`, so the storage outlives it and is
        // not mutated while it exists.
        unsafe { core::slice::from_raw_parts(self.words.as_ptr().cast::<u8>(), self.len) }
    }

    /// Number of bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether there are no bytes.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// `bytes` as little-endian `f64` values, or `None` when it is not 8-aligned, not a multiple of 8 long,
/// or the host is not little-endian.
pub(crate) fn f64s(bytes: &[u8]) -> Option<&[f64]> {
    if cfg!(target_endian = "big") {
        return None;
    }
    // SAFETY: every 8-byte pattern is a valid `f64` (NaNs included), so reinterpreting initialised bytes
    // cannot create an invalid value; `align_to` only returns a middle slice that is correctly aligned
    // and inside `bytes`, and its lifetime is that of `bytes`.
    let (prefix, middle, suffix) = unsafe { bytes.align_to::<f64>() };
    (prefix.is_empty() && suffix.is_empty()).then_some(middle)
}

/// `bytes` as little-endian `u32` values, under the same conditions as [`f64s`] with 4 in place of 8.
pub(crate) fn u32s(bytes: &[u8]) -> Option<&[u32]> {
    if cfg!(target_endian = "big") {
        return None;
    }
    // SAFETY: every 4-byte pattern is a valid `u32`; see `f64s` for the rest of the argument.
    let (prefix, middle, suffix) = unsafe { bytes.align_to::<u32>() };
    (prefix.is_empty() && suffix.is_empty()).then_some(middle)
}
