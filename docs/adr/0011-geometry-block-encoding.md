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

## Amendment for the periodic B-spline (issue 93)

The DWG mapping spike (libviprs PR 1251, `docs/spikes/dwg-mapping.md`, row M17) found that a periodic cubic,
`closed-periodic-001` of the `corpus-r0-open` corpus, is refused by `BSpline::new` under NP-14, so every
periodic DWG spline came out as `RequiresPVecCapability`. The probe has 6 unique control points and 7 knots,
and the spike asked two things: wrapped form or exact unwrap, and whether a vendor writer stores the same.

### What the 7 knots are

Seven knots for six points reads as a mistake until you read it as one period: `n + 1` knots delimit the `n`
spans of a periodic curve, with the first and the last naming the same knot one period apart (`0 .. 1`, period
1). That is the layout a periodic B-spline has when it is stored as its unique points, and it is the only
reading under which the probe is a well-formed curve. I could not check it against a vendor writer: the
corpus is acadrust-written and only cross-read by ACadSharp (`READER_MAPPING_ONLY`), and I found no
vendor-written periodic sample anywhere in the workspace. So what AutoCAD or ODA store stays an open fact, and
the decision below is built so that it does not have to be settled first (see "The layouts a producer can
hand over").

### Options

| question | options | evidence |
|---|---|---|
| how a periodic spline reaches PVec | A. exact unwrap in the producer (libviprs). B. an exact unwrap in `pvec-geometry`, a conversion that returns the clamped `BSpline` and a bound. C. a wrapped periodic representation in the profile, the codec and the container (an extension under ADR 0008). D. keep refusing and report the row as unsupported | see below |

* A is excluded by NG-B3 of the N1.2 ADR before the numbers come into it: knot insertion in libviprs is the
  second spline library the epic is trying not to grow, and the one evaluator rule (a repository check fails if
  a crate other than `pvec-geometry` defines one) is the same argument from this side.
* C (a wrapped form) costs a profile section, a Part 42 mapping, a codec section, the container flag bit
  (`ARN-16`, today refused), a minimum reader version and a conformance class, and it buys nothing a clamped
  curve does not give: ISO 10303-42 has no periodic attribute on `b_spline_curve_with_knots`, so the exact Part
  21 form of a periodic spline is already an ordinary B-spline, and the profile's own sentence ("a closed curve
  written with an unclamped knot vector and wrapped control points is an ordinary B-spline here") says so. The
  issue asks for C only "if the conversion changes the parameterisation in a way that matters to the conformance
  classes". It does not: the unwrap keeps the domain `[knots[0], knots[n]]` and the locus. What it changes is the
  knot multiplicity at the two ends, which the structural class compares as data and the geometric class (the
  locus) does not see.
* D keeps a drawing's normal closed outline out of PVec for no measured reason, which is the cost the issue
  already names.

B wins. The numbers below come from exact rational arithmetic, not from the f64 evaluator. The test tree has a
small big-integer module, and the reference curves (the clamped result, the wrapped curve formed in f64, and the
exact periodic curve with knots `k[j] + m T` computed with no rounding) are all evaluated by de Boor over the
rationals at the same parameters, so no evaluator rounding sits in any assertion and no slack is added to a
bound. The first review of this PR found that an earlier version of these tests added 200 roundoffs of slack,
more than the bound itself, so a halved bound stayed green. It was right, and this replaces that.

What is measured: the probe, a cubic with a seam of multiplicity 3 and an interior break, a seam repeated with an
inexact period, knots at 1e9 spaced just above the NP-14 limit, and 48 seeded random splines (degree 1 to 6, from
degree + 1 to degree + 8 unique points, interior knots of multiplicity up to 2, three quarters of them with
coordinates up to 1e12 and knot offsets up to 1e6), at 7 to 25 parameters each. "Survives h halvings" means the
exact deviation is still within the bound divided by 2^h.

| result | native x64 (MARS, rustc 1.99.1) | arm64 (Apple Silicon, docker) |
|---|---|---|
| probe, deviation from the f64-formed wrapped curve | within the bound, survives 6 halvings | same |
| probe, deviation from the exact periodic curve | within its bound, survives 10 halvings | same |
| random suite, closest approach to the first bound | 6 halvings (about 1/100) | same |
| random suite, closest approach to the second bound | 10 halvings | same |
| knots at 1e9, gap 1.2e-6 (just accepted), second bound | holds, survives 56 halvings (it is weak there, by construction) | same |

The first bound is not tight: the data gets within about a hundredth of it and no closer, so it cannot, by
measurement alone, tell a bound that is halved from the real one. That is why its formula is pinned in a test of
its own (`15 r u m` with `r = 2 (p + 1 - M)`, `M` the multiplicity of the seam knot), and why a test checks that
the data comes within 2^9 of it. The two machines agree on every number.

### Decision

`BSpline::from_periodic(degree, control_points, knots) -> Result<Unwrapped>` in `pvec-geometry`, where
`Unwrapped` is the clamped, closed `BSpline` and two bounds (below). The input is the decoder's report: the
degree, the `n` unique control points and the `n + 1` knots of one period. The curve it describes repeats the
points and shifts the knots by the period `T = knots[n] - knots[0]`, a span `[k[j], k[j + 1]]` is shaped by
`P[j] ..= P[j + degree]` (indices modulo `n`), and the domain is `[knots[0], knots[n]]`.

How: the wrapped, unclamped form (`n + degree` points, `n + 2 degree + 1` knots, a legal `BSpline` already) is
built and validated as such, then the two ends of the domain are inserted by Boehm's algorithm up to
multiplicity `degree + 1`, and the points and knots outside the domain are dropped. Knot insertion changes the
representation and not the curve, so the unwrap is exact in the mathematical sense, and what f64 does to it is
the two bounds below. Every knot of the result is a value of the input, never recomputed, and the seam knot is
carried bit for bit. In the numeric policy's terms the claim is: same locus and same domain as the wrapped curve, closed flag declared
and therefore checked under NP-20, every number through NP-01, NP-02, NP-14 and NP-18 like any other spline.

The bounds. There are two, because two different things are inexact, and the first version of this amendment
(and the rustdoc) ran them together.

* `deviation_bound` is the rounding of the insertions, against the f64-formed wrapped curve: the unclamped
  form that `BSpline::new` evaluates when the extended knots `k[j] +- T` are rounded to f64 as they are formed.
  Each insertion forms every new point as `(1 - a) P + a Q` with `a` in `[0, 1]`. The weight `a` carries at most
  three roundoffs, the two products and the sum three more, so one insertion moves a coordinate by at most
  `10 u m` (`u` is 2^-53 and `m` the largest absolute coordinate), and a convex combination does not amplify an
  earlier error. The Euclidean norm adds a factor of `sqrt 2`, hence `15 r u m` with `r = 2 (p + 1 - M)`
  insertions (at most `2 p`). Because the basis functions sum to one, a control point error of `d` moves the
  curve by at most `d`.
* `periodic_deviation_bound` is the distance to the exact periodic curve, the one with real-number knots. It is
  not the first bound: rounding the extended knots moves the curve, by an amount that grows as the gaps shrink.
  The reviewer found the first bound exceeded in about 0.2% of random cases against the exact curve, by up to
  8e4 times, and 1e-9 of the largest coordinate with gaps near the NP-14 limit, which is above NP-04. So the
  second bound is the first plus the knot term: every extended knot is off by at most `delta = 2 u (2 T + S)`
  (`S` the larger of `|k[0]|` and `|k[n]|`), a blending weight is off by at most `4 delta / (g - 2 delta)` (`g` the
  smallest gap between distinct knots), `p` weights are blended along an evaluation path, and each moves a point by
  at most the control hull diameter, below `3 m`. NP-14 refuses a gap under eight ulps of the knot magnitude,
  which is more than `4 delta`, so every input that is accepted has this bound (and the code refuses, as NP-14,
  an extension f64 cannot resolve, should that ever not hold). It is a worst case bound and it is weak when the
  knots are within a few thousand ulps of each other, as the table shows.

The bounds are about the unwrap, not about the evaluator, whose own rounding is separate. A consumer that wants
the distance to the mathematical curve uses the second; one that compares with what `BSpline::new` gives for
the wrapped layout uses the first.

Refused, with the rule named: a degree outside 1 to 16 or fewer than `degree + 1` points, or `n + degree` over the point limit, checked before any work (NP-18), a knot count
other than `n + 1`, a decreasing or ambiguous knot vector (including one f64 cannot keep apart after the shift, an unresolvable
extension), an empty period, or a seam knot of multiplicity
above the degree where the first and the last knot count once (NP-14), and any non-finite or out of range
number (NP-01, NP-02).

### The layouts a producer can hand over

| layout | knots | what libviprs does |
|---|---|---|
| unique points, one period of knots (the probe) | `n + 1` | `BSpline::from_periodic` |
| wrapped points and extended knots, unclamped | `m + p + 1` over `m = n + p` points, the first `p` points repeated at the end | `BSpline::new`, then `declare_closed`. Accepted today: profile section 11 defines the domain as `[knots[p], knots[n]]` for the unclamped form, and `pvec-codec` already round trips one |
| already clamped, closed by its end points | `m + p + 1` over `m` points | `BSpline::new`, then `declare_closed` if the source says closed |
| unique points with `n + p + 1` (for example uniform) knots and a periodic flag | `m + p + 1` over `m = n` points | not an unwrap input. The count is that of an open B-spline and `BSpline::new` takes it as one. The flag cannot be read from the counts, so the producer must not guess: if the source says closed, `declare_closed` refuses it under NP-20 when the ends do not meet (tested), and otherwise the curve is open. A producer that sees the flag with these counts quarantines the entity rather than calling `from_periodic` |

So whichever of the first three a vendor writer stores, libviprs hands over what the decoder says and gets a
curve or a typed refusal. It does no arithmetic of its own, which is what NG-B1, NG-B3 and NG-B5 ask. The fourth
is the layout the counts cannot separate from an open curve, and the table says so rather than claiming
every layout is caught. Telling the first layout from the others needs only the counts: it has one knot more than
points, the others have `degree + 1` more, and those differ for every degree of at least 1.

### What the unwrap throws away

The result has `n + p + 1 - M` control points (`M` the multiplicity of the seam knot) and multiplicity `p + 1` at both
ends of the domain, so to anything that walks the knots it looks like an open curve whose ends coincide, not a
closed smooth one. Two things are lost: the smooth closure (the periodic curve is `C^(p - M)` across the seam,
and the stored curve says nothing about that, only the declared closed flag and the geometry do) and the control
polygon the author edited (the `n` unique points become the clamped polygon). Neither matters to the locus, and
neither is a reason to store a wrapped form: the Part 21 exact form of this entity
(`B_SPLINE_CURVE_WITH_KNOTS('',degree,(...),.UNSPECIFIED.,.F.,.U.,(multiplicities),(knots),.UNSPECIFIED.)`,
ISO_PROFILE section 11) has `closed_curve` and `self_intersect` as its only flags and no periodic one, so a STEP
file could not carry the information either. A consumer that must hand a periodic spline back to a CAD system
as an editable periodic curve has lost what it needs, and that is the case the reversal threshold below names.

### Why `n >= p + 1`

The unwrap extends the knots by one period on each side, and `p` extra knots on a side need `p <= n`. For `n <= p`
the wrapped form is still a legal curve but a basis function then sees the same control point twice, which is
not a periodic B-spline in the usual sense and not something I found any writer produce. Supporting it means
extending by several periods, with no evidence that it is wanted, so it is refused as NP-18 (`Limit`). Relaxing
it later is additive.

### Consequences

* The container stays as it is: the periodic bit and `ARN-16` are unchanged, because the writer never needs
  them. A periodic spline is unwrapped before it is stored, so no stored curve is periodic.
* The capability registry keeps its ids and versions. Only the restriction text of `b_spline_curve` changes (a
  periodic spline is unwrapped to it, never stored wrapped), and `docs/CAPABILITIES.md` is regenerated.
* libviprs's row M17 becomes an `exact` row for the layouts above, with the domain of the source. It is not an
  `Unsupported` any more.
* Rational periodic splines (a periodic NURBS) are not covered. The same insertion works on the homogeneous
  coordinates, but the bound needs its own derivation (the division by the weight amplifies the error by the
  weight ratio, which NP-15 limits to `1e8`), and the spike has no such fixture. They stay refused: `from_periodic` has no weights
  parameter and `RationalBSpline::new` fails the knot count. A caller that drops the weights before calling
  would get a wrong curve without an error, so a producer must route a rational periodic spline to the
  unsupported path itself.
* The decision does not depend on what a vendor writer stores. If a vendor sample shows a layout that is none
  of the three above, a fourth row joins the table and, if the arithmetic is not a convex combination of the
  input, the bound is derived again.

### Reversal threshold

Move to a wrapped representation (option C, as an extension under ADR 0008) if a consumer has to hand a periodic
spline back as an editable periodic curve (the smooth closure and the control polygon are gone after the unwrap),
if the first vendor-written periodic sample shows a layout the unwrap cannot take without changing the parameterisation, or if the conformance
classes grow a check that compares knot multiplicities at the ends of a closed curve. Revisit the bound if a
fixture's measured deviation exceeds 1e-12 of the largest coordinate, which is 1e-2 of the NP-04 tolerance,
or a degree above 6 turns up in a real drawing (the random suite stops there).
