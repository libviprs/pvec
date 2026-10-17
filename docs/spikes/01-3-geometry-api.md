# Spike P1.3: the in-memory geometry API

Code: `spikes/geometry-api/` (throwaway). Every timing below is **native x64**: an Intel Core
i7-14700F on MARS, `rustc 1.99.1`, `--release`, one thread, inside the `pvec-rust:1.99.1` container
run with `--platform linux/amd64`. No arm64 run was made, so nothing here is labelled arm64.
Reproduce with `tools/spike.sh geometry-api 'cargo run -q --release --bin bench'` (and `--bin
accuracy`, `cargo test`).

## What I built

All nine v1 kinds from `ISO_PROFILE.md`: `Line`, `Circle`, `Ellipse`, `Polyline`, `Trimmed`,
`Composite`, `Bezier`, `BSpline`, `RationalBSpline`. Each supports exact evaluation, derivatives of
any order (one-sided where the numeric policy requires it), bounds, and an exact affine transform.

* Conics are closed form. Bounds of a full ellipse and of a trimmed arc come from the stationary
  points of `x(t)` and `y(t)` (`tan t = b y_k / (a x_k)`), plus the arc end points. Nothing is sampled.
* B-splines are de Boor over `[f64; N]`, `N = 2`, and `N = 3` for the rational case (weights carried in
  homogeneous coordinates, so a rational curve is the same code with one more component). Derivatives
  differentiate the control polygon (`p (P_{i+1} - P_i) / (u_{i+p+1} - u_{i+1})`), and the rational
  derivative uses the quotient recursion `C^(k) = (A^(k) - sum binom(k,i) w^(i) C^(k-i)) / w`.
* Bezier uses de Casteljau with no knot array; bounds are tight to degree 3 (roots of a quadratic).
* An affine map of a circle or ellipse is re-expressed in canonical form (see "What the prototype
  corrected" below).

## The trait shape that should survive into P3.1

```rust
pub trait Curve2d {
    fn domain(&self) -> (f64, f64);
    fn eval(&self, t: f64) -> Result<Pt, Error>;
    fn derivative(&self, t: f64, order: usize, side: Side) -> Result<Pt, Error>;
    fn bounds(&self) -> Result<Bounds, Error>;
    fn transform(&self, a: &Affine) -> Result<(Curve, ParamMap), Error>;
}
```

with a **closed enum** `Curve` over the nine kinds implementing it, and `Side { Left, Right, Both }`
from `pvec-numeric`. Reasons, with numbers:

* Every fallible step returns `Result`. Construction validates under the numeric policy, so a value
  that exists is valid and `eval` never needs to re-check the control hull.
* `transform` returns the image curve **and** the map from old to new parameters. A circle becomes an
  ellipse, a line's parameter rescales, a trimmed arc keeps its end points; without the map a caller
  cannot carry a trim or a picking parameter across a transform.
* Closed enum versus `Box<dyn Curve2d>` on a mixed set of ten kinds (`domain()` plus `eval()` per
  call): **29.2 ns enum, 30.3 ns trait object**, a 1.1 ns (4%) difference. Run twice, 29.2/30.3 and
  29.1/30.3. The speed margin is small and I do not rest the decision on it. I rest it on the profile:
  the set of kinds is fixed by `ISO_PROFILE.md` and a decoder must *reject* anything else, which is
  exactly what a closed enum does and an open trait object does not. Extensions arrive through the
  P3.5 governance rules, which can add a variant. `size_of::<Curve>()` is 104 bytes.

## Storage decision: control points and knots

Two shapes, same generic core (`Core<P: Cps, F: AsRef<[f64]>>`), so a measured difference is storage
and nothing else:

* **owned**: `Vec<[f64; 2]>` and `Vec<f64>`;
* **borrowed**: `Flat(&[f64])` (interleaved `x0 y0 x1 y1 ...`) and `&[f64]` over a buffer the caller owns.

Measured, B-spline (rational rows in the second table), median of 15 runs of 200,000 calls, native x64:

| curve | owned eval | borrowed eval | borrowed vs owned |
|---|---|---|---|
| degree 3, 8 cps | 18.3 ns | 18.4 ns | +0.5% |
| degree 3, 64 cps | 21.8 ns | 22.3 ns | +2.3% |
| degree 3, 1024 cps | 29.2 ns | 29.9 ns | +2.4% |
| degree 5, 64 cps | 31.2 ns | 31.5 ns | +1.0% |
| rational, degree 3, 8 cps | 23.2 ns | 26.7 ns (21.7 ns in the second run) | noise |
| rational, degree 3, 64 cps | 26.5 ns | 25.3 ns | -4.5% |
| rational, degree 3, 1024 cps | 33.9 ns | 32.9 ns | -2.9% |
| rational, degree 5, 64 cps | 37.0 ns | 36.1 ns | -2.4% |

A second full run reproduced every eval number within 1 ns, apart from the 8-cp rational borrowed
row (26.7 against 21.7 ns), which I treat as a measurement outlier rather than a result.

Build cost (validate, plus copy for the owned case), ns per curve:

| curve | owned build | build from a decoded flat buffer (copy + validate) | borrowed build (validate only) |
|---|---|---|---|
| degree 3, 8 cps | 203 | 202 | 181 |
| degree 3, 64 cps | 1214 | 1216 | 1204 |
| degree 3, 1024 cps | 18612 | 18628 | 18514 |
| degree 5, 64 cps | 1217 | 1218 | 1205 |

(One 64-cp copy-and-validate sample came out at 1786 in the second run; the other fifteen cells agreed.)

Struct size: 96 bytes owned, 72 bytes borrowed (plus the heap blocks the owned one points at).

**Decision: borrowed-capable generic core with owned as the convenience instantiation.** The speed
margin is not the reason, and I am not going to pretend it is: evaluation differs by at most 2.4%
one way and 4.5% the other, inside the run-to-run noise I saw, and build cost is dominated by
validation, so the copy does not show either (202 against 181 ns at 8 control points, 1216 against
1204 at 64). The decision rests on structure. The container reader (phase 4) hands out slices over a
buffer it already owns, and untrusted-input bounds (P4.8) are easier to enforce when a decode does
not allocate per curve. The rejected option is **owned-only**: it forces an allocation and a copy per
curve for no measured speed gain, and its margin over borrowed is **zero or slightly negative**
(borrowed is at most 2.4% slower on non-rational curves and up to 4.5% faster on rational ones).
Threshold that reverses it: if the production reader cannot give properly aligned `f64` slices and
the unaligned fallback makes borrowed evaluation more than 10% slower than owned on the 1024-control-
point curve (currently +2.4%), switch the reader-facing type to owned.

**A finding the storage question exposed.** Validating a curve costs about 18 ns per control point:
18.6 microseconds for the 1024-point curve, roughly 600 evaluations of it (29.2 ns each). So validation
must happen once at decode and never on the access path; the reader must not re-validate per query.
P4.8's bounds are checked before allocation, and NP-14 and NP-08 are checked once at construction.

## Evaluation cost per curve type

Native x64, ns per call, median of 15 runs of 200,000 parameters (bounds and transform: 2,000 calls):

| kind | eval | derivative (1st) | bounds | transform |
|---|---|---|---|---|
| line | 3.8 | 2.3 | 1.5 | 14.2 |
| circle | 16.8 | 18.2 | 81.1 | 57.3 |
| ellipse | 17.0 | 18.3 | 86.2 | 51.5 |
| polyline (3 segments) | 3.6 | 5.3 | 4.9 | 36.9 |
| trimmed arc | 17.7 | 20.7 | 67.2 | 79.1 |
| trimmed ellipse, reversed | 20.4 | 22.5 | 77.5 | 73.0 |
| trimmed line | 4.6 | 7.1 | 8.7 | 42.9 |
| composite (line, arc, Bezier) | 22.4 | 35.2 | 106.6 | 564.9 |
| Bezier, degree 3 | 9.7 | 27.3 | 55.5 | 99.6 |
| B-spline, degree 3, 6 cps | 19.2 | 60.9 | 8.0 | 179.7 |
| rational B-spline, degree 2, 4 cps | 20.7 | 115.6 | 4.2 | 150.7 |

A second full run agreed within about 5% on most cells. The exceptions: circle and ellipse eval
(16.8 and 17.0 against 18.8 and 16.9), line eval (3.8 against 1.9 ns, a loop the optimiser treats
differently between runs), the circle derivative (18.2 against 28.2), polyline bounds and transform
(4.9 and 36.9 against 8.5 and 61.8). Read single-digit-nanosecond cells as "a few ns", not as a ranking.

Two things I did not expect. The B-spline and rational derivatives are 3 to 6 times the cost of
evaluation because they rebuild the derivative control polygon with allocations; that is fine for a
prototype and wrong for P3.1, which should cache derivative polygons or evaluate derivatives in place.
And B-spline `bounds` is cheap only because it is the control hull, which is an enclosure, not the tight
extent (below).

## Accuracy against closed form

Native x64, `accuracy` binary. Max and mean deviation:

| check | samples | max | mean | measured as |
|---|---|---|---|---|
| rotated-frame circle vs `c + r(cos(t+phi), sin(t+phi))` | 100,000 | 7.2e-16 | 2.2e-16 | relative to radius |
| 9-point rational quadratic NURBS circle, radial error | 100,001 | 6.3e-16 | 8.9e-17 | relative to radius |
| ellipse `transform()` vs the map applied to evaluated points | 100,000 | 7.3e-16 | 1.3e-16 | relative to extent |
| B-spline degree 2 reproducing `(t, t^2)` | 100,001 | 2.2e-16 | 4.1e-17 | absolute |
| rational degree 2, equal weights 2.5, same truth | 100,001 | 3.3e-16 | 5.1e-17 | absolute |
| B-spline degree 3 reproducing `(t, t^3)` | 100,001 | 2.2e-16 | 4.6e-17 | absolute |
| rational degree 3, equal weights 2.5 | 100,001 | 4.4e-16 | 5.6e-17 | absolute |
| B-spline degree 5 reproducing `(t, t^5)` | 100,001 | 4.4e-16 | 5.8e-17 | absolute |
| rational degree 5, equal weights 2.5 | 100,001 | 6.7e-16 | 6.6e-17 | absolute |
| first derivative, degrees 2, 3, 5 | 99,999 each | 3.3e-16, 8.9e-16, 2.7e-15 | | absolute |
| full-ellipse analytic bounds vs closed form | 4 | 2.2e-16 | | absolute |

The spline truth is not a sample of mine: with control points built from the blossom of `t^p`
(`x_i` the Greville average of the interior knots, `y_i` their product) a degree-`p` spline reproduces
`(t, t^p)` exactly, on deliberately non-uniform knots. The issue asks for comparison against O1.5's
pathological known-truth probes; `pvec-gold-fixtures` has no probes yet (the repository holds only its
README), so the truth here is my own and the 15 pathological probes of `tests/pathological/`. When O1.5
lands its probes should be run through the same `Evaluator` trait and added to this table.

The largest observed error, 2.7e-15, is about 12 roundoffs. `REL` is `1e-10`, so the policy leaves five
orders of magnitude of headroom over what evaluation costs. That is deliberate (it has to absorb a Part 21
text round trip and chained transforms), but it also means `REL` could be tightened by a factor of 10 to
100 without turning a measured case red. I am not doing that in this PR: P2.6's oracle comparison is the
place to find out what the other converters do, and tightening is an ADR.

## Analytic bounds

Acceptance asked that a trimmed arc and a full ellipse match closed-form extrema within the P1.2
tolerance, and that sampled bounds turn the check red. Both tests exist and pass:

* full rotated ellipse: matches `c +- sqrt(a^2 cos^2(phi) + b^2 sin^2(phi))` (and the y analogue) to
  2.2e-16;
* a 30 to 200 degree arc of a circle, where the x minimum is at an interior stationary point and the
  other three extremes are end points, matches the hand-derived extrema;
* 64 samples of the same rotated ellipse miss the closed-form x extreme by **2.827e-3**, about 2.8e6
  times the policy tolerance of about 1e-9 at that scale, so substituting sampling turns the check red.

B-spline and rational B-spline `bounds()` return the **control hull**, a guaranteed enclosure that is
not tight. A trimmed spline returns the hull of its whole basis. Tight spline bounds need root finding
on the derivative per span; that belongs in P3.1 and is the main thing the prototype does not do.

## What the prototype corrected

Two earlier statements were wrong, and fixing them is part of this PR.

1. **`ISO_PROFILE.md`, section 13** said a transformed conic keeps its parameter. It does not. The image
   of `c + a cos t x + b sin t y` is `c' + u cos t + v sin t` with conjugate vectors `u`, `v` that are
   generally not orthogonal, so putting it in canonical form shifts the parameter by
   `t0 = atan2(2 u.v, u.u - v.v) / 2`, and reverses it when the map reverses handedness. A line's
   parameter scales by the length of the image of its direction. The profile row now says so, and the
   prototype returns a `ParamMap` for each.
2. **`NUMERIC_POLICY.md`, NP-12** judged a line trim against the width of the line's domain. A line has
   none; using `2 * MAX_ABS_COORD` made every trim shorter than about 220 look collapsed, and the first
   sample (a segment of length 5) was refused. NP-12 now treats an unbounded basis separately: a trim is
   collapsed when its length is degenerate under NP-06. `Policy::normalise_unbounded_trim` implements it,
   and the pathological case `short-line-trim` covers it. The prototype found this by failing, which is
   what it was for.

## Keeping serialization types out of the API

The prototype's `Cargo.toml` has one dependency, `pvec-numeric`, and two tests enforce the rule:
`manifest_has_no_serialization_dependency` (no `serde`, `flatbuffers`, `prost`, `hdf5`, `bincode`,
`rkyv`, `capnp` or `bytemuck` in `[dependencies]`) and `no_source_file_names_a_serialization_crate`.
The mechanism P3.1 will use is the layering those tests protect:

* `pvec-geometry` depends on `std` and `pvec-numeric` only;
* every codec is its own crate (`pvec-codec-*`) that depends on `pvec-geometry`, never the other way
  round, so a generated type can appear in a codec's private code and cannot appear in a geometry
  signature because the geometry crate cannot name it;
* the same two tests move into `crates/pvec-checks` and cover every geometry crate.

That is a dependency-graph guarantee, which is the strong form. P3.1 should add a public-signature
check as well (rustdoc JSON) as defence in depth; I have not built that here.

## Numeric-policy probes

All 15 evaluator probes of `tests/pathological/probes.rs` pass against this prototype
(`every_pathological_probe_passes_against_the_prototype`), including the two-sided-derivative refusal
and the one-sided derivatives at a full-multiplicity interior knot, which differ on the two sides
((6, -18) left, (6, -6) right) as the probe predicts. In the main repository they remain `#[ignore]`d
and red, because the prototype is a spike and not the evaluator P3.1 will deliver.

## Tests

17 tests in `spikes/geometry-api/tests/geometry.rs`, all passing on MARS (native x64), among them: all
nine kinds evaluate inside their bounds; derivatives match central differences (first order on all
kinds, second order on the rational spline); transforms are exact for every kind under six maps
(translation, rotation with scale, anisotropic scale, shear, reflection, reflection with rotation);
owned and borrowed storage are bit identical; a Bezier equals its clamped single-span B-spline.

## Not done, on purpose

Periodic (wrapped) B-splines, tight spline bounds, derivative caching, SIMD, WASM, arm64 timings. The
production crate, codecs and tessellation are P3.1 to P3.3 and P7.2.
