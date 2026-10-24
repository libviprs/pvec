# ADR 0004: The internal data layout of a block

Status: Accepted, 2026-10-16. Decides P1.5 (`docs/spikes/01-5-layouts.md`).

## Context

The panel's sixth finding: hot simple geometry belongs in a structure-of-arrays, type-batched layout, not
one serialized object per entity, and the layout serves the semantic API, not the reverse.

## Options

Native x64 on MARS unless marked; `million`, blocks of 512 entities in Morton order. "touch" walks every
value; the baseline is FlatBuffers with one table per entity.

| option | file bytes | touch ns/entity (two passes) | wasm module / touch | external crates |
|---|---|---|---|---|
| B: packed binary, offset tables, record arena | 65,813,472 | 3.7 / 3.6 | 29,365 bytes / 5.2 ns | 0 |
| A: FlatBuffers type-batched SoA | 65,372,368 | 5.2 / 5.7 | 60,511 bytes / 9.7 ns | 2 |
| C: rkyv type-batched SoA | 64,958,672 | 4.7 / 3.2 | 34,855 bytes / 4.3 ns | 23 |
| baseline: FlatBuffers per entity | 91,215,200 | 72.1 / 71.7 | 65,596 bytes / 89.9 ns | 2 |

The structure-of-arrays advantage over the per-entity baseline is 12.6 to 22.4 times on `million` natively
(19.5 times for B) and 17 times in wasm; it shrinks to 1.2 to 1.4 times when the reader builds owned
objects. B pays 7.3 more bytes per spline than A and 35.9 more per composite (6 and 22 bytes of padding),
fixable. Every SoA summing loop compiled to a packed 64-bit add in x64 (wasm not inspected).

## Decision

Layout B: type-batched SoA sections for immediate geometry and an offset-table arena of self-describing
records for splines and composites, each curve's knots, weights and control points contiguous. B and C are
indistinguishable on speed; B wins on specification ownership (a page of spec, 0 crates against 23) and on
bundle size.

## Consequences

* The reader is ours: bounds-check every offset (P4.8), fuzz it (P7.8). The 400-mutation smoke test in the
  spike found no panic.
* Block size must be set in bytes, not entities: 512 entities is 33.6 KB on `million` and 288 KB on
  `spline-heavy`, and the P1.6 simulation shows the viewport cost on spline-heavy falling from
  2,020,624 bytes (512-entity blocks) to 689,526 (64-entity leaves of a different model, at a 64 KiB merge gap).

## Reversal threshold

Choose A if the browser reader decodes in TypeScript directly rather than in the Rust evaluator compiled to
wasm (A buys a language-neutral schema at 1.7 to 1.9 times the wasm cost), or if B's reader fails fuzzing in
P7.8 in a way bounds checks cannot close. Revisit B if P8.3's corpus shows its wasm touch more than 25%
behind the faster candidate (it is 12% to 21% behind C here).

## Amendment for the advanced geometry arenas (P4.3)

Layout B's offset-table arena is realised as one arena block per payload kind (knots, weights, control
points, composite segment lists, trims) with a table of `(start, len)` entries, and a fixed 32 byte record
per advanced curve that holds only degree, flags and table indices (`docs/CONTAINER.md` chapter 3). Two
decisions go beyond the layout measured above.

A composite curve is stored as a list of record indices, so a segment that several composites share is
stored once. That means composites can reference composites, up to the depth of NP-18 (a spline is depth
1), with cycles and deeper chains refused. `docs/ISO_PROFILE.md` section 9 says nested composites are
flattened by the producer; that stays true of the Part 21 form, and the container's allowance is
reconciled with it by making the Part 21 writer (P6.2) expand references into one flat segment list. A
composite on disk can therefore be nested while one in a STEP file never is.

The periodic flag exists in the record and is refused: the profile has no wrapped periodic form (ISO_PROFILE
section 12, ADR 0011), so there is no periodic rule to check counts against.
