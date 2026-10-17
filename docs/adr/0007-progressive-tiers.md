# ADR 0007: Progressive tiers

Status: Accepted, 2026-10-16. Decides P1.7 (`docs/spikes/01-7-tier0.md`).

## Context

A first paint that silently omits deferred entities is unusable for reading a drawing. The panel's fifth
finding: Tier 0 holds all immediate exact geometry plus a bounded-error preview for every deferred entity
where feasible. Previews are display fidelity only and are never labelled exact.

## Options

Native x64 on MARS, fixtures at `pvec-p1-fixtures-r1`, error budget 0.5 px at the fit zoom, deviation
measured as two-sided Hausdorff distance against the P1.3 evaluator after `f32` quantisation.

| option | evidence |
|---|---|
| A. No tiers: fetch exact geometry as the index says | a first paint of `million` is the whole viewport's exact payload; the 1/4 viewport costs 5.7 to 6.4 MB (P1.5/P1.6) |
| B. Global Tier 0: every exact cheap entity plus a preview per deferred entity | complete coverage with 0 entities in neither class on all four fixtures; but the stream is 58,636,991 bytes on `million`, 90.51% of the packed artifact (99.61% on small, 92.20% on straddler, 37.03% on spline-heavy) |
| C. B with a collapse policy (point stubs for sub-pixel entities) | saves only 3.8% on `million` (58.64 to 56.43 MB): per-entity identity puts a floor of about 32 bytes under each entity |
| D. Tier 0 per spatial unit, driven by the viewport and the index leaf | not measured; follows from B and C |

Preview forms were chosen by measured deviation: sagitta polyline for ellipse arcs (met the budget for 100%
of ellipses on all four fixtures, worst 6.273 units against a budget of 26.04 on `million`), control polygon
for splines and composites where measured within budget (qualifies for 28,354 of 29,201 splines on
spline-heavy; worst 9.148 against 5.21, so it is not a bound by construction), sagitta polyline otherwise.
A writer generates a million previews in 176.6 ms and a reader decodes the whole Tier 0 stream in 6.96 ms.

## Decision

The tier definition of B, the replacement identity of P1.7 (a preview carries the `EntityId` of the exact
entity that supersedes it, with no separate id), and a per-unit delivery as in D: Tier 0 is built and
delivered per index leaf (ADR 0005), not as one global first pass, because a global Tier 0 is nearly the
whole artifact. A writer must measure every preview's deviation against the exact curve and may not assume
a form qualifies. Previews are display fidelity everywhere in the specification.

## Consequences

* The 90% finding moves the progressive mechanism into the spatial index (P5.1, P5.4), not into a smaller
  stub; aggregation of sub-pixel entities into shared stubs is the unexplored lever.
* Rational splines have no provable preview bound; their previews are verified after the fact, so a narrow
  excursion between samples could be missed.
* Full verification at write time was expensive in the spike (684 s on spline-heavy); a cheap, provable
  verification is a P5.4 task.

## Reversal threshold

Revisit if P8.5 shows that per-leaf Tier 0 does not reach complete coverage of a viewport in less than a
fifth of the time of fetching the viewport's exact payload, or if any fixture in the real-drawing corpus has
entities in neither class (today: 0 of 1,151,000), or if the 0.5 px budget changes.
