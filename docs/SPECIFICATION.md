# PVec v1 specification

This page assembles the v1 specification from the artefacts [FREEZE-v1.md](FREEZE-v1.md) lists. It adds no normative
statement of its own: each requirement below points at the page that decided it, and that page wins if the two ever differ.
Where FREEZE-v1.md says a row is `NOT FROZEN`, the page it points at is still the current text but may change before the
freeze, and this page says so.

## What a v1 file is

A `.pvec` file is one container: a 64 byte header, a manifest, geometry and index blocks, a block checksum table and a block
directory ([CONTAINER.md](CONTAINER.md) chapters 1 and 4). Byte order is little endian. Geometry is the 2D semantic profile
of [ISO_PROFILE.md](ISO_PROFILE.md): PVec is aligned to named ISO 10303-42:2025 constructs and validated through
ISO 10303-21 interchange, with no claim of formal Part 42 conformance. Numbers obey the policy in
[NUMERIC_POLICY.md](NUMERIC_POLICY.md) and nothing else defines a tolerance.

| Part | Normative text |
|---|---|
| header, manifest, units | [CONTAINER.md](CONTAINER.md) sections 1.1 to 1.8 |
| hot blocks and advanced arenas | [CONTAINER.md](CONTAINER.md) chapters 2 and 3, [GEOMETRY_ENCODING.md](GEOMETRY_ENCODING.md) |
| writer, block directory, integrity | [CONTAINER.md](CONTAINER.md) chapter 4 |
| capabilities, negotiation, skipping | [CONTAINER.md](CONTAINER.md) chapter 5, [CAPABILITIES.md](CAPABILITIES.md), [COMPATIBILITY.md](COMPATIBILITY.md) |
| spatial directory, straddlers, Tier 0, viewport plans | [STREAMING.md](STREAMING.md) |
| resource limits | [SECURITY_LIMITS.md](SECURITY_LIMITS.md) |
| fidelity classes and the three verdicts | [FIDELITY_TAXONOMY.md](FIDELITY_TAXONOMY.md), [CONFORMANCE.md](CONFORMANCE.md) |
| the packed buffers of the browser build | [WASM_ABI.md](WASM_ABI.md) |
| what may change within v1.x | [MIGRATION.md](MIGRATION.md) |

Not part of v1: a C header (deferred, see FREEZE-v1.md), and any 3D construct.

## Third-party reader checklist

A reader written from these documents alone does, in this order:

1. Read the first 4096 bytes and parse the header (section 1.2). Check the magic and the byte-order marker, refuse a major version it does not implement, and check every offset and length against the file length before using it.
2. Parse the manifest and enumerate the required capabilities: the capability entries of section 1.3 whose requirement byte is 1. Refuse the file, by name, if it lacks one of them. An optional capability it lacks is noted and its blocks are skipped.
3. Locate the block directory through the directory pointer of kind 1, and check every 64 byte entry against rules DIR-01 to DIR-11 (section 4.2).
4. Be able to skip an unknown optional block from its directory entry alone: kind unknown and flag bit 0 clear means skip, without requesting, checksumming or parsing the payload. An unknown must-understand block refuses the file (section 5.3).
5. Decode the blocks it knows within the limits of SECURITY_LIMITS.md, applying the numeric policy to every number it reads.
6. Report display results as display fidelity and never as geometric or structural exactness (CONFORMANCE.md).

The worked walkthrough of steps 1 to 4, against the annotated hex dump in [CONTAINER.md](CONTAINER.md#6-an-annotated-example), is the
test `crates/pvec-container/tests/issue_67_walkthrough.rs`: it reads the bytes out of the document, lists the required
capabilities, and skips the optional component block, using only the offsets in the tables.
