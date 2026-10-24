# The PVec container

This file is generated from `crates/pvec-container/src/spec.rs`. I do not edit it by hand: a test regenerates it and fails if the committed file differs by a single byte. To change it I change the constants and run `PVEC_BLESS_CONTAINER=1 cargo test -p pvec-container --test container_doc`.

## 1. The container prefix

Nothing can be read out of a `.pvec` file until its first bytes say what the file is, which capabilities it declares and where the directories live. The prefix is a fixed header and a manifest. A reader fetches the first 4 KiB with one range read, validates the header from it, and fetches the manifest (at most 1 MiB) with a second one when it did not already arrive in the first.

The encoding is the one ADR 0003 chose (a custom block container, little-endian, no external dependency). ADR 0003 and the layout spike quote a 32 byte header; that was the size of the prototype's header, which had no room for the manifest pointers, the checksum ids or a header length. The v1.0 header is 64 bytes, which adds 32 bytes to a cold open.

### 1.1 Byte order

Every multi-byte integer and every `f64` in the prefix is little-endian. The byte-order marker is `0x01020304` written little-endian, and a reader that sees it any other way refuses the file rather than byte-swapping it. Floats are IEEE 754 binary64.

### 1.2 The header

The header is 64 bytes in version 1.0 and never more than 4096. Offsets are from the first byte of the file. The reasoning for 64 rather than 32 bytes is in [ADR 0003, Amendment 2026-10-23](adr/0003-physical-container.md#amendment-2026-10-23).

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 8 | magic | bytes | `89 50 56 45 43 0D 0A 1A`; checked first, from these eight bytes alone |
| 8 | 2 | container_major | u16 | a reader refuses any major it does not implement; this reader implements 1 |
| 10 | 2 | container_minor | u16 | a higher minor parses; it only ever adds fields in reserved space or after the fixed part |
| 12 | 4 | byte_order | u32 | `0x01020304` written little-endian, so the bytes on disk are `04 03 02 01` |
| 16 | 2 | header_len | u16 | 64 to 4096; a reader skips whatever it does not know up to this length |
| 18 | 2 | reserved_a | u16 | zero |
| 20 | 1 | block_checksum | u8 | per-block checksum algorithm id: 1 is XXH3-64 (ADR 0006), 2 is SHA-256 |
| 21 | 1 | artifact_checksum | u8 | artifact-level checksum algorithm id, same ids |
| 22 | 2 | reserved_b | u16 | zero |
| 24 | 8 | manifest_offset | u64 | absolute offset of the manifest, at least `header_len` |
| 32 | 8 | manifest_length | u64 | manifest length, 1 to 1048576 bytes |
| 40 | 8 | directory_table_offset | u64 | absolute offset of the directory-pointer table, inside the manifest |
| 48 | 8 | directory_table_length | u64 | table length, a multiple of 24 (zero when there are no directories) |
| 56 | 8 | reserved_c | u64 | zero |

The manifest and the directory-pointer table are located by absolute offsets and lengths. A reader checks every pair against the artifact length before using it: the offset must be at least `header_len`, the sum must not overflow, and the end must not pass the end of the file. A pointer that fails is refused, never clamped. The directory-pointer table lies inside the manifest, at `manifest_offset + 128 + 16 * capability_count`, and the header and the manifest must agree on where.

### 1.3 The manifest

The manifest is the fixed part below, then in this order the capability table (`capability_count` entries of 16 bytes), the directory-pointer table (`directory_count` entries of 24 bytes), the producer id, the producer version and the corpus revision as UTF-8 with no terminator, and zero bytes up to the next multiple of 8. Nothing is allocated from a count before the bytes it implies are known to be inside the manifest.

Fixed part, offsets from the first byte of the manifest:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 2 | length_unit | u16 | unit code from the table in section 1.5 |
| 2 | 2 | flags | u16 | bit 0: the artifact digest is present; every other bit is zero |
| 4 | 4 | capability_count | u32 | number of capability entries |
| 8 | 8 | metres_per_unit | f64 | exactly the unit's factor from the table in section 1.5 |
| 16 | 8 | min_x | f64 | source bounds in source space, in the declared unit |
| 24 | 8 | min_y | f64 | see `min_x` |
| 32 | 8 | max_x | f64 | see `min_x`; at least `min_x` |
| 40 | 8 | max_y | f64 | see `min_x`; at least `min_y` |
| 48 | 2 | min_reader_major | u16 | the lowest capability-registry reader version that may open the file |
| 50 | 2 | min_reader_minor | u16 | see `min_reader_major` |
| 52 | 2 | min_reader_patch | u16 | see `min_reader_major` |
| 54 | 2 | reserved_a | u16 | zero |
| 56 | 4 | directory_count | u32 | number of directory pointer entries |
| 60 | 4 | reserved_b | u32 | zero |
| 64 | 8 | block_checksums_offset | u64 | per-block checksum table (P4.6); zero with a zero length means not written |
| 72 | 8 | block_checksums_length | u64 | see `block_checksums_offset` |
| 80 | 32 | artifact_digest | bytes | artifact-level digest (P4.6); all zero unless flag bit 0 is set |
| 112 | 4 | producer_id_len | u32 | bytes of UTF-8, 1 to 1024 |
| 116 | 4 | producer_version_len | u32 | bytes of UTF-8, 0 to 1024 |
| 120 | 4 | corpus_revision_len | u32 | bytes of UTF-8, 0 to 1024; zero means there is no fixture or corpus revision |
| 124 | 4 | reserved_c | u32 | zero |

Bounds are `f64` in source space and obey the numeric policy (`NP-01`, `NP-02`, `NP-03`): finite, normal or zero, at most 2^40 in magnitude, negative zero read as positive zero, and `min <= max` on both axes. A NaN or infinite bound fails parsing.

Capability entry:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | id | u32 | stable capability id from the registry (P3.4); never zero, never repeated |
| 4 | 2 | version_major | u16 | the capability version the file was written with |
| 6 | 2 | version_minor | u16 | see `version_major` |
| 8 | 2 | version_patch | u16 | see `version_major` |
| 10 | 1 | requirement | u8 | 1 required, 2 optional |
| 11 | 1 | reserved_a | u8 | zero |
| 12 | 4 | reserved_b | u32 | zero |

An id the reader does not implement still parses; what to do about it is version negotiation (P4.7).

Directory-pointer entry:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | kind | u32 | 1 block directory (P4.2), 2 component directory (P4.3), 3 spatial directory (P5.1); later kinds are skipped by a reader that does not know them |
| 4 | 4 | reserved | u32 | zero |
| 8 | 8 | offset | u64 | absolute offset, at least `header_len` |
| 16 | 8 | length | u64 | length; offset plus length is at most the artifact length |

### 1.4 Metadata and digests

The producer id and version are supplied by the caller and the producer id is never empty. The corpus revision is present only for a file built from a fixture or corpus. The block checksum table pointer and the artifact digest are written by P4.6; until then they are absent (zeros, flag bit 0 clear), which is a valid manifest.

### 1.5 Units of length

The unit is declared as an ISO 10303-41 length unit and the factor to metres is stored next to it, which must equal the table's value exactly.

| code | unit | metres per unit | ISO 10303-41 declaration |
|---|---|---|---|
| 1 | Metre | 1 | `SI_UNIT($,.METRE.)` |
| 2 | Millimetre | 0.001 | `SI_UNIT(.MILLI.,.METRE.)` |
| 3 | Centimetre | 0.01 | `SI_UNIT(.CENTI.,.METRE.)` |
| 4 | Kilometre | 1000 | `SI_UNIT(.KILO.,.METRE.)` |
| 5 | Inch | 0.0254 | `CONVERSION_BASED_UNIT('INCH',0.0254)` |
| 6 | Foot | 0.3048 | `CONVERSION_BASED_UNIT('FOOT',0.3048)` |

### 1.6 Reserved space

Every field named `reserved` is zero in version 1.0 and a reader refuses a file where one is not, so a later minor version can give it a meaning without an old reader silently misreading it. Padding between sections is zero for the same reason, which also makes the encoding canonical: one value, one byte string.

### 1.7 Growth rules

* A change that an old reader could misread bumps the major version, and an old reader refuses it with `UnsupportedVersion`.
* A change that is safe to ignore bumps the minor version: new fields go in reserved space, or after the fixed part of the header (the header's own length says where it ends) or after the last manifest section. A reader of a lower minor skips them.
* Trailing bytes after the manifest's last section are refused in a file whose minor version is the reader's, because there they can only be corruption.
* New capabilities, directory kinds and codecs are added by id (ADR 0008), never by changing the layout above.
* The header never grows past 4096 bytes and the manifest never past 1 MiB, so a reader's first fetch stays bounded.

### 1.8 Refusals

`ContainerError` has one variant for each: bad magic, unsupported major version, unsupported byte order, a truncated prefix, a bad header length, a pointer outside the artifact, an unknown checksum algorithm, a non-zero reserved field, a unit or factor that is not in the table, a bound that fails the numeric policy, a duplicate or malformed capability, a mismatched directory table and trailing bytes.
