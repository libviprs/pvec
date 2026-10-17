# ADR 0001: The PVec 2D semantic profile

Status: Accepted, 2026-10-16. Decides P1.1 (`docs/ISO_PROFILE.md`). Evidence: P1.3, `docs/spikes/01-3-geometry-api.md`.

## Context

Nothing stated which ISO 10303-42:2025 constructs PVec implements, so every claim of exactness was
unfalsifiable and the three downstream repositories would each have invented their own mapping. The
panel's first finding was that PVec must not claim full ISO conformance and must instead define a
documented 2D profile aligned to named constructs and validated through STEP interchange.

## Options

| option | what it means | evidence |
|---|---|---|
| A. Claim ISO 10303-42 conformance | implement the schema, or say we do | 7 classes of Part 42 constructs are outside any 2D drawing source (surfaces, topology, 3D-only, swept geometry, curve-on-surface, derived curves, other conics); 0 of the 1,151,000 entities in the four fixtures need one |
| B. A profile aligned to named constructs, validated through STEP interchange | 13 capabilities, each with a seven-field table, a concrete Part 21 form and a list of what a decoder rejects | the P1.3 prototype implements all nine curve kinds that the 13 entries reduce to, evaluates and transforms them exactly (worst deviation 7.3e-16 relative to extent, 17 tests), and covers every entity of every fixture (native x64, MARS) |
| C. One mathematical type per source entity class | a type for ARC, one for CIRCLE, one for each polyline bulge, and so on | the fixtures' seven entity kinds map onto eight of the nine profile kinds (all but Bezier) without loss; a circle under an anisotropic map becomes an ellipse (P1.3), which a per-class design would have to special-case in every class |

The Part 21 forms of option B are also the baseline every size ratio in phase 1 is measured against
(365,814,152 bytes for the million-entity fixture).

## Decision

Option B. `docs/ISO_PROFILE.md` is normative. The only claim PVec makes is the single sentence in its
Claims section; a repository check fails the gate if any file says more. A distinct source entity class
is not a reason for a distinct mathematical type, and source mapping belongs to EPIC N.

## Consequences

* P1.3's prototype corrected one row of the profile (parameter maps under affine transforms), which is the
  evidence that the profile has to be validated by running code, not only by reading it. Amending a table
  is an ADR amendment from now on.
* Decoders reject by the lists in the profile, so hostile-input tests (P2.7) can be derived from it.
* The profile was written without the licensed text of Part 42; its construct names are unreviewed against
  the standard. A reviewer with the text checks each table before v1 (P8.7 gate).

## Reversal threshold

Reopen the profile if either holds: a licensed review finds a named construct whose definition differs
from the PVec restriction, or a STEP round trip in P2.5/P6.3 changes the locus of any fixture entity by more
than the NP-04 tolerance. Today the measured round-trip change is within tolerance: 2,099 of 1,000,000
entities come back with changed last bits and the worst deviation is 1.1e-13 (P1.4, native x64).
