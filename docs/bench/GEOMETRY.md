# Geometry evaluation and tessellation benchmark (P8.4)

This is the cost profile of the exact curve mathematics, per curve type, per transform and per error budget, so that
P8.6 tunes against numbers and not a hunch. It measures; it changes no evaluator and no subdivision rule. The harness is
`crates/pvec-bench/benches/geometry.rs`, the driver is `tools/geometry-bench.sh`, the raw rows are `docs/bench/geometry.csv`
(one number per row, columns `table,entry,platform,curve,transform,budget_px,metric,value,note`), and the test that holds this
document to the CSV is `crates/pvec-bench/tests/issue_63_geometry.rs`.

## Inputs

* Corpus lock: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`.
  The per-type tables use ten synthetic curve types (below), 64 exact images of each. The huge-coordinate rows use the first
  6,000 buildable curves of `huge-coordinate-1e5` at their own coordinates. The viewport-weighted summary uses the real entity mix
  of `architectural-1e5` from the lock.
* Curves: line (a trimmed line segment, since `tessellate` refuses an unbounded line), polyline of 16 vertices, circle, ellipse,
  trimmed (a circular arc), composite (segment, arc, Bezier), cubic Bezier, degree 3 B-spline over 12 control points, degree 7
  B-spline over 20, rational B-spline with weights from 0.5 to 2. Each type is a base curve moved and scaled 1 to 4 times by
  `Curve2d::transform`, so tolerances differ from curve to curve.
* Transform arms: identity, translation, uniform scale 3, rotation by 0.5 rad plus scale 2, and `chain-offset`, the source to map
  step of the full chain, a translation by (6,480,000.125, 5,900,000.25) so the curve is evaluated at UTM-scale coordinates. The
  images are exact (`Curve2d::transform` refuses what it cannot represent), and an image that is refused would be a row with a note.
* Budgets: 0.1, 0.25, 0.5 and 1.0 screen pixels. A pixel is a two-thousandth of the curve's diagonal, so a curve is drawn about
  two thousand pixels across whatever its size. The achieved error is measured here, as the largest distance from 4096 points of the
  exact curve to the emitted polyline, and not read from the tessellator's own report.
* Baseline: the P1.3 prototype (`spikes/geometry-api`, binary `corpus_eval`) evaluating the same reference cubic at the same
  200,000 parameters with the same median-of-15 method.

## Platforms

Every row says its platform. **native x64** is MARS (Intel, `linux/amd64` container, rustc 1.99.1, release). **arm64** is mercury
(Apple silicon, the Colima VM, `linux/arm64` container, same compiler). Both hosts were shared and loaded while the harness ran
(load averages from 6 to 12), so read a time as a ratio between rows of one table and not as an absolute speed; no arm64 number is
compared with an x64 number anywhere in the verdicts, and the two tables below are separate for that reason. `perf` is not available
in the containers, so instructions and cache misses per evaluation are not reported, and `pvec-geometry` has no SIMD kernel, so there
is no scalar against SIMD variance to report either; both are rows with a note in the CSV.

## Thresholds

Declared in `tests/issue_63_geometry.rs` and `src/geometry_bench.rs` before the first run. Failure if:

* `point_at` on a degree 3 B-spline exceeds 150 ns per evaluation, or is slower than the P1.3 prototype at all;
* `tessellate` allocates more than once per curve at any budget;
* halving the screen error budget grows a conic's segment count by more than 1.5x (the analytic bound should scale with the
  square root of the budget, not linearly);
* evaluation at a 10^6 coordinate offset costs more than 1.2x the identity case, or its accuracy leaves P1.2's scale-aware tolerance;
* any tessellation exceeds its requested screen error.

A missed threshold is a result and is recorded below. P8.6 and P8.7 decide what to do about it.

## Results

- bspline3-point-at-150ns on linux-aarch64: PASS (point_at on the reference cubic 37.7 ns (limit 150 ns), P1.3 prototype 40.0 ns)
- tessellate-one-allocation on linux-aarch64: FAIL (19 allocations for one tessellation of a composite at 0.1 px, the most of 40 rows (limit 1))
- conic-halving-1.5x on linux-aarch64: FAIL (the largest growth over 4 halvings is 2.00x, circle from 1 px to 0.5 px: 64.0 to 128.0 segments (limit 1.5x))
- offset-1.2x-and-accuracy on linux-aarch64: PASS (the dearest offset costs 1.06x (synthetic line: 8.0 ns against 7.6 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere)
- error-budget-never-exceeded on linux-aarch64: FAIL (1 of 40 rows over the budget: bspline3 at 0.25 px reached 0.250185 px)
- bspline3-point-at-150ns on linux-x86_64: FAIL (point_at on the reference cubic 19.7 ns (limit 150 ns), P1.3 prototype 18.6 ns)
- tessellate-one-allocation on linux-x86_64: FAIL (19 allocations for one tessellation of a composite at 0.1 px, the most of 40 rows (limit 1))
- conic-halving-1.5x on linux-x86_64: FAIL (the largest growth over 4 halvings is 2.00x, circle from 1 px to 0.5 px: 64.0 to 128.0 segments (limit 1.5x))
- offset-1.2x-and-accuracy on linux-x86_64: PASS (the dearest offset costs 1.19x (synthetic ellipse: 19.6 ns against 16.5 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere)
- error-budget-never-exceeded on linux-x86_64: FAIL (1 of 40 rows over the budget: bspline3 at 0.25 px reached 0.250185 px)

### What the verdicts say

**The evaluator is fast and the prototype is not beaten.** `point_at` on the reference cubic is 19.7 ns on x64 and 37.7 ns on arm64,
far inside 150 ns. The P1.3 prototype does the same work in 18.6 ns on x64, so the second clause of the threshold ("slower than the
prototype at all") fails on x64 by about 6%, and passes on arm64 (40.0 ns for the prototype on that loaded host). The production
`eval` validates the parameter and the side where the prototype does less, and 1 ns of that is what I measure. Whether 6% matters is
for P8.6; the 150 ns half has a factor of seven of room.

**`tessellate` allocates 6 to 19 times per curve, not once.** The tessellator grows one `Vec` by pushing and recurses through a
`Refine` struct, so every doubling of the output is a reallocation, and a composite tessellates its segments into their own vectors
and joins them (19 calls at 0.1 px). A line allocates once. This is the plainest gap between the code and the threshold, and the fix
(count the output first, or reserve from the budget) is the kind of parameter P8.6 tunes.

**Halving the budget doubles a conic.** A circle needs 64 segments at 1 px, 128 at 0.5 and 256 at 0.1: bisection from eight spans can
only double, so a budget cut that needs 1.41x the vertices pays 2x when it crosses a threshold. The ellipse and the cubic grow in
smaller steps because their curvature varies (an ellipse goes 104, 144, 256 over 0.5, 0.25, 0.1 px), but a uniformly curved circle
shows the step. Over the long run the growth is the square root; at any one halving it can be 2x.

**An offset of 10^6 costs nothing measurable, and the accuracy is inside the tolerance.** Evaluation at the offset is within noise of
the identity case: on x64 the ratios are 0.97 to 1.01 for every type but the ellipse, on arm64 0.89 to 1.06 (the small curves wobble most on
a loaded host). The dearest ratio, 1.19x for the ellipse on x64, is one row that I read as noise, because the ellipse's other
transforms agree with identity to 2%, but it is 0.01 under the limit and I would not call it settled. The error of the offset image against the local frame
placed with a two-sum is at most 3.3e-9 world units on either platform (a rounding of a number near 6.5 million is 9.3e-10), against a smallest tolerance of 1.9e-8 from P1.2.

**One tessellation is a hair over its budget.** The cubic at 0.25 px reaches 0.250185 px measured against 4096 points of the exact
curve. The tessellator checks each chord at a handful of interior samples, so a deviation between the samples can exceed the budget by
a part in a thousand. It is the smallest possible miss, and it is a miss: "a speed number obtained by breaking the error budget is not
a result". The other 39 rows are inside their budgets, and the circle and the ellipse are under by a wide margin (the circle's 0.213 px
at a 0.25 px budget is the step above).

**There is no analytic hit test to measure.** `hit_test_ns` is a row with a note on every curve and transform: the picking API is P7.5
(`pvec#56`) and is open. The row is there so the gap is on the record and not silently dropped.

## Evaluation cost, native x64

Nanoseconds per call, mean over 64 images of each base curve and 256 parameters each (`bounds` per curve). The transform arms are
the exact image of the curve; they change the numbers inside the curve and not the code that evaluates it, which is why a row is
nearly flat across them.

| curve | metric | identity | translation | uniform-scale | rotation-scale | chain-offset |
|---|---|---|---|---|---|---|
| line | point_at | 4.6 | 4.7 | 4.6 | 4.6 | 4.6 |
| line | derivative | 5.7 | 5.8 | 5.7 | 5.7 | 5.6 |
| line | bounds | 12.2 | 12.0 | 12.0 | 12.5 | 12.0 |
| polyline | point_at | 4.7 | 4.8 | 4.7 | 4.7 | 4.7 |
| polyline | derivative | 5.2 | 5.2 | 5.2 | 5.2 | 5.2 |
| polyline | bounds | 28.7 | 28.8 | 28.4 | 27.4 | 28.5 |
| circle | point_at | 16.3 | 16.4 | 16.4 | 16.4 | 16.4 |
| circle | derivative | 17.8 | 17.8 | 17.8 | 17.9 | 17.9 |
| circle | bounds | 59.6 | 59.6 | 59.5 | 94.3 | 59.5 |
| ellipse | point_at | 16.5 | 16.4 | 16.5 | 16.8 | 19.6 |
| ellipse | derivative | 17.8 | 17.8 | 17.8 | 25.3 | 17.9 |
| ellipse | bounds | 60.4 | 60.5 | 60.4 | 130.5 | 60.6 |
| trimmed | point_at | 14.9 | 15.1 | 14.9 | 15.0 | 14.9 |
| trimmed | derivative | 19.4 | 19.4 | 19.3 | 19.4 | 19.4 |
| trimmed | bounds | 44.0 | 44.3 | 44.0 | 78.9 | 44.0 |
| composite | point_at | 23.0 | 23.1 | 23.0 | 23.0 | 23.0 |
| composite | derivative | 36.4 | 36.4 | 36.4 | 36.5 | 36.4 |
| composite | bounds | 131.7 | 131.5 | 131.6 | 189.3 | 132.0 |
| bezier | point_at | 22.0 | 22.0 | 22.0 | 22.0 | 22.0 |
| bezier | derivative | 25.7 | 25.7 | 25.7 | 25.7 | 25.7 |
| bezier | bounds | 80.5 | 80.6 | 80.5 | 91.7 | 84.1 |
| bspline3 | point_at | 19.7 | 19.7 | 19.7 | 19.7 | 19.8 |
| bspline3 | derivative | 22.4 | 22.5 | 22.5 | 22.5 | 22.4 |
| bspline3 | bounds | 26.8 | 26.7 | 26.7 | 25.7 | 26.7 |
| bspline7 | point_at | 40.2 | 40.2 | 40.2 | 40.3 | 40.3 |
| bspline7 | derivative | 43.3 | 43.1 | 43.2 | 47.6 | 43.2 |
| bspline7 | bounds | 29.9 | 30.0 | 30.4 | 29.0 | 30.0 |
| nurbs | point_at | 23.3 | 23.3 | 23.3 | 23.3 | 23.3 |
| nurbs | derivative | 51.4 | 51.4 | 51.5 | 51.4 | 51.5 |
| nurbs | bounds | 25.3 | 25.3 | 25.2 | 24.4 | 25.5 |

| curve | budget px | segments | ns/segment | allocs (max) | achieved error px |
|---|---|---|---|---|---|
| line | 0.1 | 1.0 | 53 | 1 | 0.0000 |
| line | 0.25 | 1.0 | 39 | 1 | 0.0000 |
| line | 0.5 | 1.0 | 32 | 1 | 0.0000 |
| line | 1 | 1.0 | 27 | 1 | 0.0000 |
| polyline | 0.1 | 15.0 | 1 | 1 | 0.0000 |
| polyline | 0.25 | 15.0 | 1 | 1 | 0.0000 |
| polyline | 0.5 | 15.0 | 2 | 1 | 0.0000 |
| polyline | 1 | 15.0 | 2 | 1 | 0.0000 |
| circle | 0.1 | 256.0 | 301 | 9 | 0.0532 |
| circle | 0.25 | 128.0 | 305 | 8 | 0.2130 |
| circle | 0.5 | 128.0 | 333 | 8 | 0.2130 |
| circle | 1 | 64.0 | 323 | 7 | 0.8517 |
| ellipse | 0.1 | 256.0 | 323 | 9 | 0.0705 |
| ellipse | 0.25 | 144.0 | 296 | 8 | 0.2480 |
| ellipse | 0.5 | 104.0 | 325 | 7 | 0.4811 |
| ellipse | 1 | 72.0 | 277 | 7 | 0.9668 |
| trimmed | 0.1 | 128.0 | 324 | 8 | 0.0498 |
| trimmed | 0.25 | 64.0 | 324 | 7 | 0.1992 |
| trimmed | 0.5 | 64.0 | 291 | 7 | 0.1992 |
| trimmed | 1 | 32.0 | 274 | 6 | 0.7968 |
| composite | 0.1 | 193.0 | 416 | 19 | 0.0754 |
| composite | 0.25 | 119.0 | 439 | 17 | 0.2450 |
| composite | 0.5 | 65.0 | 422 | 16 | 0.4963 |
| composite | 1 | 59.0 | 405 | 15 | 0.9947 |
| bezier | 0.1 | 128.0 | 585 | 8 | 0.0500 |
| bezier | 0.25 | 64.0 | 575 | 7 | 0.2003 |
| bezier | 0.5 | 64.0 | 575 | 7 | 0.2003 |
| bezier | 1 | 32.0 | 531 | 6 | 0.8013 |
| bspline3 | 0.1 | 229.0 | 458 | 8 | 0.0997 |
| bspline3 | 0.25 | 151.0 | 443 | 8 | 0.2502 |
| bspline3 | 0.5 | 103.0 | 445 | 7 | 0.4809 |
| bspline3 | 1 | 74.0 | 438 | 7 | 0.9992 |
| bspline7 | 0.1 | 220.0 | 785 | 8 | 0.0992 |
| bspline7 | 0.25 | 138.0 | 756 | 8 | 0.2479 |
| bspline7 | 0.5 | 100.0 | 753 | 7 | 0.4834 |
| bspline7 | 1 | 72.0 | 775 | 7 | 0.8370 |
| nurbs | 0.1 | 211.0 | 636 | 8 | 0.0999 |
| nurbs | 0.25 | 125.0 | 620 | 7 | 0.2489 |
| nurbs | 0.5 | 99.0 | 622 | 7 | 0.4840 |
| nurbs | 1 | 64.0 | 613 | 7 | 0.9779 |

## Evaluation cost, arm64

| curve | metric | identity | translation | uniform-scale | rotation-scale | chain-offset |
|---|---|---|---|---|---|---|
| line | point_at | 7.6 | 7.4 | 6.9 | 7.1 | 8.0 |
| line | derivative | 7.8 | 7.6 | 6.9 | 7.4 | 7.4 |
| line | bounds | 20.8 | 18.7 | 18.8 | 21.8 | 38.4 |
| polyline | point_at | 5.3 | 5.3 | 5.3 | 4.7 | 4.7 |
| polyline | derivative | 5.6 | 5.6 | 5.6 | 5.1 | 5.1 |
| polyline | bounds | 24.3 | 24.3 | 24.3 | 21.8 | 21.8 |
| circle | point_at | 27.0 | 26.0 | 26.9 | 25.7 | 24.8 |
| circle | derivative | 27.9 | 26.5 | 27.5 | 28.0 | 26.6 |
| circle | bounds | 95.5 | 91.3 | 91.8 | 135.5 | 90.2 |
| ellipse | point_at | 27.0 | 26.8 | 27.2 | 27.5 | 27.2 |
| ellipse | derivative | 27.8 | 28.7 | 28.8 | 28.7 | 28.9 |
| ellipse | bounds | 94.4 | 98.2 | 98.5 | 149.4 | 98.6 |
| trimmed | point_at | 27.5 | 26.2 | 26.3 | 26.3 | 26.3 |
| trimmed | derivative | 33.3 | 33.0 | 33.2 | 33.3 | 33.4 |
| trimmed | bounds | 77.7 | 78.4 | 78.3 | 125.0 | 78.4 |
| composite | point_at | 36.9 | 36.2 | 36.1 | 36.6 | 36.7 |
| composite | derivative | 56.2 | 56.7 | 56.5 | 56.2 | 57.4 |
| composite | bounds | 254.7 | 255.0 | 254.1 | 320.6 | 256.1 |
| bezier | point_at | 29.4 | 29.4 | 29.5 | 29.6 | 29.6 |
| bezier | derivative | 31.1 | 31.1 | 31.1 | 31.2 | 31.1 |
| bezier | bounds | 137.3 | 137.3 | 137.8 | 171.9 | 137.5 |
| bspline3 | point_at | 37.7 | 37.7 | 37.7 | 37.0 | 37.9 |
| bspline3 | derivative | 40.7 | 40.7 | 43.0 | 40.8 | 40.8 |
| bspline3 | bounds | 20.0 | 19.9 | 19.9 | 20.1 | 19.9 |
| bspline7 | point_at | 125.7 | 131.5 | 125.8 | 125.9 | 126.1 |
| bspline7 | derivative | 115.5 | 116.6 | 129.5 | 115.4 | 115.6 |
| bspline7 | bounds | 26.0 | 25.2 | 25.4 | 25.3 | 25.2 |
| nurbs | point_at | 59.8 | 60.2 | 60.4 | 59.9 | 60.3 |
| nurbs | derivative | 104.6 | 100.4 | 100.4 | 101.8 | 101.9 |
| nurbs | bounds | 17.7 | 17.9 | 17.7 | 17.8 | 17.7 |

| curve | budget px | segments | ns/segment | allocs (max) | achieved error px |
|---|---|---|---|---|---|
| line | 0.1 | 1.0 | 60 | 1 | 0.0000 |
| line | 0.25 | 1.0 | 52 | 1 | 0.0000 |
| line | 0.5 | 1.0 | 53 | 1 | 0.0000 |
| line | 1 | 1.0 | 51 | 1 | 0.0000 |
| polyline | 0.1 | 15.0 | 8 | 1 | 0.0000 |
| polyline | 0.25 | 15.0 | 7 | 1 | 0.0000 |
| polyline | 0.5 | 15.0 | 9 | 1 | 0.0000 |
| polyline | 1 | 15.0 | 4 | 1 | 0.0000 |
| circle | 0.1 | 256.0 | 533 | 9 | 0.0532 |
| circle | 0.25 | 128.0 | 506 | 8 | 0.2130 |
| circle | 0.5 | 128.0 | 490 | 8 | 0.2130 |
| circle | 1 | 64.0 | 469 | 7 | 0.8517 |
| ellipse | 0.1 | 256.0 | 496 | 9 | 0.0705 |
| ellipse | 0.25 | 144.0 | 496 | 8 | 0.2480 |
| ellipse | 0.5 | 104.0 | 521 | 7 | 0.4811 |
| ellipse | 1 | 72.0 | 507 | 7 | 0.9668 |
| trimmed | 0.1 | 128.0 | 574 | 8 | 0.0498 |
| trimmed | 0.25 | 64.0 | 533 | 7 | 0.1992 |
| trimmed | 0.5 | 64.0 | 504 | 7 | 0.1992 |
| trimmed | 1 | 32.0 | 521 | 6 | 0.7968 |
| composite | 0.1 | 193.0 | 698 | 19 | 0.0754 |
| composite | 0.25 | 119.0 | 661 | 17 | 0.2450 |
| composite | 0.5 | 65.0 | 572 | 16 | 0.4963 |
| composite | 1 | 59.0 | 600 | 15 | 0.9947 |
| bezier | 0.1 | 128.0 | 784 | 8 | 0.0500 |
| bezier | 0.25 | 64.0 | 768 | 7 | 0.2003 |
| bezier | 0.5 | 64.0 | 728 | 7 | 0.2003 |
| bezier | 1 | 32.0 | 768 | 6 | 0.8013 |
| bspline3 | 0.1 | 229.0 | 774 | 8 | 0.0997 |
| bspline3 | 0.25 | 151.0 | 802 | 8 | 0.2502 |
| bspline3 | 0.5 | 103.0 | 748 | 7 | 0.4809 |
| bspline3 | 1 | 74.0 | 736 | 7 | 0.9992 |
| bspline7 | 0.1 | 220.0 | 1900 | 8 | 0.0992 |
| bspline7 | 0.25 | 138.0 | 1885 | 8 | 0.2479 |
| bspline7 | 0.5 | 100.0 | 1951 | 7 | 0.4834 |
| bspline7 | 1 | 72.0 | 1891 | 7 | 0.8370 |
| nurbs | 0.1 | 211.0 | 996 | 8 | 0.0999 |
| nurbs | 0.25 | 125.0 | 979 | 7 | 0.2489 |
| nurbs | 0.5 | 99.0 | 1040 | 7 | 0.4840 |
| nurbs | 1 | 64.0 | 1059 | 7 | 0.9779 |

## Viewport-weighted summary

The architectural entry `architectural-1e5` has the mix recorded in the lock (73.0% lines, 16.7% polylines, 5.7% trimmed curves,
2.8% circles, 0.9% ellipses, 0.9% B-splines). Weighting the per-type costs by it gives the `summary` rows of the CSV: the weighted
`point_at` cost per entity and the weighted cost of one tessellation at 0.5 px. Lines and polylines, three quarters of the drawing,
tessellate in tens of nanoseconds per vertex; the conics and splines, a fifth of it, are where the microseconds go. The cost profile
that `libviprs/maplibre-pvec#1` can publish is the per-type tables above plus these two rows.

## What this does not do

It changes no evaluator and no subdivision strategy (P8.6 tunes the measured parameters), measures no GPU rasterisation (EPIC M) and
no STEP evaluation (P8.2), and adds no curve type. P1.3 in this document is the prototype baseline only.
