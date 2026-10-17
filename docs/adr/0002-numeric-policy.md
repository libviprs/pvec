# ADR 0002: The numeric robustness contract

Status: Accepted, 2026-10-16. Decides P1.2 (`docs/NUMERIC_POLICY.md`, `crates/pvec-numeric`). This ADR is
accepted before the container ADR (0003), as the phase gate requires.

## Context

An epsilon chosen per repository makes cross-repository comparison meaningless: `pvec`, `libviprs`,
`maplibre-pvec` and `pvec-gold-fixtures` would each be right by their own arithmetic. The panel required
numerical robustness to be a specification concern with its pathological tests written first.

## Options

| option | what it means | evidence (native x64, MARS) |
|---|---|---|
| A. Each repository picks an epsilon | the status quo for most geometry libraries | no cross-repository comparison is possible; rejected without measurement |
| B. One relative tolerance against the drawing or domain size | `|a - b| <= REL * extent` | P1.3 tried the domain-width form for trims of an unbounded line: with a width of 2^41 every trim shorter than about 220 units was refused as collapsed, including the first sample (length 5) |
| C. A triple: absolute floor, relative to the feature scale, ULPs of the magnitude, with the scale term named | `tol = max(ABS, REL * scale, ULP * ulp(mag))` | worst measured evaluation error is 2.7e-15 (about 12 roundoffs) against `REL` 1e-10, five orders of headroom; widening any one constant ten-fold turns at least one of 40 policy cases red (`abs`: 3 cases, `rel`: 8, `ulp`: 2, `weight_floor`: 1, `axis_ratio_max`: 1); narrowing also does |

## Decision

Option C with the values and the twenty rules of `docs/NUMERIC_POLICY.md`, and one amendment found by the
P1.3 prototype: an unbounded basis (a line) is judged by its own length, not by a domain width. Counts and
dimensions are numbers (2^28 entities, degree 16, 65,536 control points, 4 nesting levels). `pvec-numeric`
is the only place a tolerance literal may appear, enforced by a repository test, and no other repository
defines an epsilon.

## Consequences

* The pathological evaluator probes (15) are red until P3.1 and pass against the P1.3 prototype.
* Every later number-bearing document cites a rule id; P4.8's limits cite the count table and do not
  restate it.
* The constants are deliberately loose. They could be tightened 10 to 100 times without turning a measured
  case red; that decision is held for the P2.6 oracle comparison.

## Reversal threshold

Amend the constants if the dual-oracle comparison (P2.6) shows a known-truth fixture whose evaluation error
exceeds 1% of `REL` (1e-12; the measured maximum is 2.7e-15), or if two independent converters disagree on
a fixture by more than `REL` after the documented parameter normalisation. Raise a maximum count only by a
new ADR; lowering one is a breaking change.
