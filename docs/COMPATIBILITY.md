# PVec compatibility rules

A reader that meets a capability it has never heard of has a few honest options, and the format says which one applies where, so no implementation has to invent its own. This file is the policy. The behaviour is in `crates/pvec-container/src/negotiate.rs` and `crates/pvec-capabilities`, the machine-readable registry snapshot is `docs/capabilities.toml`, and the declared old/new reader matrix is `tests/compat/matrix.txt`. Chapter 5 of `docs/CONTAINER.md` is the generated contract for negotiation, and `docs/CAPABILITIES.md` is the generated registry table.

## Versions

There are two kinds of version and they answer different questions.

* The **format version** (`FORMAT_VERSION`, 1.3.0 today) is the version of the container and the registry together. A reader has one. A major bump is the only place a breaking change is allowed.
* The **capability version** is the semantic version of one capability (`docs/CAPABILITIES.md`, column version). A file declares, in its manifest, every capability it uses with the version it was written at. Within a major version a minor only adds, and what it adds is ignorable by the growth rules of `docs/CONTAINER.md` section 1.7. A patch changes no byte a reader sees.

The **minimum reader support** field is the manifest's `min_reader` (P3.4): the lowest format version that may open the file at all. Each registry entry also carries a `min_reader`, the lowest reader that understands that capability. Both are checked before anything else, and a reader below either refuses the file.

A capability is **required** or **optional**. A required capability is one a reader cannot ignore without misreading the geometry, so a reader that lacks it refuses the file and renders nothing partially. An optional capability only adds information about entities, so a reader that lacks it skips it and says so. A file can declare a capability required when the registry says it is optional, never the other way round, and a capability whose class is never required (component, raster, calibration) cannot be declared required (`NEGOTIATE-04`).

## Decision table

The reader decides from the manifest and the block descriptors alone. No payload is looked at to make any of these decisions.

| Capability class | Reader must | Exact behaviour |
|---|---|---|
| required, unknown | refuse the file with a named error | `Outcome::Refuse(Refusal::MissingRequired { id, version, name: None })`, rule `NEGOTIATE-01`. `ContainerReader::open` returns `ReadError::Negotiation` carrying it, and its message names the capability id and the version the file declares. Capabilities are checked in ascending id order, so the error names the lowest unknown required id; `pvec capabilities` (P7.7) lists all of them. |
| optional skip-safe block, unknown | skip by directory length, read the rest, list it in the capability report | `Outcome::OpenDegraded(PartialOpen)`. The block is listed in `PartialOpen::blocks` as a `SkippedBlock` with `SkipReason::UnknownBlockKind` (or `UnknownCodec`), and its capability in `PartialOpen::capabilities` with `SkipReason::NotImplemented`. The payload is not fetched, checksummed, decompressed or parsed, so a block of garbage with a valid length is skipped. A block with the must-understand bit set is not skip-safe and refuses the file (`DIR-04`). |
| optional component stream, unknown | ignore the stream; geometry conformance is unaffected | `Outcome::OpenDegraded(PartialOpen)` with the capability listed under `SkipReason::NotImplemented` and an `Effect` (`Unstyled` for layers and line styles, `Uncomponented` for the rest). The geometry decoded is bit-identical to the same file without the stream. |
| optional capability-owned block kind (kinds 9 and 10, capability 21), capability not implemented | skip by directory length, list the capability, say what is left undone | `Outcome::OpenDegraded(PartialOpen)`. Each entry is a `SkippedBlock` with `SkipReason::UnknownBlockKind` that names capability 21 from the entry, and 21 is listed under `SkipReason::NotImplemented` with `Effect::Unrastered`. An optional 21 leaves the manifest's `min_reader` at 1.0.0, so a 1.0.0 reader reaches this row and not `NEGOTIATE-03`. A reader that implements 21 is handed the entries instead. A block of one of these kinds that names another capability, or none, refuses the file (`NEGOTIATE-06`). |
| optional capability-owned block kind (kind 11, capability 22), capability not implemented | skip by directory length, list 22, say the file carries a calibration the reader cannot show | `Outcome::OpenDegraded(PartialOpen)`. The kind 11 entry is a `SkippedBlock` with `SkipReason::UnknownBlockKind` naming capability 22, and 22 is listed under `SkipReason::NotImplemented` with `Effect::Uncalibrated` ("this file carries a scale or georeference your reader cannot show"). An optional 22 leaves `min_reader` at 1.0.0, so a 1.0.0 or 1.1.0 reader reaches this row. The geometry is bit-identical. A 22 declared required refuses at `NEGOTIATE-04`. |
| optional paint streams (23 to 25, ADR 0021), not implemented | skip by length, list them, say what is left undone | `Outcome::OpenDegraded(PartialOpen)` with 23, 24 and 25 under `SkipReason::NotImplemented` (an `Effect::Uncomponented` for 23, `Effect::Unstyled` for 24), the payloads not read. All are components at min reader 1.0.0, so a 1.0.0 reader reaches this row. The geometry is bit-identical, and the file draws in the legacy stage order with the viewer's own stroke colour. |
| known, newer minor | read the understood subset and report the version gap | `Outcome::Open`. `negotiate::version_gaps` returns a `VersionGap { id, file, supported }` for each such capability, so a consumer can report it. |
| known, newer major, optional | refuse that capability only | `Outcome::OpenDegraded`, the capability listed under `SkipReason::NewerMajor`, the rest of the file read. |
| known, newer major, required | refuse the file | `Outcome::Refuse(Refusal::IncompatibleMajor { id, file, supported_major })`, rule `NEGOTIATE-02`. |
| file needs a newer reader | refuse the file | `Outcome::Refuse(Refusal::ReaderTooOld { file_requires, reader })`, rule `NEGOTIATE-03`. |

A rewrite that goes through the decoded model refuses when anything was skipped (`PartialOpen::ensure_lossless_rewrite`). A tool that must keep what it did not understand copies the blocks raw.

The matrix in `tests/compat/matrix.txt` declares the outcome of nineteen reader and file pairs, with the readers `s_old` (geometry only, none of 14 to 25), `s_new` (every v1 capability plus a future one), `s_raster` (a 1.1.0 reader with every capability but 22 to 25) and `v1`. The outcomes are written in the fixture, not computed by the harness, and `crates/pvec-container/tests/compat_matrix.rs` turns red if any of them changes.

## Deprecation lifecycle

A capability moves through four states, one step at a time:

`active -> deprecated (written and read) -> write-disabled (read-only) -> removed`

* **active**: written and read.
* **deprecated**: still written and still read; the registry announces it will go. It stays at least 2 minor releases as `deprecated`.
* **write-disabled**: a writer must not produce it; a reader still reads it. It stays at least 2 minor releases as `write-disabled`.
* **removed**: it leaves the registry and its id goes on the retired list for good. Removal requires a major version.

The numbers are `DEPRECATED_MIN_MINORS` and `WRITE_DISABLED_MIN_MINORS` in `pvec-capabilities`, and the document test fails if this text and the constants disagree. They are the proposed defaults of this change and the one place to change them.

`removed` is not a status in `docs/capabilities.toml`: a removed capability is an id on the `[[retired]]` list. Each entry records `status` and `status_since`, the format version at which it entered that status, which is what the minimum is counted from. A new major format version counts as long enough for any state.

## The registry lint

`pvec_capabilities::lint_transition` compares the snapshot on the base branch with the one in a change. `crates/pvec-checks/tests/compatibility_doc.rs` runs it when `PVEC_CAPABILITIES_BASE` names the base snapshot (`git show origin/main:docs/capabilities.toml`). It fails when:

* a capability is removed without being `write-disabled`, or without being on the retired list, or before the minimum number of minor releases;
* a status skips a step, goes back, or is left before its minimum;
* a breaking change lands on a format version whose major did not go up: a removal, a capability major bump, an optional capability becoming required, a raised minimum reader, a changed class;
* a capability version or the format version goes down;
* a live capability uses a retired id.

## What is not promised

A capability marked `unstable` carries no compatibility promise: it can change or vanish in any release, and the rules above do not apply to it. A file that uses one is not a conformant v1 file. The registry holds none today (`docs/capabilities.toml` has only `geometry` and `component` entries), and the unstable surface of the Rust API (`docs/API.md`) is a separate matter.

## Not covered yet

* `wasm_abi_version` follows these rules once the WASM ABI (P7.3) is on main.
* Freezing the registry contents is P8.7, and migration prose is P8.8.
