# ADR 0015: The semantic class stream

Status: Accepted, 2026-10-23. Decides P3.9 (`crates/pvec-components`, `docs/COMPONENT_STREAMS.md`). Builds on
ADR 0010 (the registry) and ADR 0014 (component streams).

## Context

PVec carries geometry and independent component streams, but nothing says what a drawn thing is in the
building. A reader that wants only the walls, or a room, cannot ask. The registry rules (ADR 0010) say how a
new stream gets its id: one counter, no reuse, no gaps, optional by construction, no ISO construct. What was
open is how rooms and corridors are represented when they are not drawn, how the closed enum grows, and
where the checks that need geometry live.

## Options

| question | options | evidence |
|---|---|---|
| the capability id | A. take the next id, 20. B. reuse the reserved DWG semantics space (19) | 19 is opaque by design and a reader skips it as a whole, so classes inside it could not be skipped on their own, and ADR 0010 forbids reusing an id. A is one new registry entry, and the contiguity check (`UnaccountedId`) holds |
| a room or corridor | A. a closed curve duplicated into the stream. B. a record of `EntityId` references to existing entities, outer ring plus holes | A embeds curve parameters outside the geometry oracle, which the issue and ADR 0014 forbid. B is the hatch rule again: ring kind 1 is references and 2, the inline-curve word, is refused as embedded geometry |
| growing the enum | A. vendor values. B. closed per stream version, extended by a version bump | with A a reader cannot tell a value it should ignore from corruption. Under B an undefined value is refused and a newer major version is skipped whole by the frame |
| where closure is checked | A. in the stream crate. B. through a trait the caller implements | A needs a dependency on `pvec-geometry`, which ADR 0014 keeps out so that a stream cannot reach a curve. B (`EntityGeometry`, endpoints only) keeps the crate free of it, and the tests implement the trait over the real decoded geometry |
| confidence | A. on every entry. B. only on `inferred` entries, optional | a rule or explicit entry has nothing to be unsure about, so a confidence there is a lie by construction and is refused |

## Decision

B in every row, with A for the id. `semantic_class` is id 20, version 1.0.0, optional, with no ISO construct
and no STEP or GIS behaviour, registered like the other components and framed by ADR 0014's frame. Element
classes are `Other`, `Wall`, `Column`; regions are `Room`, `Corridor`; sources are `layer-rule`, `block-rule`,
`inferred`, `explicit`. The new limit this ADR introduces is at most 256 holes per region (the ring limit is
ADR 0014's 4096). Closure tolerance is a parameter of `validate`, because the crate owns no tolerance.

## Consequences

* Stripping the stream, or reading with no support for it, changes no geometry number. The tests show it
  with the same bit-for-bit comparison as the other streams. The P2.5 and P2.6 gates do not exist yet
  (P2.8), so the evaluator is the stand-in verdict, as it was for #83.
* Bumping the stream's version goes through `Registry::bump_component_version` and touches no geometry
  version.
* Deciding what is a wall or a room is not here (N4.9, N6.9), and neither are labels, furniture, doors or 3D.
* `entities_of` and `regions_of` are the filter P7.5's spatial query will use.

## Reversal threshold

Move the classes to a table (a name per class, an id per entity) if a third kind of element is needed
before a stream version bump is acceptable. Raise the hole limit if a real drawing has a room with more
than 256 columns in it.
