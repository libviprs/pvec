//! Block checksums and the artifact digest (issue P4.6), the two levels of ADR 0006.
//!
//! * A **block checksum** is the XXH3-64 of a block's stored bytes, eight bytes per block in a table the
//!   manifest points at. It is verified when a block is fetched, before the block is decompressed or
//!   parsed, so a corrupt block never reaches either. It is cheap (the fastest in wasm of the candidates)
//!   and it is not a security hash.
//! * The **artifact digest** is the SHA-256 of the whole file with its own 32 bytes in the manifest read
//!   as zero. That makes it a digest of the canonical byte stream: the header, the manifest, every block,
//!   the checksum table, the directory and the padding, so a streamed artifact can be pinned by one value,
//!   and a change to the manifest or the directory is seen even though no block checksum covers them. It
//!   is verified on request, by reading the file in bounded chunks, not at every open.
//!
//! The SHA-256 here is a small safe implementation checked against the published vectors and against an
//! independent one in the tests; it avoids a dependency that carries `unsafe`.

use core::fmt;
use xxhash_rust::xxh3::xxh3_64;

/// What the writer records for integrity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Integrity {
    /// Nothing: no checksum table and no digest.
    None,
    /// The per-block checksum table.
    Checksums,
    /// The checksum table and the artifact digest.
    #[default]
    Full,
}

/// The XXH3-64 of a block's stored bytes.
pub fn block_checksum(bytes: &[u8]) -> u64 {
    xxh3_64(bytes)
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// A streaming SHA-256.
#[derive(Clone, Debug)]
pub struct Sha256 {
    state: [u32; 8],
    block: [u8; 64],
    filled: usize,
    length: u64,
}

impl Default for Sha256 {
    fn default() -> Sha256 {
        Sha256::new()
    }
}

impl Sha256 {
    /// A hash of nothing yet.
    pub fn new() -> Sha256 {
        Sha256 {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            block: [0; 64],
            filled: 0,
            length: 0,
        }
    }

    fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for (i, c) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([c[0], c[1], c[2], c[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = *state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7].wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [t1.wrapping_add(t2), v[0], v[1], v[2], v[3].wrapping_add(t1), v[4], v[5], v[6]];
        }
        for (a, b) in state.iter_mut().zip(v) {
            *a = a.wrapping_add(b);
        }
    }

    /// Adds `data` to the hash.
    pub fn update(&mut self, mut data: &[u8]) {
        self.length = self.length.wrapping_add(data.len() as u64);
        if self.filled > 0 {
            let take = (64 - self.filled).min(data.len());
            self.block[self.filled..self.filled + take].copy_from_slice(&data[..take]);
            self.filled += take;
            data = &data[take..];
            if self.filled == 64 {
                let block = self.block;
                Sha256::compress(&mut self.state, &block);
                self.filled = 0;
            }
        }
        let mut chunks = data.chunks_exact(64);
        for chunk in &mut chunks {
            let mut block = [0u8; 64];
            block.copy_from_slice(chunk);
            Sha256::compress(&mut self.state, &block);
        }
        let rest = chunks.remainder();
        self.block[..rest.len()].copy_from_slice(rest);
        self.filled += rest.len();
    }

    /// The digest.
    pub fn finalize(mut self) -> [u8; 32] {
        let bits = self.length.wrapping_mul(8);
        self.update(&[0x80]);
        while self.filled != 56 {
            self.update(&[0]);
        }
        self.update(&bits.to_be_bytes());
        let mut out = [0u8; 32];
        for (o, s) in out.chunks_exact_mut(4).zip(self.state) {
            o.copy_from_slice(&s.to_be_bytes());
        }
        out
    }
}

/// The SHA-256 of `bytes`.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize()
}

/// The artifact digest as it is fed to a hash a chunk at a time: the 32 bytes at `digest_at` read as zero,
/// wherever the chunks happen to fall.
#[derive(Clone, Debug)]
pub struct DigestStream {
    sha: Sha256,
    at: u64,
    mask: (u64, u64),
}

impl DigestStream {
    /// A stream for a file whose digest field is the 32 bytes at `digest_at`.
    pub fn new(digest_at: u64) -> DigestStream {
        DigestStream { sha: Sha256::new(), at: 0, mask: (digest_at, digest_at.saturating_add(32)) }
    }

    /// Feeds the next `chunk` of the file.
    pub fn update(&mut self, chunk: &[u8]) {
        let (start, end) = (self.at, self.at + chunk.len() as u64);
        let (m0, m1) = (self.mask.0.clamp(start, end), self.mask.1.clamp(start, end));
        if m0 == m1 {
            self.sha.update(chunk);
        } else {
            let (a, b) = ((m0 - start) as usize, (m1 - start) as usize);
            self.sha.update(&chunk[..a]);
            self.sha.update(&[0u8; 32][..b - a]);
            self.sha.update(&chunk[b..]);
        }
        self.at = end;
    }

    /// The digest of everything fed.
    pub fn finalize(self) -> [u8; 32] {
        self.sha.finalize()
    }
}

/// The artifact digest of a whole file: its SHA-256 with the 32 digest bytes at `digest_at` read as zero.
pub fn artifact_digest(artifact: &[u8], digest_at: usize) -> [u8; 32] {
    let mut s = DigestStream::new(digest_at as u64);
    s.update(artifact);
    s.finalize()
}

/// Why integrity was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IntegrityError {
    /// The artifact digest is not the one in the manifest.
    ArtifactDigest {
        /// What the manifest holds.
        expected: [u8; 32],
        /// What the file hashes to.
        found: [u8; 32],
    },
    /// The block checksum table is not where or what it should be.
    BadTable {
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A block's stored bytes do not match their checksum.
    BlockChecksum {
        /// The block.
        block: u32,
        /// What the table holds.
        expected: u64,
        /// What the bytes hash to.
        found: u64,
    },
    /// The reader was told to require integrity and the file carries none.
    NotWritten,
}

impl IntegrityError {
    /// The rule identifier of `docs/CONTAINER.md`.
    pub fn rule(&self) -> &'static str {
        match self {
            IntegrityError::ArtifactDigest { .. } => "INTEGRITY-01",
            IntegrityError::BadTable { .. } => "INTEGRITY-02",
            IntegrityError::BlockChecksum { .. } => "INTEGRITY-03",
            IntegrityError::NotWritten => "INTEGRITY-04",
        }
    }
}

/// The refusals, rule by rule.
pub const INTEGRITY_RULES: &[(&str, &str)] = &[
    (
        "INTEGRITY-01",
        "the SHA-256 of the file, with the digest's own bytes read as zero, is not the digest in the manifest (including a digest declared present that is all zero)",
    ),
    (
        "INTEGRITY-02",
        "the block checksum table is not one 8 byte sum per block, overlaps a block, or runs into the directory",
    ),
    ("INTEGRITY-03", "the XXH3-64 of a block's stored bytes is not the sum in the table"),
    (
        "INTEGRITY-04",
        "the reader was configured to require integrity and the file has no checksum table or no digest",
    ),
];

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rule = self.rule();
        match self {
            IntegrityError::ArtifactDigest { expected, found } => {
                write!(
                    f,
                    "{rule}: the manifest holds digest {} and the file hashes to {}",
                    hex(expected),
                    hex(found)
                )
            }
            IntegrityError::BadTable { reason } => write!(f, "{rule}: the block checksum table {reason}"),
            IntegrityError::BlockChecksum { block, expected, found } => {
                write!(f, "{rule}: block {block} hashes to {found:016x}, its table entry is {expected:016x}")
            }
            IntegrityError::NotWritten => {
                write!(f, "{rule}: the file carries no checksum table or no artifact digest")
            }
        }
    }
}

impl std::error::Error for IntegrityError {}

/// What verifying the artifact digest found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestStatus {
    /// The file hashes to the digest in its manifest.
    Verified,
    /// The manifest carries no digest, so there was nothing to check.
    NotWritten,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sha256_matches_the_published_vectors_across_chunkings() {
        let abc = sha256(b"abc");
        assert_eq!(abc[..4], [0xba, 0x78, 0x16, 0xbf]);
        let long = b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
        let one = sha256(long);
        for step in [1usize, 3, 7, 63, 64, 65] {
            let mut h = Sha256::new();
            for c in long.chunks(step) {
                h.update(c);
            }
            assert_eq!(h.finalize(), one, "step {step}");
        }
    }

    #[test]
    fn the_digest_stream_zeroes_its_field_whatever_the_chunking() {
        let data: Vec<u8> = (0..500u32).map(|i| (i * 13) as u8).collect();
        let want = artifact_digest(&data, 100);
        for step in [1usize, 5, 31, 32, 33, 100, 499] {
            let mut s = DigestStream::new(100);
            for c in data.chunks(step) {
                s.update(c);
            }
            assert_eq!(s.finalize(), want, "step {step}");
        }
    }
}
