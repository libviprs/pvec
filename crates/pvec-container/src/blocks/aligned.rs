//! A byte buffer whose first byte is 8-aligned, and the casts that read it in place.
//!
//! This is the only module of the crate that contains `unsafe`, in exactly three places: the
//! `from_raw_parts` that views the zero-initialised `Vec<u64>` storage as bytes, and the `align_to`
//! casts to `f64` and `u32`. Each carries a `SAFETY:` comment, and `pvec-checks` fails the build if an
//! `unsafe` token appears anywhere else, without a `SAFETY:` comment, or if this file's count grows
//! past three.
//!
//! There is no safe spelling that yields a `&[f64]` (or `&[u32]`) borrowed from bytes: `std` offers
//! `from_le_bytes` on copies and nothing that reinterprets a borrowed slice. A safe byte-offset column
//! view is possible, though: a type that holds the `&[u8]` of a column and decodes element `i` with
//! `f64::from_le_bytes` on demand needs no `unsafe` and no alignment, at the price that callers get an
//! accessor instead of a slice. Whether the public view stays `&[f64]` is for issue #54 to decide; until
//! it does, these casts are what keeps the slices zero-copy.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer() -> AlignedBytes {
        AlignedBytes::from_bytes(&[0u8; 32])
    }

    #[test]
    fn the_aligned_buffer_starts_on_an_8_byte_boundary_and_casts() {
        let b = buffer();
        assert_eq!(b.as_bytes().as_ptr() as usize % 8, 0);
        assert_eq!(f64s(b.as_bytes()).map(<[f64]>::len), Some(4));
        assert_eq!(u32s(b.as_bytes()).map(<[u32]>::len), Some(8));
    }

    #[test]
    fn a_buffer_offset_by_one_byte_yields_none_from_f64s_and_u32s() {
        let b = buffer();
        let bytes = b.as_bytes();
        // 8 and 4 bytes long, so only the start address is wrong
        assert!(f64s(&bytes[1..9]).is_none());
        assert!(u32s(&bytes[1..5]).is_none());
        // and the same with a longer window
        assert!(f64s(&bytes[1..25]).is_none());
        assert!(u32s(&bytes[1..29]).is_none());
    }

    #[test]
    fn an_offset_that_suits_u32_but_not_f64_is_refused_by_f64s_only() {
        let b = buffer();
        let bytes = b.as_bytes();
        assert!(f64s(&bytes[4..12]).is_none());
        assert_eq!(u32s(&bytes[4..12]).map(<[u32]>::len), Some(2));
    }

    #[test]
    fn a_length_that_is_not_a_multiple_of_the_element_is_refused() {
        let b = buffer();
        let bytes = b.as_bytes();
        assert!(f64s(&bytes[0..12]).is_none());
        assert!(u32s(&bytes[0..6]).is_none());
    }

    #[test]
    fn an_empty_buffer_is_fine() {
        let b = AlignedBytes::default();
        assert!(b.is_empty());
        assert_eq!(b.as_bytes().len(), 0);
        assert_eq!(f64s(b.as_bytes()).map(<[f64]>::len), Some(0));
    }
}
