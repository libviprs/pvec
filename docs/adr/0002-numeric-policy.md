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

## Amendment 2026-10-23: the canonical form of a full-period trim (#94)

The DWG mapping spike (libviprs#1042, PR libviprs#1251) found that a whole DWG ellipse, the pair 0 and 2 pi,
reaches PVec as a `trimmed_curve` over an `ellipse` with `span` equal to 2 pi, because NP-11 rule 3 keeps a
full period as a trim. The locus and the domain are right, the kind is not `ellipse`. The reader adapter may
not compare the span to 2 pi itself (N1.2 ADR, NG-10), so the question is what PVec says a full-period trim
is. NP-11 never said, which makes it a design decision and not a bug, so I am deciding it here.

| option | what it means | evidence (native x64 on MARS, arm64 in a local container, the same 12 tests) |
|---|---|---|
| A. Leave it | a whole ellipse is a trimmed curve with a full-period span, and the reader calls them equivalent | works today, but every whole ellipse reads as a different kind from its source and from `circle`, and the alternative to it is a second copy of NP-11 in each reader |
| B. A predicate only | `Trimmed::is_full_period()`, the caller rebuilds the untrimmed curve itself | the decision stays PVec's, but each caller repeats the rebuild and has to remember that the start and the direction are lost by it |
| C. Canonicalise every full period | `trimmed_conic` returns the untrimmed curve whenever rule 3 holds | with a start of 1.0 the trimmed curve at `tau = 0` is the point at parameter 1, the untrimmed one is the point at parameter 0, so `eval` returns a different point. A backwards full period would lose its direction. NP-12 says a trim is never silently swapped or dropped, and this drops two things |
| D. Canonicalise the identity parameterisation only | `trimmed_conic` returns the untrimmed curve when the trim is a full period, forward, starting at the origin of the period (NP-11 rule 6); every other trim is the `Trimmed` that `Trimmed::new` builds | for the 0 to 2 pi case, `eval` at 33 samples of `tau` and `bounds` are bit-identical between the full trim and the untrimmed circle and ellipse; a start of 1.0 or pi and a reversed full period stay trimmed, and the same tests pin the refusals NP-11 and NP-12 already make |

Decision: D, plus the predicate of B (`Trimmed::is_full_period`) so a consumer that already has a `Trimmed`
can ask without rebuilding.

Why D, from the numeric policy. The tolerance is NP-11's own: `tp = tol(2 pi, max(|t1|, |t2|))`, which for
trims near 2 pi is `REL * 2 pi`, about 6.3e-10 (the `REL` term wins over `ABS` and over the ULP term of
about 7e-15). The end is a full period when `|raw - 2 pi| <= tp`, already NP-11 rule 3, and the start is the
origin when it is within the same `tp` of 0 from either side, so a start written as `-1e-10` counts and a
start of `2e-9` does not. I did not invent a second tolerance, and no literal was added outside
`policy.rs`. What the rule keeps is the thing NP-12 protects: nothing about where the curve starts or which
way it runs is thrown away to make the kind look tidier. C is rejected for exactly that reason, A and B
because they leave the numeric rule spread over the callers, which is what #94 asked to avoid.

What the other two cases of the issue become. Equal start and end is an empty arc, so it is refused as
collapsed under rule 2 (it is not a full period, and treating it as one would turn an empty entity into a
whole one). A full turn written backwards is still ambiguous under rule 4. A circle written as a full-period
trim behaves like an ellipse.

What does not change. `Trimmed::new` and `Trimmed::from_normalised` still store a full period as a trim, so
the block codec round trip of ADR 0011 never changes a kind, and the canonicalisation lives only in the one
constructor a producer chooses to call. The constants and rules 1 to 5 are untouched, so the P2.6 reversal
threshold above is not affected.

Reversal: if a real source turns out to write whole conics starting at a non-zero parameter and expects the
untrimmed kind (more than 1% of whole conics in a corpus of at least 100 drawings), reconsider C and
write the start and the direction loss into the conversion report.

