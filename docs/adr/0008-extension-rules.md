# ADR 0008: Extension rules

Status: Accepted, 2026-10-16. Governance mechanics are P3.5; this ADR fixes the container-level rules the
phase 1 measurements support.

## Context

PVec has to evolve (new curve kinds, components for style, text and hatch, new codecs) without breaking old
readers, and a reader must reject what it cannot understand rather than guess. Style, text and hatch
semantics stay outside the geometry oracle and evolve as independent components.

## Options

Measured on layout B's container in the P1.5 spike (native x64, MARS, `million`):

| option | what it costs |
|---|---|
| A. Closed set, reject anything unknown | no cost, but every addition breaks every old reader |
| B. A kind id and a must-understand flag per block, required capabilities listed in the header; unknown optional blocks are skipped, unknown required ones reject the file | skipping an unknown block costs reading its fixed-size directory entry: 56 bytes (0.11 bytes per entity at 512 entities per block); cold open reads 109,456 bytes of header and directory for 1,954 blocks (0.17% of the 65.8 MB file) |
| C. Free-form extension namespaces with a version number each | the same skip cost, plus an unbounded registry for readers to reason about |

The hostile-input checks that make option B safe were measured: layout B's decoder survives 400 mutated
blocks without a panic, and the Tier 0 decoder rejects a lying count of `u32::MAX`, truncation at every 37th
byte, trailing bytes and unknown tags.

## Decision

Option B. A block's kind id and must-understand bit live in its directory entry; the header lists required
capabilities as a set, and a reader that lacks one rejects the whole file, never partially. Geometry
semantics are never extended by a vendor block: a new geometry kind needs an ADR and a section of
`ISO_PROFILE.md`. Components (style, text, hatch) are optional blocks keyed to `EntityId` and are skipped by
readers that do not know them. Codec ids follow the same rule (ADR 0006).

## Consequences

* The directory entry carries two more small fields (kind and flags), which fit in the padding of the 64-byte
  entry of ADR 0006.
* Version negotiation is capability-based, not a single version number, so P7.6's compatibility rules are
  about sets.
* An unknown required block makes the file unreadable by design; the tooling (P7.7) must report which
  capability was missing.

## Reversal threshold

Reconsider if P2.7's hostile tests find a skip path that can be made to read outside the file with a valid
directory entry, if the P8.3 corpus shows the directory exceeding 0.5% of file size (today 0.17% on the
million-entity fixture), or if more than three independent component kinds are needed before v1 and the flat
set of capabilities stops being reviewable.
