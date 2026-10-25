# ADR 0014: Component streams

Status: Accepted, 2026-10-17. Decides P3.7 (`crates/pvec-components`, `docs/COMPONENT_STREAMS.md`). Builds on
ADR 0008 (components are optional blocks keyed to `EntityId`) and ADR 0010 (the registry).

## Context

The panel's tenth finding keeps style, text and hatch outside the geometry oracle. ADR 0008 says components
are optional blocks that a reader skips when it does not know them. What was left open is how a stream is
framed so that skipping needs nothing from the payload, how independence from the geometry is guaranteed
and not only observed, and what a hatch may hold.

## Options

| question | options | evidence |
|---|---|---|
| framing | A. a length-prefixed payload per stream with its capability id and version in a fixed frame. B. self-describing payloads | with A a reader steps over an unknown stream by its declared length and never looks inside (a test fills the payload of an unknown stream with garbage and still opens the set). Under B the reader must parse to find the end |
| a stream from another major version | A. try to read it. B. skip it and report | the registry knows only one format per major version, so A would misread; B is a report, and the geometry opens |
| independence | A. observe it in tests only. B. also make it structural: the components crate depends on neither `pvec-geometry` nor `pvec-codec` | B means nothing in the crate can reach a curve; `pvec-checks` enforces the dependency allow-list. The tests still decode and evaluate the geometry with every stream, with each one removed in turn and with none, and the numbers are bit-identical (A alone would only show it for the cases tried) |
| what a hatch holds | A. boundary curves inline. B. references to existing entities plus fill parameters | A would put curve parameters outside the geometry oracle, which the issue forbids. The boundary-kind word is a tag: 1 means references, and 2 (the word an inline curve would use) is refused as embedded geometry, so the rule is checked and not only documented |
| a version bump | A. edit the registry data. B. `Registry::bump_component_version`, components only, upwards only | B makes the independence a property of the API: it cannot touch a geometry version |

## Decision

B in every row, with the frame of A. Six component entries (14 to 19) are registered optional, with no ISO
construct and no STEP or GIS behaviour; a frame that declares a geometry capability id is refused on write
and on read. An unknown, unimplemented or other-major stream is returned as a `Skipped` with its reason and
the set still opens. Every typed payload refers to entities by `EntityId`, is consumed exactly and checks
each count against the bytes left before it can size an allocation. The sixth stream, reserved space for
future DWG semantics, is opaque on purpose.

## Consequences

* A reader that implements none of the streams opens a fixture that carries all six, skips them all and still
  evaluates the linework; a test shows it.
* The numeric limits on names, text, dashes, boundaries and handles are the ones in
  `docs/COMPONENT_STREAMS.md`. They are new numbers this ADR introduces, small on purpose, and raising one is
  an ADR like any other limit.
* The meaning of the streams (text and fonts, hatch semantics, provenance content) is left to EPIC N and
  EPIC M; this crate fixes only the shape.

## Reversal threshold

Move to self-describing payloads if a stream appears whose records cannot be skipped by a length prefix. Move
the size limits if EPIC N's first real drawing exceeds one of them (the first to watch is the 255 byte layer
name).

## Amendment 2026-10-25: layer state travels in the layer stream (#119)

The layer stream stored a name table and one index per entity, so a drawing with every entity on an off layer
converted to an artifact that looked the same as one with everything shown. The panel added one flags byte per
layer-table entry: bit 0 off, bit 1 frozen, bit 2 locked, the other five bits zero and refused on read and on
write (COMP-12). The byte is state, not policy: PVec records what the layer table said and the consumer decides
what to draw. The layer stream moves from 1.0 to 2.0 through `Registry::bump_component_version`, because the
decoder consumes its payload exactly and a trailing byte is not tolerated. Nothing outside this workspace had
read layer 1.0, so the bump is a major one, and `LayerStream::decode_at` still reads a 1.0 payload as all-zero
flags. The geometry capability versions do not move. One panel member dissented on principle, that drawing
policy should stay out of the format; the closed bit layout is the answer to that.
