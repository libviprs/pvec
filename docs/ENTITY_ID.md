# Entity identity

`EntityId` is the identity of an entity inside one `.pvec` artifact. It is a four byte index, nothing
else, and this page is the rule for what it does and does not promise. The implementation is
`crates/pvec-entity`.

## What an id is

* A fixed-width unsigned integer below the entity limit of NP-18 (2^28). Four bytes is enough for that
  limit and is what the id arrays of the packed layout (ADR 0004) already spend per entity. A size assertion in the crate
  fails the build if anything wider (a string, a handle, a pointer) is ever put in it.
* dense across the artifact: the ids are exactly `0..n` for `n` entities, so the maximum id plus one
  is the entity count and a gap is a validation error, not a quirk.
* Assigned in the writer's canonical order. The id of an entity is its position in that order.

## Stability

An id is stable within a given artifact: every block, index entry, component stream and picking
result in that artifact means the same entity by the same id, for the life of the file.

An id is not stable across regenerations from a changed source. The same input and corpus revision
gives identical ids, because the generation (P2.4) is deterministic, but reordering the input changes the
ids, and so does adding or removing an entity before the point of change. I am not promising cross
artifact identity of any kind, and nothing should be built that assumes it.

## Preview and exact

A preview and the exact geometry that replaces it carry the same `EntityId`. A viewer that picked an
entity from a preview and then received its exact form keeps pointing at the same thing, which is the
replacement identity that P1.7 requires (ADR 0007).

## What does not go in the id

Source handles, the original DWG identity, layer names and provenance live in the provenance component
stream (P3.7). They are never encoded inside an `EntityId`, because they have a different lifetime:
a handle can survive a regeneration that renumbers every id.

## Resolving an id

An id resolves to the entity's kind, bounds and representation through the `EntityTable`, which
keeps those in arrays of their own. Resolving to bounds therefore does not decode the geometry payload.
