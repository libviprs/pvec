# ADR 0010: The capability registry as data

Status: Accepted, 2026-10-16. Decides the shape of P3.4 (`crates/pvec-capabilities`, `docs/CAPABILITIES.md`).
Builds on ADR 0008 (extension rules).

## Context

ADR 0008 chose capability sets over a single version number, which makes the registry the thing every
reader, writer and codec agrees on. Three questions were left open: how an id is kept from ever being
reused, what a registry entry may claim about STEP and GIS before phases 2 and 6 have measured anything,
and where the list of required capabilities lives.

## Options

| question | options | evidence |
|---|---|---|
| keeping ids from being reused | A. a retired list that is checked against live ids. B. A plus a rule that every id from 1 to the highest is live or retired | with A alone, deleting an entry and forgetting to retire it leaves a hole that the next author fills. B turns the hole into a validation error (`UnaccountedId`), and checking it is a sort and a scan of the entries, not a walk over the id range |
| numbering | A. an arbitrary number per entry. B. geometry ids equal the section number in `ISO_PROFILE.md` | B makes the document and the registry cross-checkable by eye, and a test pins every id, so a renumbering is a visible breaking change |
| STEP and GIS columns | A. state what a round trip achieves. B. state what the profile declares | nothing in phase 3 has measured a round trip (phases 2 and 6 do), so A would be an unverified claim. B is true today and the generated document says which it is |
| where requirements live | A. the file declares them. B. the registry declares a default and the file may only strengthen it | under A a file could mark a circle optional and a reader would render a drawing with the arcs missing |

## Decision

B in every row. Geometry entries are required, components are optional by construction and a file cannot
declare one required, a minimum reader version above the reader's own refuses the file with both versions
named, and an unknown optional capability is skipped while an unknown required one refuses. GIS is
"exact" only where the simple-feature types hold the curve (point, polyline) and "densified" for the rest,
with the exception stated per entry (a trimmed line and a straight composite are exact). Registry
validation that needs the profile document (`validate_against_profile`) takes the parsed sections as
data, so the crate itself stays dependency free and does no I/O; `pvec-checks` feeds it the real file.

## Consequences

* `docs/CAPABILITIES.md` is generated and a test compares it byte for byte; `PVEC_BLESS_CAPABILITIES=1`
  regenerates it.
* The component entries and the reserved space for future DWG semantics arrive with P3.7, and extension
  entries with P3.5; both go through the same validation, and neither can take an id from the retired list.
* The negotiation here is the minimum the acceptance criteria name. Parse-time skipping and the
  skip-safety mechanics are P4.7, the compatibility matrix is P7.6.

## Reversal threshold

Relax the contiguity rule if capability ids ever need to be allocated from separate ranges by separate
authors (a second registry); until then one counter is simpler to audit. Move the GIS column to measured
values once P6.8's fidelity reports exist.
