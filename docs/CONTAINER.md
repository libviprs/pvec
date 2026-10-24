# The PVec container

This file is generated from `crates/pvec-container/src/spec.rs` and the `hot.rs`, `arena.rs`, `advanced.rs`, `directory.rs` and `writer.rs` modules under `crates/pvec-container/src/blocks/`. I do not edit it by hand: a test regenerates it and fails if the committed file differs by a single byte. To change it I change the constants and run `PVEC_BLESS_CONTAINER=1 cargo test -p pvec-container --test container_doc`.

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

The manifest and the directory-pointer table are located by absolute offsets and lengths. A reader checks every pair against the artifact length before using it: the offset must be at least `header_len`, the sum must not overflow, and the end must not pass the end of the file. A pointer that fails is refused, never clamped. The directory-pointer table lies inside the manifest, at `manifest_offset + 128 + 16 * capability_count`, and the header and the manifest must agree on where. A directory, or the block checksum table, that overlaps the manifest's own bytes is refused, since the manifest is the one range every other pointer must stay clear of.

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

`ContainerError` has one variant for each: bad magic, unsupported major version, unsupported byte order, a truncated prefix, a bad header length, a pointer outside the artifact, a directory table length that is not a multiple of 24, a directory or checksum table inside the manifest, an unknown checksum algorithm, a non-zero reserved field, a unit or factor that is not in the table, a bound that fails the numeric policy, a duplicate or malformed capability, a mismatched directory table and trailing bytes.

## 2. Hot geometry blocks

Simple geometry that a viewport reads over and over lives in hot blocks: one block per geometry type, holding parallel columns (structure of arrays, layout B of ADR 0004) and not one serialised record per entity. A reader decodes a block by slicing its columns out of the bytes, so a viewport costs a bounded number of column copies and no per-entity pointer chase or branch. Which spatial partition a block belongs to is decided by the spatial index (P5.1, P5.2), not here.

### 2.1 Block header

The header is 32 bytes, little-endian, and the first byte of the block must be 8-aligned in memory (the reader refuses a buffer that is not, rather than copying it). Offsets are from the first byte of the block.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | magic | bytes | `50 56 48 31` (`PVH1`) |
| 4 | 2 | version | u16 | 1; a reader refuses any other |
| 6 | 2 | kind | u16 | geometry type, from the table in section 2.2 |
| 8 | 4 | entity_count | u32 | entities in the block, 1 to 2^28 - 1 |
| 12 | 4 | vertex_count | u32 | total vertices of a polyline block; zero for every other kind |
| 16 | 4 | block_length | u32 | length of the whole block in bytes, equal to the bytes present |
| 20 | 12 | reserved | bytes | zero |

### 2.2 Kinds and columns

Coordinates are `f64` end to end: there is no quantisation or rounding in this layer, and a value comes back with the bits it went in with. They are finite (`NP-01`); a NaN or an infinity refuses the block. These are the immediate display forms (a line is its two end points); the exact curve parameters of ADR 0011 stay in the codec sections of P3.2.

| id | kind | `f64` columns, in order | meaning |
|---|---|---|---|
| 1 | Line | `x0`, `y0`, `x1`, `y1` | start point (`x0`, `y0`) and end point (`x1`, `y1`), one entry per segment |
| 2 | Polyline | `x`, `y` | all x, then all y, concatenated run by run; run `i` is `offsets[i]..offsets[i+1]` |
| 3 | CircleArc | `cx`, `cy`, `radius`, `start`, `sweep` | centre, radius, start angle and signed sweep in radians; a full circle sweeps 2 pi |
| 4 | EllipseArc | `cx`, `cy`, `rx`, `ry`, `rotation`, `start`, `sweep` | centre, the two semi-axes, the rotation of the first axis, start angle and signed sweep in radians |

### 2.3 Column order and alignment

After the header the columns follow in this order: the entity ids (`u32`, `n` of them), the style references (`u32`, `n`), for a polyline block the run offsets (`u32`, `n + 1`), zero padding up to a multiple of 8, then the `f64` columns of the kind, each `n` long (a polyline block: the vertex count) with no padding between them. Every column starts at its natural alignment: a `u32` column on a multiple of 4 and an `f64` column on a multiple of 8, from the first byte of the block. Because the block itself starts on 8, a decoded block is directly loadable, including in WASM linear memory, where a `Float64Array` over the memory throws on a byte offset that is not a multiple of 8. The only padding a block can contain is the gap before the first `f64` column (0 or 4 bytes), the block is a multiple of 8 bytes long, and the header's `block_length` is that length.

Padding is zero and a reader refuses a block where it is not, which keeps the encoding canonical: decoding and re-encoding gives the same bytes.

### 2.4 Order, ids and styles

Entities are stored in strictly ascending `EntityId` order (P3.6), so the same set of entities always encodes to the same bytes whatever order the writer saw them in (P4.4). An id is below `2^28` (NP-18) and appears once. The style reference is an opaque `u32` per entity that the style component stream (P3.7) resolves; this layer stores it and does not interpret it. ADR 0011 left references out of the geometry codec because the component streams point at entities by id, but a viewport that draws from these blocks needs the style next to the coordinates, and four bytes per entity is the price.

### 2.5 Polyline runs

The run offsets index the `x` and `y` columns. The first is 0, they never decrease, the last is the vertex count in the header, and every run has between 2 and 1048576 vertices. A block whose run offsets break any of these is refused whole and never read. The vertex count is at most `entity_count * 1048576`, so a lying count cannot size anything before the bytes are checked.

### 2.6 Refusals

Each refusal is a typed `HotError` with a rule identifier. Every count is compared with the bytes left before it is used, and decoding allocates nothing.

| rule | meaning |
|---|---|
| HOT-01 | the block is shorter than its structure requires |
| HOT-02 | bad magic |
| HOT-03 | unsupported version |
| HOT-04 | unknown kind id |
| HOT-05 | a reserved header field is not zero |
| HOT-06 | a padding byte is not zero |
| HOT-07 | the declared length differs from the bytes present, or part of the block is unused |
| HOT-08 | no entities |
| HOT-09 | more entities than NP-18 allows |
| HOT-10 | a vertex count that the entity count cannot account for, or vertices in a block with no runs |
| HOT-11 | an entity id at or above the NP-18 limit |
| HOT-12 | ids that are not strictly ascending (the builder reports a repeated id here too) |
| HOT-13 | run offsets that decrease |
| HOT-14 | run offsets that do not start at zero or do not end at the vertex count |
| HOT-15 | a polyline run of fewer than 2 or more than 2^20 vertices |
| HOT-16 | a NaN or infinite coordinate (NP-01) |
| HOT-17 | the buffer does not start on an 8-byte boundary |
| HOT-18 | the host is not little-endian, so columns cannot be borrowed in place |
| HOT-19 | (builder) an entity of another kind than the block |
| HOT-20 | (builder) the block would not fit 32-bit lengths |

## 3. Advanced geometry arenas

B-spline knot vectors, weight arrays, control-point sets, composite segment lists and trim domains have a different length for every entity, which the fixed columns of chapter 2 cannot express. They live in arenas: one block per payload kind, holding every entity's values back to back, with an offset table that says where each entity's run starts and how long it is. An advanced curve is a fixed record of integers, so the curve is located by two integer reads (its record, then the offset-table entry of the payload kind it needs) and each payload is one contiguous run. This is layout B of ADR 0004 and the panel's sixth finding: there is no nested per-entity byte blob to chase and no per-entity allocation. The exact curve parameters are those of ADR 0011 and `docs/GEOMETRY_ENCODING.md`; this chapter is the container's view of them.

### 3.1 Arena block

An arena block is a 32 byte header, the offset table, and the payload, all little-endian. The first byte of the block must be 8-aligned in memory (the reader refuses a buffer that is not, rather than copying it). Offsets are from the first byte of the block.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | magic | bytes | `50 56 41 31` (`PVA1`) |
| 4 | 2 | version | u16 | 1; a reader refuses any other |
| 6 | 2 | payload_kind | u16 | what the arena holds, from the table in section 3.2 |
| 8 | 4 | entry_count | u32 | entries in the offset table |
| 12 | 4 | element_count | u32 | elements in each payload column, the arena length the entries are checked against |
| 16 | 4 | block_length | u32 | length of the whole block in bytes, equal to the bytes present |
| 20 | 12 | reserved | bytes | zero |

After the header come `entry_count` offset-table entries of 8 bytes each (see section 3.3), which leaves the payload on a multiple of 8, and then the payload columns, each `element_count` elements long, one after the other with no padding between them. Every column starts at its natural alignment: an `f64` column on a multiple of 8 and a `u32` column on a multiple of 4, from the first byte of the block, which is what a `Float64Array` over WASM linear memory needs. The block is a multiple of 8 bytes long, so the only padding it can contain is the 0 or 4 bytes after a segment arena with an odd element count. Padding is zero and a reader refuses a block where it is not, which keeps the encoding canonical.

### 3.2 Payload kinds

| id | kind | element | columns | one entry holds |
|---|---|---|---|---|
| 1 | Knots | f64 | `knots` | one knot vector per entry; the same vector may serve many curves |
| 2 | Weights | f64 | `weights` | one weight per control point of a rational curve |
| 3 | ControlPoints | f64 | `x`, `y` | all x of the entry, in one column, and all y at the same positions in the second |
| 4 | Segments | u32 | `segments` | record indices naming the segments of a composite curve, in order |
| 5 | Trims | f64 | `trims` | `t1`, `t2` per segment of a composite, flattened: twice as long as its segment list |

Coordinates, knots, weights and trims are `f64` end to end, so a value comes back with the bits it went in with. The two control-point columns have the same length and the entry's range applies to both.

### 3.3 The offset table

An entry is `start: u32` then `len: u32`, both counted in elements (not bytes) of the arena's columns. Entry `i` of the table is at `32 + 8 * i`. The entry names the run `start..start + len` of each column. Nothing requires entries to be ordered or disjoint: two entries may name the same range, and that is how equal payloads share storage (section 3.6). An index of `ffffffff` in a record means the curve has no payload of that kind; it is never a table index.

A reader checks an entry in range before it hands out a slice: `start + len` is added with overflow checked, and the sum must not exceed `element_count`. An entry that fails is refused, never clamped. The table is checked entry by entry as it is used, so opening an arena does not cost anything in the number of entries.

### 3.4 Advanced curve records

The record block is a 32 byte header followed by `record_count` records of 32 bytes each, in strictly ascending entity id order (P3.6), so the same set of curves always encodes to the same bytes (P4.4). A record is degree, flags and one offset-table index per arena, and nothing else: there is no nested per-entity byte blob, and a record never holds a number of the curve.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | magic | bytes | `50 56 52 31` (`PVR1`) |
| 4 | 2 | version | u16 | 1; a reader refuses any other |
| 6 | 2 | reserved_a | u16 | zero |
| 8 | 4 | record_count | u32 | records in the block, 1 to 2^28 - 1 |
| 12 | 4 | block_length | u32 | length of the whole block in bytes, equal to the bytes present |
| 16 | 16 | reserved_b | bytes | zero |

The record:

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 4 | id | u32 | entity id, below 2^28; records are in strictly ascending id order |
| 4 | 4 | style | u32 | opaque style reference, as in the hot blocks |
| 8 | 1 | kind | u8 | 1 spline, 2 composite |
| 9 | 1 | degree | u8 | 1 to 16 for a spline, 0 for a composite |
| 10 | 2 | flags | u16 | bit 0 rational, bit 1 closed, bit 2 periodic (refused); the rest zero |
| 12 | 4 | knots | u32 | index into the knots offset table; `ffffffff` for a composite |
| 16 | 4 | weights | u32 | index into the weights offset table; `ffffffff` unless rational |
| 20 | 4 | control_points | u32 | index into the control-points offset table; `ffffffff` for a composite |
| 24 | 4 | segments | u32 | index into the segments offset table; `ffffffff` for a spline |
| 28 | 4 | trims | u32 | index into the trims offset table; `ffffffff` for no trims |

A spline (kind 1) needs knots and control points, and weights if and only if the rational flag is set; it has no segment list or trims. A composite (kind 2) has a degree of 0, a segment list, optionally trims, and no knots, weights or control points; it cannot be rational. The closed flag is a claim about the geometry that NP-20 checks for a spline (the end meets the start); for a composite it is carried and not checked, because checking it needs the segments evaluated, which is not this layer's job.

The periodic flag is defined and refused. ISO_PROFILE section 11 and ADR 0011 say the profile has no wrapped periodic form (a closed curve is an ordinary clamped B-spline), so there is no periodic rule to apply, and a record that sets the bit makes the whole block unreadable rather than half-understood. A periodic spline is unwrapped before it is stored (`BSpline::from_periodic`, ADR 0011), so the writer never has a reason to set the bit. A later minor version can give the bit a meaning by lifting that refusal.

### 3.5 Counts, weights and composites

For a spline of degree `p` with `n` control points the knot vector has `n + p + 1` entries and a rational curve has exactly `n` weights. A weight array that is shorter is refused, not padded with ones, and so is one that is longer. Beyond the counts, the knots go through NP-14 (non-decreasing, multiplicities, no ambiguous pairs), the weights through NP-15, every coordinate through NP-01 and NP-02, and the closed flag through NP-20.

A composite's segment list is a list of record indices: it names other records of the same block, so a segment that two composites use is one record and its geometry is stored once. The list has 1 to 65536 entries (NP-18) and, when the composite has trims, the trims entry has two numbers `t1`, `t2` per segment. This layer does not check a trim against the curve it trims (NP-12 needs that curve's domain), and it only references records of its own block: a segment that is a hot line or arc has to be written as a degree 1 spline here.

Composites may nest up to the depth of NP-18, counting the outermost curve as 1, so a composite of composites of splines is depth 3. This is an allowance of the container, which stores a composite as references so that shared geometry is stored once. It is not a change to the Part 21 form of `docs/ISO_PROFILE.md` section 9, where a `composite_curve` segment's parent is never another composite: a writer of Part 21 (P6.2) expands nested references into one flat list of segments, as that section says, and ADR 0004 records the amendment. `curve` follows nothing. `nesting_depth` walks the references with each record read once, and refuses a cycle (a composite that leads back to itself, itself included) and a chain deeper than the limit before anything is resolved. `check_nesting` does the same for every record at once and scans each segment list once, however many composites share it, so its cost is the size of the file and not the product of composites and list lengths. The writer runs it over everything it wrote and refuses the set if it fails.

### 3.6 Sharing and canonical order

An arena builder adds a payload once: if the same bits were added before, it returns the existing table index. Two entities with the same knot vector therefore share one table entry and one arena range, and the record of each points at it. Entries are numbered in the order the writer first needs them, and the writer visits curves in ascending entity id order, so the bytes do not depend on the order curves were pushed in. Sharing compares bits, not values: vectors one ulp apart are two payloads.

### 3.7 What a reader may assume and what it must verify

A reader may assume, once the six blocks have parsed: every block is exactly as long as its header says, its padding is zero, and every column is inside the buffer and at its natural alignment; the record block's ids ascend and are below 2^28; every record has a known kind, only defined flags, no periodic flag, and exactly the index fields its kind uses; and each arena is the kind its slot expects.

A reader must verify, as it uses them: that an offset-table index is below the entry count and the entry lies inside the arena (`ArenaView::slice` and its siblings do both); that the numbers are finite and satisfy the numeric policy (`AdvancedView::curve` does, for a spline); that a composite's segment indices name records and its trims are twice as many as its segments (`curve` does); and that nesting is acyclic and shallow enough (`nesting_depth`, which a reader calls before it resolves segments). It must also apply the ceilings of P4.8, which cite NP-18 and are not restated here, and it must not assume two entries are disjoint.

Reading one curve costs at most one offset-table read and one arena read for each payload kind it has, and the number does not depend on how many curves are in the file; a test counts them with a probe.

### 3.8 Refusals

Each refusal is a typed `ArenaError` with a rule identifier. Every count is compared with the bytes present before it is used, and parsing allocates nothing.

| rule | meaning |
|---|---|
| ARN-01 | the block is shorter than its structure requires |
| ARN-02 | bad magic |
| ARN-03 | unsupported version |
| ARN-04 | an unknown payload or record kind id, or an arena of the wrong kind in its slot |
| ARN-05 | a reserved field is not zero |
| ARN-06 | a padding byte is not zero |
| ARN-07 | the declared length differs from the bytes present, or part of the block is unused |
| ARN-08 | the buffer does not start on an 8-byte boundary |
| ARN-09 | the host is not little-endian, so payloads cannot be borrowed in place |
| ARN-10 | an offset-table index or a record index at or past the end of its table |
| ARN-11 | an entry whose start plus len passes the arena length, or overflows |
| ARN-12 | an accessor used on an arena of another payload kind |
| ARN-13 | the knot count is not control points plus degree plus one |
| ARN-14 | weights that are missing, extra, or not one per control point (never padded) |
| ARN-15 | control-point x and y columns of different lengths |
| ARN-16 | the periodic flag is set; this profile has no wrapped periodic form |
| ARN-17 | flag bits that are not defined, or a flag the record kind cannot have |
| ARN-18 | a record field its kind does not use is set, or one it needs is empty |
| ARN-19 | record ids that are not strictly ascending, or at or above the NP-18 limit |
| ARN-20 | a segment that names a record that does not exist |
| ARN-21 | a composite whose segment lists lead back to itself |
| ARN-22 | composites nested deeper than NP-18 allows |
| ARN-23 | trim values that are not two per segment |
| ARN-24 | a numeric policy rule (NP-nn) refused a count or a value; the error carries it |
| ARN-25 | a block with no records |
| ARN-26 | (builder) the same entity id twice |
| ARN-27 | (builder) a composite names an entity that was not pushed |
| ARN-28 | (builder) the blocks would not fit 32-bit lengths |

## 4. The writer

The writer turns semantic geometry into one artifact, and the same input gives a byte-identical artifact: the same file whatever order the entities, capabilities and streams arrive in, whichever thread pushed them, whichever process or machine wrote it, and whenever it ran. An artifact that differed between two runs could not be cached by digest, could not be diffed in review and would make every fidelity report unreproducible. The caller decides the entity ids, the styles, the cells, the bounds, the unit, the producer identity and the capabilities to declare; the writer decides everything else.

### 4.1 File layout

The artifact is, in this order and with nothing between them but zero padding: the 64 byte header at offset 0, the manifest at offset 64, the blocks, and the block directory, which ends the file. Every block starts on a multiple of 8 bytes from the first byte of the file, the directory too, so a block can be mapped and read in place. The manifest holds one directory pointer, of kind 1 (the block directory), and declares no block checksum table and no artifact digest: P4.6 adds those, and an artifact without them is a valid one (section 1.4). The header and the manifest agree on where the directory pointer table lies, as chapter 1 requires.

### 4.2 The block directory

The block directory is a table of 64 byte entries, one per block, in file order. A block carries its offset, its stored length, the length once decoded (equal while no codec is used), a count, its spatial cell, the bounds of its entities, a kind and a subtype, flags and a codec id. ADR 0003 priced the entry at 56 bytes and ADR 0006 at 64 once the codec and the uncompressed length were added; ADR 0008's kind and flags fit in the same 64 bytes.

| offset | size | name | encoding | meaning |
|---|---|---|---|---|
| 0 | 8 | offset | u64 | absolute offset of the stored block, a multiple of 8 |
| 8 | 4 | stored_length | u32 | bytes of the block as stored |
| 12 | 4 | uncompressed_length | u32 | bytes of the block once decoded; equal to `stored_length` while the codec is 0 |
| 16 | 4 | count | u32 | entities, records, offset-table entries or streams in the block |
| 20 | 4 | cell | u32 | spatial cell id (P5.1); 0 for a block that has none |
| 24 | 8 | min_x | f64 | bounds of the entities in the block; zero unless the has-bounds flag is set |
| 32 | 8 | min_y | f64 | see `min_x` |
| 40 | 8 | max_x | f64 | see `min_x` |
| 48 | 8 | max_y | f64 | see `min_x` |
| 56 | 2 | kind | u16 | block kind from the table in section 4.2 |
| 58 | 2 | subtype | u16 | geometry type of a hot block, payload kind of an arena, otherwise 0 |
| 60 | 1 | flags | u8 | bit 0 must-understand, bit 1 has-bounds; the rest zero |
| 61 | 1 | codec | u8 | 0 stored as it is; later ids belong to P4.6 |
| 62 | 2 | reserved | u16 | zero |

The block kinds:

| id | name | holds | subtype | count |
|---|---|---|---|---|
| 1 | hot | a hot geometry block (chapter 2) | the hot geometry type, 1 to 4 | entities |
| 2 | records | the advanced curve records (chapter 3) | 0 | records |
| 3 | arena | one advanced payload arena (chapter 3) | the payload kind, 1 to 5 | offset-table entries |
| 4 | components | the component stream set | 0 | streams |

Flag bit 0 is must-understand: a reader that does not know the kind refuses the file (ADR 0008). Geometry blocks carry it; the component stream set does not, so a reader that does not know it skips it. Flag bit 1 says the bounds are meaningful: a hot block and the record block carry the union of the bounds the caller gave their entities, an arena or a component set carries none and its four bounds are zero. The codec is 0 (stored as it is); P4.6 adds the others. A reader checks every entry before using it:

| rule | meaning |
|---|---|
| DIR-01 | the table is not a whole number of 64 byte entries |
| DIR-02 | a reserved field is not zero |
| DIR-03 | a flag bit other than must-understand and has-bounds is set |
| DIR-04 | a must-understand block of a kind the reader does not know (an optional one is kept and skipped) |
| DIR-05 | a codec id the reader does not know on a must-understand block (on an optional one the entry is kept and skipped) |
| DIR-06 | a block that starts before the end of the header, runs past the end of the artifact, or whose end overflows |
| DIR-07 | a block that does not start on a multiple of 8 |
| DIR-08 | a block that starts before the previous block ends |
| DIR-09 | stored and uncompressed lengths that differ while the codec is 0 |
| DIR-10 | bounds that are not finite, in range and ordered, or non-zero without the has-bounds flag |
| DIR-11 | a block that overlaps the manifest or the directory table (checked when the caller knows where they are) |

### 4.3 Canonical order

Every order below is decided by the writer from the content, never from the order of arrival. A block is one group of entities that share a block kind, a spatial cell id and a geometry type; the writer does not split a group by size (that is tuning, P8.6), so a group is one block.

| key | orders | ascending by |
|---|---|---|
| block kind | the blocks of the file | the kind id of section 4.2, so hot blocks come first and the component set last |
| spatial cell id | the blocks of one kind | the cell id the caller gave the entity (P5.1 decides what a cell is; the writer only orders by it) |
| geometry type | the blocks of one kind and cell | the hot geometry type or the payload kind |
| `EntityId` | the entities inside a block | the id, so a block's id column ascends and equal sets of entities give equal columns |
| capability id | the manifest's capability table | the stable id from the registry (P3.4), each declared once |
| stream id | the streams of the component set | the capability id, which is the id of a stream (P3.7) |

The advanced curves are one set of six blocks (the record block and the five arenas) in cell 0, because a composite names other records of its own block, so a curve cannot be split from its segments by cell; an advanced curve given another cell is refused. In short, cell 0 is both no cell (the advanced set and the component set) and an ordinary cell a hot block can be in, so a block in cell 0 does not say which it is: P5.1 gives the spatial directory the way to tell the two apart (the block kind does today). The records are in ascending `EntityId` order and the arenas number their entries in the order the writer first needs them, which is the same order, so shared payloads land on the same entry whatever order the curves were pushed in (section 3.6). The capabilities are those the caller listed plus those of the streams pushed, each once, and each is declared with the version and requirement the registry gives it; the manifest's minimum reader is the highest the declared capabilities ask for, and at least 1.0.0.

Every scalar of every block, the directory and the manifest is written with an explicit little-endian conversion, so nothing depends on the byte order of the host that wrote it. The tests write the same input on x86_64 and on arm64 and compare the digests with a committed golden file.

### 4.4 Stable metadata

The producer id, the producer version and the corpus revision are exactly what the caller gives the writer, and there is no timestamp. No absolute path, host name, user name, environment variable, process id, hash-map iteration order or address-derived value is read, so a file written from another directory, host or time zone is the same file. Bounds are normalised so that negative zero is written as positive zero. The bounds `push_curve` derives for a model curve are conservative boxes from comparisons and additions only (vertices, control points, end points, or the whole circle or ellipse for an arc), because the model's own bounds of an arc go through `sin_cos` and `atan2`. The writer calls no transcendental function: a curve whose stored form would need one (an arc on a rotated frame, whose angle needs `atan2`) is refused (WRT-08) instead of being stored with a last bit that belongs to the C library of the machine.

### 4.5 Refusals

Each refusal is a typed `WriteError` with a rule identifier.

| rule | meaning |
|---|---|
| WRT-01 | there is no entity to write (component streams alone are not an artifact) |
| WRT-02 | an entity id is used by two entities, whatever their geometry types |
| WRT-03 | an advanced curve was given a spatial cell; cells belong to hot blocks |
| WRT-04 | a capability id the registry does not know |
| WRT-05 | two component streams of one capability |
| WRT-06 | bounds that are not finite, in range and ordered, on an entity or as the source bounds |
| WRT-07 | a block or the artifact does not fit its 32-bit length fields |
| WRT-08 | a curve the container cannot store exactly and deterministically |
| WRT-09 | a hot block builder refused (the error carries its HOT rule) |
| WRT-10 | an advanced block builder refused (the error carries its ARN rule) |
| WRT-11 | the component stream encoder refused |
| WRT-12 | the manifest refused (the error carries its container error) |
| WRT-13 | the semantic model refused |

### 4.6 Review checklist

A change to the writer, the directory or a block builder is reviewed against these, and the `determinism` tests and the source scan in `tests/determinism.rs` enforce them:

* No wall-clock time, no `SystemTime` or `Instant`, reaches the output.
* No absolute path, no current directory, no environment variable and no hostname reaches the output.
* No hash-map iteration order reaches the output: a `HashMap` or `HashSet` may be looked up, never iterated.
* No pointer value or address reaches the output, and no pointer is formatted.
* Every scalar is written with `to_le_bytes` (or its sibling), never with a native-endian conversion.
* No transcendental function runs on the write path, because its last bit is the platform's.
* A new canonical-ordering key gets a `SortKey`, a fixture where it matters and a mutation test that goes red without it; a key with no mutation test is not covered.
* A new field of the artifact is written from caller-supplied or derived values only.
