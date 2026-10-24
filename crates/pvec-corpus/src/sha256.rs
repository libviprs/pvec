//! SHA-256 (FIPS 180-4), so the pin can be checked with no dependency and no process.

/// The digest as 64 lowercase hex digits.
pub fn hex_digest(_data: &[u8]) -> String {
    todo!("sha256::hex_digest")
}
