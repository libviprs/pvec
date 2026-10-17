# PVec numeric policy

**This document is the only definition of a numeric tolerance for `libviprs/pvec`,
`libviprs/libviprs`, `libviprs/maplibre-pvec` and `libviprs/pvec-gold-fixtures`. No other
repository defines an epsilon; they cite this one and reuse `pvec-numeric`.**

Citation form, used by the other three repositories: `PVec numeric policy v1, NP-nn` (for example
"PVec numeric policy v1, NP-05"), with a link to this file at a tagged revision. A rule that is
not in this file does not exist, and a tolerance that is not derived from the table in NP-04 is a
bug.

I wrote every rule as a predicate you can evaluate by hand: inputs, a formula, a decision. The
reference implementation is `crates/pvec-numeric` (rules are in `src/predicates.rs`, the numbers
are in `src/policy.rs`), and `tests/pathological/` exercises each rule at its edges. When the
text and the code disagree, the text wins and the code is the bug.

Status: the constants below are my starting values for phase 1. They are checked against measured
evaluation error in `docs/spikes/01-3-geometry-api.md`, and phase 2's oracle comparison (P2.6) can
move them through the ADR process only, never in passing.

## The constants

| Name | Value | Used by |
|---|---|---|
| `ABS` | `1e-12` (coordinate units) | NP-04, NP-06, NP-10 |
| `REL` | `1e-10` | NP-04, NP-19 |
| `ULP` | `8` | NP-04, NP-06 |
| `WEIGHT_FLOOR` | `1e-8` | NP-15 |
| `AXIS_RATIO_MAX` | `1e6` | NP-07 |
| `MAX_ABS_COORD` | `2^40` = 1099511627776 | NP-02 |

Why those values. `f64` has a unit roundoff of about `1.1e-16`. `REL = 1e-10` is about nine hundred
thousand roundoffs, which is deliberately loose enough that a curve written to Part 21 with 17
significant digits, read back, transformed and compared still falls inside it, and deliberately
tight enough that a feature one part in ten billion of the drawing is still resolved. `ABS` is only
a floor for values that are exactly or almost zero, where a relative term has nothing to be
relative to. `ULP = 8` accounts for the rounding of a few chained operations. `MAX_ABS_COORD = 2^40`
keeps one ULP at the bound at `2^-12`, which is about `0.00024` of a unit, so a drawing in
nanometres of a one kilometre extent still has sub-unit resolution.

## Rules

### NP-01 Finite and normal values

Every coordinate, parameter, radius, weight and knot is an `f64` that is finite and either zero or
at least `f64::MIN_POSITIVE` in magnitude. NaN, positive and negative infinity, and subnormal
values are refused. This is enforced twice: at parse time, before a value is stored, and at
construction of any in-memory geometry value. Subnormals are refused because some targets flush
them to zero and some do not, which would make the same file mean two things.

### NP-02 Coordinate magnitude

`|x| <= MAX_ABS_COORD` for every number covered by NP-01. A larger magnitude is refused as out of
range. Intermediate results may exceed it; stored and exchanged values may not.

### NP-03 Negative zero

`-0.0` is stored and compared as `+0.0`. Construction normalises it, so files are deterministic.

### NP-04 The tolerance triple

For two values of magnitude at most `mag`, belonging to a feature of size `scale` (the **scale
term**), the tolerance is

```text
tol(scale, mag) = max( ABS,  REL * |scale|,  ULP * ulp(mag) )
```

where `ulp(x)` is the gap between `|x|` and the next larger `f64`. The three terms mean: `ABS` is a
floor in coordinate units, `REL * scale` is the size-relative term, and `ULP * ulp(mag)` is the noise
floor of the arithmetic at the magnitude of the numbers involved. The scale term is **the size of
the feature being compared**, not the size of the drawing and not the magnitude of its coordinates:
for a point on a segment it is the segment length, for an arc it is the radius, for a parameter it is
the width of the parameter domain. Magnitude enters only through the ULP term. That split is what
keeps a one-unit feature at coordinate `2^39` meaningful (its tolerance is a few thousandths of a
unit, not a tenth) without making it look equal to its neighbour.

### NP-05 Equality

`near(a, b, scale)` is true when both are finite and either `a == b` or
`|a - b| <= tol(scale, max(|a|, |b|))`. Points are `near` when both coordinates are. `near` is not
transitive and is never used to merge, sort or hash values; it is used to decide whether two things
that are supposed to be equal are.

### NP-06 Degenerate length

A length `len` between points of magnitude at most `mag` is degenerate when it is not finite or

```text
len <= max( ABS, ULP * ulp(mag) )
```

There is no relative term: a length has no larger feature to be relative to. Zero-length segments
are refused by readers and are removed by producers, who record the removal as a fidelity loss.

### NP-07 Zero radius and conditioning

A radius or semi-axis `r` at a centre of magnitude `c` is refused when it is not finite, not
positive, or degenerate under NP-06 with magnitude `max(c, r)`. An ellipse is also refused when
`max(a, b) / min(a, b) > AXIS_RATIO_MAX`, because past that ratio the minor axis is below the
rounding noise of evaluating the major one.

### NP-08 Coincident control points and degenerate hulls

Coincident consecutive control points are legal (a sharp corner is made that way). A control hull
is classified from its points `p_0 .. p_n`: take `far`, the point farthest from `p_0`. If
`dist(p_0, far)` is degenerate under NP-06 the hull is a **point** and the curve is refused. Else if
every `p_i` has `orient(p_0, far, p_i) = collinear` under NP-09 the hull is **linear**, which is
legal and evaluates as an ordinary curve on a line. Otherwise it is **planar**.

### NP-09 Orientation

For points `a`, `b`, `c` let `det = (a_x - c_x)(b_y - c_y) - (a_y - c_y)(b_x - c_x)`, `L` the longer
of `|ab|` and `|ac|`, and `u = 2^-53`. The band is

```text
band = max( (3 + 16u) * u * (|left| + |right|),  tol(L, mag) * L )
```

with `left` and `right` the two products of `det`. The result is `collinear` when `|det| <= band`,
`ccw` when `det > band`, and `cw` when `det < -band`. The first term is Shewchuk's static error
bound for the floating point determinant, so a decision outside the band is the sign of the exact
determinant. The second turns the distance tolerance of NP-04 into a determinant bound.

### NP-10 Direction

A direction `(dx, dy)` is zero when `max(|dx|, |dy|) <= ABS`. Otherwise it is normalised by dividing
by its largest component and then by its length, so squaring cannot overflow or underflow. A stored
direction must satisfy `near(|d|, 1, 1)`.

### NP-11 Periodic parameter normalisation

For a periodic basis of period `P` (`2 pi` for circle and ellipse) and trims `t1`, `t2` with sense
`s`, let `raw = t2 - t1` if `s`, else `t1 - t2`, and `tp = tol(P, max(|t1|, |t2|))`. Then:

1. `|raw| > P + tp`: refused (more than one period).
2. `|raw| <= tp`: refused as collapsed (NP-12).
3. `|raw - P| <= tp`: the full period, `span = P`.
4. `|raw + P| <= tp`: refused as ambiguous (a full turn written backwards cannot be told from an
   empty one).
5. Otherwise `span = raw mod P` (Euclidean, in `[0, P)`).

The normalised start is `t1 mod P` if `s`, else `t2 mod P`, taken into `[0, P)`, and the arc is
flagged reversed when `s` is false.

### NP-12 Trim domains

A trim is collapsed when its span is at most `tp`. For a basis with a bounded domain `[lo, hi]` the
width must exceed `tol(hi - lo, max(|lo|, |hi|))`, both trims must lie in `[lo - tp, hi + tp]`
(and are clamped into it), `t1 < t2` must agree with the sense (`t1 < t2` if and only if sense is
true), and a trim is refused otherwise. An inverted trim and a zero-length trim are therefore never
silently swapped or dropped.

### NP-13 Parameter domain and unit map

A B-spline evaluates over `[knots[degree], knots[n]]` (zero based, `n` the last control point), in
its native parameter. To compare parameters across curves, `unit(t) = (t - lo) / (hi - lo)`, which
requires `hi - lo > tol(hi - lo, max(|lo|, |hi|))`; a narrower domain is refused as empty. Polyline
and composite parameters are the PVec definitions of `ISO_PROFILE.md`, mapped the same way.

### NP-14 Knots

For degree `p` and `n+1` control points the knot vector has `n + p + 2` entries, all passing NP-01,
non-decreasing. With `W = hi - lo` and `tp = tol(W, mag)`, two adjacent knots that differ at all
must differ by more than `tp`; a closer pair is refused as ambiguous, because it is neither one knot
nor two. The multiplicity of any value is at most `p + 1`, and of a value strictly inside `(lo, hi)`
at most `p`. A multiplicity of exactly `p` is a **full multiplicity** interior knot: a legal break
where the curve is only `C^0`. A multiplicity of `p + 1` inside the domain would make the curve
discontinuous and is refused; the producer splits the curve into a composite instead.

### NP-15 Weights

Every weight passes NP-01, is strictly positive, and is at least `WEIGHT_FLOOR` times the largest
weight. A weight below that is refused, because the homogeneous division amplifies the error of the
control point it belongs to without bound as the weight approaches zero.

### NP-16 Continuity and one-sided derivatives

A B-spline is `C^(p - m)` at a knot of multiplicity `m`. The two-sided derivative of order `k`
exists at that knot if and only if `k <= p - m`. Derivatives of any order are always available as
**one-sided limits** from inside the domain. Comparisons at a break (a full multiplicity knot, a
composite joint whose continuity is only position) use the left limit and the right limit
separately and never a two-sided value. A request for a two-sided derivative where `k > p - m` is
an error, not a silently chosen side.

### NP-17 Composite joints

The end of segment `k` and the start of segment `k + 1` are joined when `near_point(end, start,
scale)` with `scale` the longer of the two segments. A greater distance is a gap and the composite
is refused. A declared continuity of `.CONT_SAME_GRADIENT.` or better is a claim that conformance
tests verify with one-sided derivatives; the decoder checks position only.

### NP-18 Maximum counts and dimensions

| Quantity | Maximum |
|---|---|
| Entities in one file | 268435456 (`2^28`) |
| Degree of a B-spline or Bezier | 16 |
| Control points on one curve | 65536 (`2^16`) |
| Knots on one curve | 65553 (`2^16 + 16 + 1`) |
| Vertices in one polyline | 1048576 (`2^20`) |
| Segments in one composite curve | 65536 (`2^16`) |
| Curve nesting depth (outermost counts as 1) | 4 |
| Dimensions | exactly 2 |

These are numbers, not adjectives. The reader's resource limits (P4.8) cite this table and do not
restate it. A count is checked before it sizes any allocation. Raising a maximum is an ADR; lowering
one is a breaking change.

### NP-19 Affine transforms

An affine map `[a b tx; c d ty]` is usable when all six numbers pass NP-01, `tx` and `ty` pass
NP-02, `|a|, |b|, |c|, |d| <= 2^20`, and with `big = max(|a|, |b|, |c|, |d|)` the determinant satisfies
`|ad - bc| > REL * big^2`. Otherwise it is singular or out of range and is refused.

### NP-20 Closed flag

A curve whose `closed` flag is true must end where it starts: `near_point(start, end, scale)` with
`scale` the curve's extent. A closed flag that contradicts the geometry is refused. A curve whose
ends happen to coincide but whose flag is false is accepted unchanged; the flag is never inferred.

## What the tests prove

`tests/pathological/` holds one named case per rule edge. Every case names its rule, and a test
checks that each rule it names has a section in this file. Two meta-tests guard the constants from
both sides: widening any single tolerance by a factor of ten turns at least one case red, and so
does narrowing it. The evaluator probes (near-degenerate arcs, collinear rational hulls,
full-multiplicity interior knots, huge and mixed-magnitude coordinates, weights near zero,
denormal parameters, inverted and zero-length trims) are written as data with known truth before
any evaluator exists, so they are red until the geometry crate (P3.1) lands, and the geometry spike
(P1.3) runs them against its prototype.

A repository check fails if a tolerance-shaped numeric literal (an exponent-form float, or
`EPSILON`) appears in any Rust file outside `crates/pvec-numeric/src/policy.rs`.
