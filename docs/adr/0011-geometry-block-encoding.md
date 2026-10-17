# ADR 0011: Exact parameters in the geometry block

Status: Accepted, 2026-10-16. Decides how P3.2 and P3.3 write curves into the layout of ADR 0004
(`docs/GEOMETRY_ENCODING.md`, `crates/pvec-codec`).

## Context

ADR 0004 measured layout B on entities stored as their natural source numbers: a line as four
coordinates, an arc as centre, radius and two angles. The semantic model of ADR 0001 is different: a
segment is a trimmed curve over a line, with an origin, a unit direction and a normalised trim, and the
structural conformance class (P2.5) compares exactly those fields. The acceptance criteria ask for
bit-exact round trips, for kinds to be preserved, and for trims to survive "including inverted and periodic
cases, normalised per P1.2".

## Options

| question | options | evidence |
|---|---|---|
| what a segment stores | A. two end points, rebuilt as origin, normalised direction and trim `[0, length]`. B. the curve's own numbers: origin, stored unit direction, trim start, trim span, reversed | A cannot be bit exact: a unit direction recomputed from two end points need not equal the stored one in the last bit (a seeded search over 20,000 random directions finds stored unit vectors that normalising again moves, in `stored_axes_are_validated_not_renormalised_...`), and a segment whose trim does not start at 0 or runs backwards has no end-point form at all. B is exact for every valid segment |
| how a decoder builds the axis | A. `Placement2::new` / `Line::new` (normalise). B. `from_stored` (check unit length within NP-05, keep the bits) | the same search: normalising a unit vector again moves bits for some directions (the test only requires that it finds at least one, and I did not record how many), which would break the round trip. B follows ISO_PROFILE section 2, which already says a stored direction that is not unit length is rejected |
| the trim | A. rebuild with `Trimmed::new` from the end parameters. B. `Trimmed::from_normalised` from `(start, span, reversed)` | `new` re-normalises under NP-11 and `start + span` is not `t2` bit for bit; B keeps what was written |
| component references per entity (named in the issue) | A. a reserved `u32` column per kind. B. none | P3.7's streams refer to entities by `EntityId`, so the entity side needs no reference. A is 4 bytes per entity of dead weight |
| padding | A. ignored on read. B. must be zero | B keeps the encoding canonical: decode then encode reproduces the bytes, which a test asserts, and padding cannot carry data |

## Decision

B in every row. The cost is size: from the column widths in `docs/GEOMETRY_ENCODING.md`, an unbounded line
is 36 bytes of payload per entity (id plus four numbers), a segment is 53 (id, six numbers, a flag byte), a
circle 44, an arc 61, an ellipse 52 and an elliptical arc 69, against about 37 for the four-coordinate line
that ADR 0004 measured. That is arithmetic from the layout, not a measurement on a fixture. P8.3 owns the
comparison on the benchmark corpus, and a compact form for canonical segments can be added later as a new
section kind without touching the existing ones, because the kind directory is open ended.

## Consequences

* The reader rebuilds every curve through the validating constructors, so a malformed block cannot produce
  an unvalidated curve, and a refusal names the rule (`NP-nn` from the geometry crate, `ENC-nn` for the
  layout).
* A curve kind with no codec is refused at encode time. P3.3 adds the spline kinds; until then a spline is
  an error, never a polyline.
* The segment tag table of the composite record is shared with the standalone sections, so P3.3 extends one
  table.

## Reversal threshold

Add a compact segment section if P8.3 shows that segment payload is more than 25% of block bytes on a real
drawing and a canonical-form fraction above 90% of segments. Reconsider zero-padding as a requirement if
the reader cost of checking it shows up in P8.4's decode benchmark.

## Amendment for the spline codecs (P3.3)

The same rules carry over to Bezier, B-spline and rational B-spline records. A spline record is four
u32 (degree, control point count, knot count, flags) followed by the control points, the knots and, for a
rational curve, the weights, all contiguous so a curve's arrays are read as one run (the P1.5
measurement). A Bezier record states its degree and has no knots; its degree must be its point count minus
one. A rational curve is its own section, so a set of equal weights stays rational, which is what
`docs/ISO_PROFILE.md` section 12 says. Trimmed splines get a section of their own and composite segments get
tags 5 to 10 in the table they already shared. The profile has no wrapped periodic form, so a closed curve
written with an unclamped knot vector and wrapped control points is an ordinary B-spline here.

The counts are the only numbers that size an allocation, and every one is compared with the bytes left
before the arrays are read. The evaluator is not touched: the codec has none, and a repository check fails
if any crate but `pvec-geometry` defines one.
