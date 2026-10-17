# ADR 0003: The physical container

Status: Accepted, 2026-10-16. Decides P1.4 against P1.5. Precondition met: ADR 0002 (the numeric policy) is
already accepted. Both comparison tables are on the same fixture revision, `pvec-p1-fixtures-r1`:
`docs/spikes/01-4-step-hdf.md` and `docs/spikes/01-5-layouts.md`, with the joint table in
`docs/spikes/01-8-benchmark-summary.md`.

## Context

PVec must not invent a container before measuring the standard binary option. ISO/TS 10303-26 STEP-HDF was
a mandatory phase 1 candidate. I did not have the Part 26 text, so the HDF5 candidates are two
HDF5-backed representations of the fixtures (a table per EXPRESS entity type, and a tuned structure of
arrays), not a conformance claim.

## Options

All numbers are native x64 on MARS, `million` (1,000,000 entities) unless stated, and the viewport is the
1/16 x 1/16 step of the committed script (4,031 visible entities).

| option | file bytes (ratio to Part 21) | viewport bytes / ranges | whole script bytes | wasm reader | exactness |
|---|---|---|---|---|---|
| STEP-HDF-style tables (H1), contiguous | 162,091,844 (0.443) | 44,036,240 / 56 | 700,130,652 | 483,228 bytes (pure Rust, newest format only), variable-length path 10 MiB/s | 2,099 entities change last bits |
| HDF5 structure of arrays (H2), contiguous | 100,478,313 (0.275) | 42,297,754 / 36 | 534,690,511 | same reader, 1,097 to 1,130 MiB/s on arrays | exact |
| HDF5 H2, shuffle + deflate 6 (256 MiB chunk cache) | 70,011,564 (0.191) | 40,193,926 / 13 | 458,788,466 | same | exact |
| Custom block container, layout B | 65,813,472 (0.180) | 944,504 / 10 | 166,108,504 | 29,365 bytes, no dependency | exact |

On the other fixtures the order is the same: for `spline-heavy` the 1/16 viewport costs 3,589,737 bytes in
H2 and 2,020,624 in layout B; for `straddler` 4,856,511 and 329,720.

## Decision

A custom block container: a 32-byte header, independent blocks, and a directory of one fixed-size entry
per block (offset, length, count, bounds). The layout inside a block is ADR 0004, the index over blocks is
ADR 0005. STEP-HDF and HDF5 are rejected as the v1 container and kept as a benchmark baseline in P8.2.
HDF5 is not offered as a v1 export target.

## Consequences

* PVec owns its reader, so its hostile-input handling is ours to fuzz (P7.8); the HDF5 library's was not
  ours to harden, and the pure-Rust reader mis-parses oldest-format compound types.
* The semantic profile and the numeric policy, not the container, are what interoperate with STEP
  (ADRs 0001 and 0002).

## Reversal threshold

Reverse to adopting HDF5 as the storage under a PVec-defined block directory and index only if both hold on
a rerun against corpus revision r2 or later: a Morton-ordered, chunk-per-block HDF5 file reads at most twice
layout B's bytes for the 1/16 viewport on `million` (at most 1.9 MB, against the 42.3 MB measured), and a
maintained pure-Rust reader decodes variable-length data in wasm at 100 MiB/s or better.
