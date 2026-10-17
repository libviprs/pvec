# ADR 0005: The spatial index and payload ownership

Status: Accepted, 2026-10-16. Decides P1.6 (`docs/spikes/01-6-spatial-index.md`).

## Context

Ancestor ownership alone congests the root and hot chunks, and a zero-duplication rule optimises the wrong
quantity. The panel asked for the payload to be stored once, bounded duplication of lightweight spatial
references to be permitted, and index layouts to be judged on total viewport I/O in source space.

## Options

A simulation over the shared fixtures (a byte model calibrated to layout B, within 5% per kind), native
x64 on MARS, the committed viewport script. "Interactive" is the script without its two whole-drawing
steps; bytes are at a 64 KiB merge gap. Every query is checked against brute force.

| model | `million` index bytes | duplication | `million` interactive MB / ranges | `stress-long` interactive MB |
|---|---|---|---|---|
| M1 loose quadtree (ancestor ownership) | 23,640,000 | 1.00 | 46.45 / 277 | 54.80 |
| M2 packed Hilbert R-tree, leaf 64, fanout 32 | 516,256 | 1.00 | **29.08** / 95 | **15.89** |
| M2 packed Hilbert R-tree, leaf 512, fanout 64 | 63,616 | 1.00 | 31.12 / 80 | 20.02 |
| M3 cells with duplicated 12-byte references | 39,221,080 | 2.00 | 48.80 / 280 | 56.06 |
| M3 with 28-byte references carrying bounds | 71,156,984 | 2.00 | 59.61 / 298 | 21.35 |
| M4 leaf cells plus long-entity buckets, leaf level 8 | 1,091,792 | 1.00 | 30.58 / 154 | 16.68 |

`stress-long` is `straddler` plus 2,000 expensive full-width polylines generated inside the spike (not
corpus). It reproduces the congestion: a 1/16 viewport showing 593 entities reads 6.70 MB under the
quadtree and 0.80 MB under the R-tree. No candidate is rejected for duplication: M3 loses because its
reference bytes and counts are read in full by the whole-drawing views and cost many ranges, not because
a payload is duplicated (it is not).

## Decision

M2: a packed R-tree over a Hilbert ordering of entity centres, every payload owned by exactly one leaf,
small leaves. The leaf is sized in bytes in implementation (P5.1 tunes it; I measured only entity counts).
Keep M4's long-entity bucket as a safety valve for drawings with many expensive long entities. The Morton
blocks and flat directory of P1.5's spike are the degenerate case of this tree.

## Consequences

* The block directory of ADR 0003 becomes the leaf level of a tree with a fanout of 32 to 64; on
  `million` the whole index is 0.8% of the payload at leaf 64.
* A long entity widens its leaf's bounds (0.958 of the world on `straddler`), costing a few KB per query
  with cheap kinds and more with expensive ones; the bucket valve is the answer.
* Visibility here is bounds overlap. A diagonal line can be fetched without crossing the viewport.

## Reversal threshold

Switch to M4 if, on the P8.3 corpus, M4's interactive bytes come within 5% of M2's while M4 needs fewer
ranges, or if more than 1% of leaves in a real drawing have bounds wider than 25% of the world side (on
`stress-long` the widest leaf is 0.958 and M2 still wins). Switch to M3 only if a 4-byte reference plus an
entity-offset table (not built) brings its interactive bytes within 5% of M2's with under twice the ranges.
