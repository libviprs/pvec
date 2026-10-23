//! The fixed header.

use crate::error::ContainerError;
use crate::manifest::Manifest;
use crate::spec::{BYTE_ORDER_MARKER, MAGIC, h};

/// A checksum algorithm id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    /// XXH3-64 (ADR 0006, per block).
    Xxh3_64,
    /// SHA-256 (ADR 0006, per artifact).
    Sha256,
}

impl ChecksumAlgorithm {
    /// The id stored in the file.
    pub fn id(self) -> u8 {
        match self {
            ChecksumAlgorithm::Xxh3_64 => 1,
            ChecksumAlgorithm::Sha256 => 2,
        }
    }

    /// The algorithm for an id.
    pub fn from_id(id: u8) -> Option<ChecksumAlgorithm> {
        match id {
            1 => Some(ChecksumAlgorithm::Xxh3_64),
            2 => Some(ChecksumAlgorithm::Sha256),
            _ => None,
        }
    }
}

/// The parsed header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    /// Container major version.
    pub major: u16,
    /// Container minor version.
    pub minor: u16,
    /// Length of the header in the file.
    pub header_len: u16,
    /// Per-block checksum algorithm.
    pub block_checksum: ChecksumAlgorithm,
    /// Artifact-level checksum algorithm.
    pub artifact_checksum: ChecksumAlgorithm,
    /// Where the manifest starts.
    pub manifest_offset: u64,
    /// How long it is.
    pub manifest_length: u64,
    /// Where the directory-pointer table starts.
    pub directory_table_offset: u64,
    /// How long it is.
    pub directory_table_length: u64,
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Checks that `offset..offset + length` lies in `floor..artifact_len` without overflowing.
pub(crate) fn check_range(
    field: &'static str,
    offset: u64,
    length: u64,
    floor: u64,
    artifact_len: u64,
) -> Result<(), ContainerError> {
    let ok = offset >= floor && offset.checked_add(length).is_some_and(|end| end <= artifact_len);
    if ok { Ok(()) } else { Err(ContainerError::PointerOutOfRange { field, offset, length, artifact_len }) }
}

impl Header {
    /// The fixed header of this version, in bytes.
    pub const LEN: usize = 64;
    /// The largest header a reader will fetch.
    pub const MAX_LEN: usize = 4096;
    /// The one major version this reader implements.
    pub const MAJOR: u16 = 1;
    /// The newest minor version this reader knows the layout of.
    pub const MINOR: u16 = 0;

    /// Parses and validates a header from the first bytes of an artifact of `artifact_len` bytes.
    ///
    /// The magic is judged from the first eight bytes alone, so a file that is not a PVec file is
    /// refused without the parser looking at anything after them.
    pub fn parse(prefix: &[u8], artifact_len: u64) -> Result<Header, ContainerError> {
        let truncated =
            |needed: usize| ContainerError::Truncated { needed: needed as u64, got: prefix.len() as u64 };
        if prefix.len() < MAGIC.len() {
            return Err(truncated(MAGIC.len()));
        }
        if prefix[h::MAGIC..h::MAGIC + MAGIC.len()] != MAGIC {
            return Err(ContainerError::BadMagic);
        }
        if prefix.len() < Header::LEN {
            return Err(truncated(Header::LEN));
        }
        let major = u16_at(prefix, h::MAJOR);
        if major != Header::MAJOR {
            return Err(ContainerError::UnsupportedVersion { found: major, supported: Header::MAJOR });
        }
        let marker = u32::from_le_bytes([
            prefix[h::BYTE_ORDER],
            prefix[h::BYTE_ORDER + 1],
            prefix[h::BYTE_ORDER + 2],
            prefix[h::BYTE_ORDER + 3],
        ]);
        if marker != BYTE_ORDER_MARKER {
            return Err(ContainerError::UnsupportedByteOrder { found: marker });
        }
        let header_len = u16_at(prefix, h::HEADER_LEN);
        if (header_len as usize) < Header::LEN || (header_len as usize) > Header::MAX_LEN {
            return Err(ContainerError::BadHeaderLength { found: header_len as u32 });
        }
        if prefix.len() < header_len as usize {
            return Err(truncated(header_len as usize));
        }
        let block_id = prefix[h::BLOCK_CHECKSUM];
        let block_checksum = ChecksumAlgorithm::from_id(block_id)
            .ok_or(ContainerError::UnknownChecksumAlgorithm { id: block_id })?;
        let artifact_id = prefix[h::ARTIFACT_CHECKSUM];
        let artifact_checksum = ChecksumAlgorithm::from_id(artifact_id)
            .ok_or(ContainerError::UnknownChecksumAlgorithm { id: artifact_id })?;
        for (at, len) in [(h::RESERVED_A, 2), (h::RESERVED_B, 2), (h::RESERVED_C, 8)] {
            if prefix[at..at + len].iter().any(|b| *b != 0) {
                return Err(ContainerError::NonZeroReserved { field: "header reserved" });
            }
        }
        let manifest_offset = u64_at(prefix, h::MANIFEST_OFFSET);
        let manifest_length = u64_at(prefix, h::MANIFEST_LENGTH);
        let directory_table_offset = u64_at(prefix, h::DIRECTORY_TABLE_OFFSET);
        let directory_table_length = u64_at(prefix, h::DIRECTORY_TABLE_LENGTH);
        let floor = header_len as u64;
        check_range("manifest", manifest_offset, manifest_length, floor, artifact_len)?;
        if manifest_length == 0 {
            return Err(ContainerError::PointerOutOfRange {
                field: "manifest",
                offset: manifest_offset,
                length: 0,
                artifact_len,
            });
        }
        if manifest_length > Manifest::MAX_LEN {
            return Err(ContainerError::ManifestTooLarge { length: manifest_length });
        }
        check_range("directory_table", directory_table_offset, directory_table_length, floor, artifact_len)?;
        Ok(Header {
            major,
            minor: u16_at(prefix, h::MINOR),
            header_len,
            block_checksum,
            artifact_checksum,
            manifest_offset,
            manifest_length,
            directory_table_offset,
            directory_table_length,
        })
    }

    /// The fixed 64-byte encoding.
    pub fn encode(&self) -> [u8; Header::LEN] {
        let mut b = [0u8; Header::LEN];
        b[h::MAGIC..h::MAGIC + MAGIC.len()].copy_from_slice(&MAGIC);
        b[h::MAJOR..h::MAJOR + 2].copy_from_slice(&self.major.to_le_bytes());
        b[h::MINOR..h::MINOR + 2].copy_from_slice(&self.minor.to_le_bytes());
        b[h::BYTE_ORDER..h::BYTE_ORDER + 4].copy_from_slice(&BYTE_ORDER_MARKER.to_le_bytes());
        b[h::HEADER_LEN..h::HEADER_LEN + 2].copy_from_slice(&self.header_len.to_le_bytes());
        b[h::BLOCK_CHECKSUM] = self.block_checksum.id();
        b[h::ARTIFACT_CHECKSUM] = self.artifact_checksum.id();
        b[h::MANIFEST_OFFSET..h::MANIFEST_OFFSET + 8].copy_from_slice(&self.manifest_offset.to_le_bytes());
        b[h::MANIFEST_LENGTH..h::MANIFEST_LENGTH + 8].copy_from_slice(&self.manifest_length.to_le_bytes());
        b[h::DIRECTORY_TABLE_OFFSET..h::DIRECTORY_TABLE_OFFSET + 8]
            .copy_from_slice(&self.directory_table_offset.to_le_bytes());
        b[h::DIRECTORY_TABLE_LENGTH..h::DIRECTORY_TABLE_LENGTH + 8]
            .copy_from_slice(&self.directory_table_length.to_le_bytes());
        b
    }
}
