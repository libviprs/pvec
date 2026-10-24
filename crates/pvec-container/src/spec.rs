//! The field tables behind `docs/CONTAINER.md` chapter 1.
//!
//! The parser and the encoder read every offset from the constants in this file, and
//! [`render_markdown`] writes the document's tables from the same constants, so the code and the
//! document cannot describe two different layouts without a test failing.

use crate::manifest::{LengthUnit, Manifest};

/// One field of a fixed structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// Field name.
    pub name: &'static str,
    /// Byte offset inside the structure.
    pub offset: usize,
    /// Size in bytes.
    pub size: usize,
    /// Encoding, for the document.
    pub encoding: &'static str,
    /// One-line meaning.
    pub meaning: &'static str,
}

/// The eight magic bytes: a high-bit byte so text tools do not mistake the file for text, `PVEC`,
/// then CR LF and SUB so a line-ending translation or a DOS `type` corrupts them visibly.
pub const MAGIC: [u8; 8] = [0x89, b'P', b'V', b'E', b'C', 0x0d, 0x0a, 0x1a];
/// Written as a little-endian `u32`; a big-endian writer would put `01 02 03 04` on disk.
pub const BYTE_ORDER_MARKER: u32 = 0x0102_0304;

/// Header offsets.
pub mod h {
    /// Magic.
    pub const MAGIC: usize = 0;
    /// Container major version.
    pub const MAJOR: usize = 8;
    /// Container minor version.
    pub const MINOR: usize = 10;
    /// Byte-order marker.
    pub const BYTE_ORDER: usize = 12;
    /// Header length.
    pub const HEADER_LEN: usize = 16;
    /// Reserved.
    pub const RESERVED_A: usize = 18;
    /// Block checksum algorithm id.
    pub const BLOCK_CHECKSUM: usize = 20;
    /// Artifact checksum algorithm id.
    pub const ARTIFACT_CHECKSUM: usize = 21;
    /// Reserved.
    pub const RESERVED_B: usize = 22;
    /// Manifest offset.
    pub const MANIFEST_OFFSET: usize = 24;
    /// Manifest length.
    pub const MANIFEST_LENGTH: usize = 32;
    /// Directory-pointer table offset.
    pub const DIRECTORY_TABLE_OFFSET: usize = 40;
    /// Directory-pointer table length.
    pub const DIRECTORY_TABLE_LENGTH: usize = 48;
    /// Reserved.
    pub const RESERVED_C: usize = 56;
}

/// Manifest offsets (inside the fixed part).
pub mod m {
    /// Length unit code.
    pub const UNIT: usize = 0;
    /// Flags.
    pub const FLAGS: usize = 2;
    /// Capability count.
    pub const CAPABILITY_COUNT: usize = 4;
    /// Metres per unit.
    pub const METRES_PER_UNIT: usize = 8;
    /// `min_x`.
    pub const MIN_X: usize = 16;
    /// `min_y`.
    pub const MIN_Y: usize = 24;
    /// `max_x`.
    pub const MAX_X: usize = 32;
    /// `max_y`.
    pub const MAX_Y: usize = 40;
    /// Minimum reader major.
    pub const MIN_READER_MAJOR: usize = 48;
    /// Minimum reader minor.
    pub const MIN_READER_MINOR: usize = 50;
    /// Minimum reader patch.
    pub const MIN_READER_PATCH: usize = 52;
    /// Reserved.
    pub const RESERVED_A: usize = 54;
    /// Directory count.
    pub const DIRECTORY_COUNT: usize = 56;
    /// Reserved.
    pub const RESERVED_B: usize = 60;
    /// Block checksum table offset.
    pub const BLOCK_CHECKSUMS_OFFSET: usize = 64;
    /// Block checksum table length.
    pub const BLOCK_CHECKSUMS_LENGTH: usize = 72;
    /// Artifact digest.
    pub const ARTIFACT_DIGEST: usize = 80;
    /// Producer id length.
    pub const PRODUCER_ID_LEN: usize = 112;
    /// Producer version length.
    pub const PRODUCER_VERSION_LEN: usize = 116;
    /// Corpus revision length.
    pub const CORPUS_REVISION_LEN: usize = 120;
    /// Reserved.
    pub const RESERVED_C: usize = 124;
    /// Flag bit 0: the artifact digest is present.
    pub const FLAG_DIGEST: u16 = 1;
}

/// Capability entry offsets.
pub mod cap {
    /// Entry size.
    pub const SIZE: usize = 16;
    /// Capability id.
    pub const ID: usize = 0;
    /// Version major.
    pub const MAJOR: usize = 4;
    /// Version minor.
    pub const MINOR: usize = 6;
    /// Version patch.
    pub const PATCH: usize = 8;
    /// Requirement byte.
    pub const REQUIREMENT: usize = 10;
    /// Reserved byte.
    pub const RESERVED_A: usize = 11;
    /// Reserved word.
    pub const RESERVED_B: usize = 12;
    /// Requirement byte: required.
    pub const REQUIRED: u8 = 1;
    /// Requirement byte: optional.
    pub const OPTIONAL: u8 = 2;
}

/// Directory entry offsets.
pub mod dir {
    /// Entry size.
    pub const SIZE: usize = 24;
    /// Kind.
    pub const KIND: usize = 0;
    /// Reserved.
    pub const RESERVED: usize = 4;
    /// Offset.
    pub const OFFSET: usize = 8;
    /// Length.
    pub const LENGTH: usize = 16;
}

/// The longest producer id, producer version or corpus revision, in bytes.
pub const MAX_STRING_LEN: u32 = 1024;

const fn f(
    name: &'static str,
    offset: usize,
    size: usize,
    encoding: &'static str,
    meaning: &'static str,
) -> FieldSpec {
    FieldSpec { name, offset, size, encoding, meaning }
}

/// The header fields.
pub const HEADER_FIELDS: &[FieldSpec] = &[
    f(
        "magic",
        h::MAGIC,
        8,
        "bytes",
        "`89 50 56 45 43 0D 0A 1A`; checked first, from these eight bytes alone",
    ),
    f(
        "container_major",
        h::MAJOR,
        2,
        "u16",
        "a reader refuses any major it does not implement; this reader implements 1",
    ),
    f(
        "container_minor",
        h::MINOR,
        2,
        "u16",
        "a higher minor parses; it only ever adds fields in reserved space or after the fixed part",
    ),
    f(
        "byte_order",
        h::BYTE_ORDER,
        4,
        "u32",
        "`0x01020304` written little-endian, so the bytes on disk are `04 03 02 01`",
    ),
    f(
        "header_len",
        h::HEADER_LEN,
        2,
        "u16",
        "64 to 4096; a reader skips whatever it does not know up to this length",
    ),
    f("reserved_a", h::RESERVED_A, 2, "u16", "zero"),
    f(
        "block_checksum",
        h::BLOCK_CHECKSUM,
        1,
        "u8",
        "per-block checksum algorithm id: 1 is XXH3-64 (ADR 0006), 2 is SHA-256",
    ),
    f("artifact_checksum", h::ARTIFACT_CHECKSUM, 1, "u8", "artifact-level checksum algorithm id, same ids"),
    f("reserved_b", h::RESERVED_B, 2, "u16", "zero"),
    f(
        "manifest_offset",
        h::MANIFEST_OFFSET,
        8,
        "u64",
        "absolute offset of the manifest, at least `header_len`",
    ),
    f("manifest_length", h::MANIFEST_LENGTH, 8, "u64", "manifest length, 1 to 1048576 bytes"),
    f(
        "directory_table_offset",
        h::DIRECTORY_TABLE_OFFSET,
        8,
        "u64",
        "absolute offset of the directory-pointer table, inside the manifest",
    ),
    f(
        "directory_table_length",
        h::DIRECTORY_TABLE_LENGTH,
        8,
        "u64",
        "table length, a multiple of 24 (zero when there are no directories)",
    ),
    f("reserved_c", h::RESERVED_C, 8, "u64", "zero"),
];

/// The fixed part of the manifest.
pub const MANIFEST_FIELDS: &[FieldSpec] = &[
    f("length_unit", m::UNIT, 2, "u16", "unit code from the table in section 1.5"),
    f("flags", m::FLAGS, 2, "u16", "bit 0: the artifact digest is present; every other bit is zero"),
    f("capability_count", m::CAPABILITY_COUNT, 4, "u32", "number of capability entries"),
    f(
        "metres_per_unit",
        m::METRES_PER_UNIT,
        8,
        "f64",
        "exactly the unit's factor from the table in section 1.5",
    ),
    f("min_x", m::MIN_X, 8, "f64", "source bounds in source space, in the declared unit"),
    f("min_y", m::MIN_Y, 8, "f64", "see `min_x`"),
    f("max_x", m::MAX_X, 8, "f64", "see `min_x`; at least `min_x`"),
    f("max_y", m::MAX_Y, 8, "f64", "see `min_x`; at least `min_y`"),
    f(
        "min_reader_major",
        m::MIN_READER_MAJOR,
        2,
        "u16",
        "the lowest capability-registry reader version that may open the file",
    ),
    f("min_reader_minor", m::MIN_READER_MINOR, 2, "u16", "see `min_reader_major`"),
    f("min_reader_patch", m::MIN_READER_PATCH, 2, "u16", "see `min_reader_major`"),
    f("reserved_a", m::RESERVED_A, 2, "u16", "zero"),
    f("directory_count", m::DIRECTORY_COUNT, 4, "u32", "number of directory pointer entries"),
    f("reserved_b", m::RESERVED_B, 4, "u32", "zero"),
    f(
        "block_checksums_offset",
        m::BLOCK_CHECKSUMS_OFFSET,
        8,
        "u64",
        "per-block checksum table (P4.6); zero with a zero length means not written",
    ),
    f("block_checksums_length", m::BLOCK_CHECKSUMS_LENGTH, 8, "u64", "see `block_checksums_offset`"),
    f(
        "artifact_digest",
        m::ARTIFACT_DIGEST,
        32,
        "bytes",
        "artifact-level digest (P4.6); all zero unless flag bit 0 is set",
    ),
    f("producer_id_len", m::PRODUCER_ID_LEN, 4, "u32", "bytes of UTF-8, 1 to 1024"),
    f("producer_version_len", m::PRODUCER_VERSION_LEN, 4, "u32", "bytes of UTF-8, 0 to 1024"),
    f(
        "corpus_revision_len",
        m::CORPUS_REVISION_LEN,
        4,
        "u32",
        "bytes of UTF-8, 0 to 1024; zero means there is no fixture or corpus revision",
    ),
    f("reserved_c", m::RESERVED_C, 4, "u32", "zero"),
];

/// One capability table entry.
pub const CAPABILITY_ENTRY_FIELDS: &[FieldSpec] = &[
    f("id", cap::ID, 4, "u32", "stable capability id from the registry (P3.4); never zero, never repeated"),
    f("version_major", cap::MAJOR, 2, "u16", "the capability version the file was written with"),
    f("version_minor", cap::MINOR, 2, "u16", "see `version_major`"),
    f("version_patch", cap::PATCH, 2, "u16", "see `version_major`"),
    f("requirement", cap::REQUIREMENT, 1, "u8", "1 required, 2 optional"),
    f("reserved_a", cap::RESERVED_A, 1, "u8", "zero"),
    f("reserved_b", cap::RESERVED_B, 4, "u32", "zero"),
];

/// One directory-pointer table entry.
pub const DIRECTORY_ENTRY_FIELDS: &[FieldSpec] = &[
    f(
        "kind",
        dir::KIND,
        4,
        "u32",
        "1 block directory (P4.2), 2 component directory (P4.3), 3 spatial directory (P5.1); later kinds are skipped by a reader that does not know them",
    ),
    f("reserved", dir::RESERVED, 4, "u32", "zero"),
    f("offset", dir::OFFSET, 8, "u64", "absolute offset, at least `header_len`"),
    f("length", dir::LENGTH, 8, "u64", "length; offset plus length is at most the artifact length"),
];

fn table(fields: &[FieldSpec]) -> String {
    let mut s = String::from("| offset | size | name | encoding | meaning |\n|---|---|---|---|---|\n");
    for x in fields {
        s.push_str(&format!("| {} | {} | {} | {} | {} |\n", x.offset, x.size, x.name, x.encoding, x.meaning));
    }
    s
}

/// Chapter 1 of `docs/CONTAINER.md`, rendered from the constants.
pub fn render_markdown() -> String {
    let mut s = String::new();
    s.push_str(
        "# The PVec container\n\n\
This file is generated from `crates/pvec-container/src/spec.rs`. I do not edit it by hand: a test \
regenerates it and fails if the committed file differs by a single byte. To change it I change the \
constants and run `PVEC_BLESS_CONTAINER=1 cargo test -p pvec-container --test container_doc`.\n\n\
## 1. The container prefix\n\n\
Nothing can be read out of a `.pvec` file until its first bytes say what the file is, which \
capabilities it declares and where the directories live. The prefix is a fixed header and a manifest. \
A reader fetches the first 4 KiB with one range read, validates the header from it, and fetches the \
manifest (at most 1 MiB) with a second one when it did not already arrive in the first.\n\n\
The encoding is the one ADR 0003 chose (a custom block container, little-endian, no external \
dependency). ADR 0003 and the layout spike quote a 32 byte header; that was the size of the \
prototype's header, which had no room for the manifest pointers, the checksum ids or a header \
length. The v1.0 header is 64 bytes, which adds 32 bytes to a cold open.\n\n",
    );
    s.push_str(&format!(
        "### 1.1 Byte order\n\nEvery multi-byte integer and every `f64` in the prefix is little-endian. \
The byte-order marker is `0x{BYTE_ORDER_MARKER:08X}` written little-endian, and a reader that sees it \
any other way refuses the file rather than byte-swapping it. Floats are IEEE 754 binary64.\n\n"
    ));
    s.push_str(&format!(
        "### 1.2 The header\n\nThe header is {} bytes in version 1.0 and never more than 4096. Offsets are from the \
first byte of the file. The reasoning for 64 rather than 32 bytes is in [ADR 0003, Amendment 2026-10-23](adr/0003-physical-container.md#amendment-2026-10-23).\n\n",
        HEADER_FIELDS.iter().map(|x| x.size).sum::<usize>()
    ));
    s.push_str(&table(HEADER_FIELDS));
    s.push_str(&format!(
        "\nThe manifest and the directory-pointer table are located by absolute offsets and lengths. A reader \
checks every pair against the artifact length before using it: the offset must be at least `header_len`, \
the sum must not overflow, and the end must not pass the end of the file. A pointer that fails is refused, \
never clamped. The directory-pointer table lies inside the manifest, at `manifest_offset + {} + 16 * \
capability_count`, and the header and the manifest must agree on where.\n\n",
        Manifest::FIXED_LEN
    ));
    s.push_str(&format!(
        "### 1.3 The manifest\n\nThe manifest is the fixed part below, then in this order the capability table \
(`capability_count` entries of {} bytes), the directory-pointer table (`directory_count` entries of {} \
bytes), the producer id, the producer version and the corpus revision as UTF-8 with no terminator, and zero \
bytes up to the next multiple of 8. Nothing is allocated from a count before the bytes it implies are known \
to be inside the manifest.\n\n",
        cap::SIZE,
        dir::SIZE
    ));
    s.push_str("Fixed part, offsets from the first byte of the manifest:\n\n");
    s.push_str(&table(MANIFEST_FIELDS));
    s.push_str("\nBounds are `f64` in source space and obey the numeric policy (`NP-01`, `NP-02`, `NP-03`): finite, normal or zero, at most 2^40 in magnitude, negative zero read as positive zero, and `min <= max` on both axes. A NaN or infinite bound fails parsing.\n\n");
    s.push_str("Capability entry:\n\n");
    s.push_str(&table(CAPABILITY_ENTRY_FIELDS));
    s.push_str("\nAn id the reader does not implement still parses; what to do about it is version negotiation (P4.7).\n\n");
    s.push_str("Directory-pointer entry:\n\n");
    s.push_str(&table(DIRECTORY_ENTRY_FIELDS));
    s.push_str("\n### 1.4 Metadata and digests\n\nThe producer id and version are supplied by the caller and the producer id is never empty. \
The corpus revision is present only for a file built from a fixture or corpus. The block checksum table pointer and the \
artifact digest are written by P4.6; until then they are absent (zeros, flag bit 0 clear), which is a valid manifest.\n\n");
    s.push_str("### 1.5 Units of length\n\nThe unit is declared as an ISO 10303-41 length unit and the factor to metres is stored next to it, \
which must equal the table's value exactly.\n\n| code | unit | metres per unit | ISO 10303-41 declaration |\n|---|---|---|---|\n");
    for u in LengthUnit::ALL {
        s.push_str(&format!(
            "| {} | {:?} | {} | `{}` |\n",
            u.code(),
            u,
            u.metres_per_unit(),
            u.iso_declaration()
        ));
    }
    s.push_str(
        "\n### 1.6 Reserved space\n\nEvery field named `reserved` is zero in version 1.0 and a reader refuses a file where one is not, \
so a later minor version can give it a meaning without an old reader silently misreading it. Padding between sections is zero for the \
same reason, which also makes the encoding canonical: one value, one byte string.\n\n\
### 1.7 Growth rules\n\n\
* A change that an old reader could misread bumps the major version, and an old reader refuses it with `UnsupportedVersion`.\n\
* A change that is safe to ignore bumps the minor version: new fields go in reserved space, or after the fixed part of the header (the \
header's own length says where it ends) or after the last manifest section. A reader of a lower minor skips them.\n\
* Trailing bytes after the manifest's last section are refused in a file whose minor version is the reader's, because there they can only be \
corruption.\n\
* New capabilities, directory kinds and codecs are added by id (ADR 0008), never by changing the layout above.\n\
* The header never grows past 4096 bytes and the manifest never past 1 MiB, so a reader's first fetch stays bounded.\n\n\
### 1.8 Refusals\n\n\
`ContainerError` has one variant for each: bad magic, unsupported major version, unsupported byte order, a truncated prefix, a bad header \
length, a pointer outside the artifact, an unknown checksum algorithm, a non-zero reserved field, a unit or factor that is not in the table, \
a bound that fails the numeric policy, a duplicate or malformed capability, a mismatched directory table and trailing bytes.\n",
    );
    s
}
