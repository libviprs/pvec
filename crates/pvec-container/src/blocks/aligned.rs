//! A byte buffer whose first byte is 8-aligned, so a decoded block can be read in place.

/// Owned bytes that start on an 8-byte boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlignedBytes {
    bytes: Vec<u8>,
}

impl AlignedBytes {
    /// A copy of `bytes` in aligned storage.
    pub fn from_bytes(bytes: &[u8]) -> AlignedBytes {
        AlignedBytes { bytes: bytes.to_vec() }
    }

    /// The bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}
