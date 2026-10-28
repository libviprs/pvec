# Geometry evaluation and tessellation benchmark (P8.4)

This is the cost profile of the exact curve mathematics, per curve type, per transform and per error budget, so that
P8.6 tunes against numbers and not a hunch. It measures. The first run (before P8.6) changed nothing; this file now holds the second, taken after the P8.6 changes to the tessellator and the cubic evaluator that its findings asked for. The harness is
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
(Apple silicon, the Colima VM, `linux/arm64` container, same compiler). Both hosts were shared while the harness ran
(load averages from 1.7 to 3.9 at the start of the second run, 6 to 12 in the first), so read a time as a ratio between rows of one table and not as an absolute speed; no arm64 number is
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

- bspline3-point-at-150ns on linux-aarch64: FAIL (point_at on the reference cubic 26.2 ns (limit 150 ns), P1.3 prototype 24.1 ns)
- tessellate-one-allocation on linux-aarch64: PASS (at most 1 allocations over 40 rows)
- conic-halving-1.5x on linux-aarch64: PASS (the largest growth over 4 halvings is 1.44x, ellipse from 1 px to 0.5 px: 64.0 to 92.0 segments (limit 1.5x))
- offset-1.2x-and-accuracy on linux-aarch64: PASS (the dearest offset costs 1.00x (synthetic composite: 29.1 ns against 29.0 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere)
- error-budget-never-exceeded on linux-aarch64: PASS (40 rows, none above its budget)
- bspline3-point-at-150ns on linux-x86_64: PASS (point_at on the reference cubic 18.0 ns (limit 150 ns), P1.3 prototype 18.7 ns)
- tessellate-one-allocation on linux-x86_64: PASS (at most 1 allocations over 40 rows)
- conic-halving-1.5x on linux-x86_64: PASS (the largest growth over 4 halvings is 1.44x, ellipse from 1 px to 0.5 px: 64.0 to 92.0 segments (limit 1.5x))
- offset-1.2x-and-accuracy on linux-x86_64: PASS (the dearest offset costs 1.01x (synthetic trimmed: 14.7 ns against 14.5 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere)
- error-budget-never-exceeded on linux-x86_64: PASS (40 rows, none above its budget)

### What the verdicts say

This is the second run. The first one (P8.4, before P8.6) failed three thresholds on both platforms and a fourth on x64; the
numbers it gave are quoted below as "first run", and `docs/bench/TUNING.md` has the rows for each change. The tessellator and
the cubic evaluator changed in between (pvec#65, pvec#192), the benchmark did not.

**The evaluator is fast, and the prototype is beaten on x64.** `point_at` on the reference cubic is 18.0 ns on x64 and 26.2 ns on
arm64, far inside 150 ns. The P1.3 prototype does the same work in 18.7 ns on x64, so the second clause of the threshold ("slower
than the prototype at all") passes there now; the first run had 19.7 ns against 18.6 ns. The de Boor loops index a slice of the
2p + 2 knots of the span, and the finite-result test is one compare on `x * 0 + y * 0`. On arm64 the prototype is 24.1 ns against
the 26.2 ns of `eval`, 8.7% slower, so that clause still fails there. The first run passed it (37.7 ns against 40.0 ns) only because
the host was loaded to 6 or more and both numbers were mostly waiting; on a quiet host the gap was 29.7 ns against 25.0 ns, and
the change brings it down by more than half. I did not find the remaining 2 ns, and I accept it in `TUNING.md` with that reason.

**`tessellate` allocates once per curve.** A tessellation writes into a stack buffer of 512 vertices and copies it once into the
result (a longer one is counted first and written into a buffer of exactly its length), and a composite writes all its segments
into the same buffer. The first run had 6 to 19 allocations per curve, from growing one `Vec` by pushing, from the `Refine`
recursion's own vectors and from the vector each composite segment made for itself. A line allocates once, as before.

**Halving the budget no longer doubles a conic.** A chord that is too far off is cut into about the square root of the ratio of its
deviation to the tolerance pieces, where the first run bisected, so a conic's count follows the square root of the budget in steps
of one piece per initial span and not in steps of two. A circle needs 72 segments at 1 px, 96 at 0.5 and 136 at 0.25 (first run:
64, 128, 128), an ellipse 64, 92 and 132. The largest growth over a halving is 1.44x (the ellipse from 1 px to 0.5 px), against 1.5x
allowed and 2.00x in the first run. The cut uses 1.1 times that square root: without the factor the ellipse grows 1.53x from 1 px to 0.5 px (68 to 104 segments),
and with 1.05 a circle lands on exactly 1.5x (64 to 96), because a chord that comes out just over its tolerance is cut in two and
that shows as a lump in the count.

**An offset of 10^6 costs nothing measurable, and the accuracy is inside the tolerance.** Evaluation at the offset is within noise
of the identity case: the ratios are 0.99 to 1.02 on x64 and 0.99 to 1.00 on arm64 (the dearest in the first run was 1.19x for the
ellipse on a loaded x64 host, which I read as noise then and do now). The error of the offset image against the local frame placed with a two-sum is at most 3.3e-9 world
units on either platform (a rounding of a number near 6.5 million is 9.3e-10), against a smallest tolerance of 1.9e-8 from P1.2.

**No tessellation is over its budget.** The cubic at 0.25 px reached 0.250185 px in the first run, measured against 4096 points of
the exact curve, because the tessellator checks each chord at seven interior samples and a deviation between them can pass the
budget by a part in a thousand. A chord is accepted now only when its sampled deviation times 1.05 is inside the tolerance, and all
40 rows are inside their budgets. The margin costs segments where a curve is not conic: the cubic spline goes from 229 to 255
segments at 0.1 px and from 74 to 81 at 1 px, about 9%, while the conics and the trimmed arc get fewer (a circle at 0.1 px has 216
against 256) because of the cut rule above.

**Cost per segment fell by a third.** The mean over the 40 rows of ns per segment is 244 ns on x64 (first run 379, and 363 in the
P8.6 baseline run on the same host) and 345 ns on arm64 (668, and 504 in the baseline), and the mean segment count over the rows
is 91.7 against 95.4 on the first run. Most of it is that the one buffer replaces the vector growth and the per-segment vectors.

**There is no analytic hit test to measure.** `hit_test_ns` is a row with a note on every curve and transform: the picking API is P7.5
(`pvec#56`) and is open. The row is there so the gap is on the record and not silently dropped.

## Evaluation cost, native x64

| curve | metric | identity | translation | uniform-scale | rotation-scale | chain-offset |
|---|---|---|---|---|---|---|
| line | point_at | 3.9 | 3.8 | 3.8 | 3.8 | 3.8 |
| line | derivative | 4.9 | 4.9 | 4.8 | 4.8 | 4.8 |
| line | bounds | 11.8 | 11.7 | 11.8 | 12.2 | 11.7 |
| polyline | point_at | 4.1 | 4.1 | 4.1 | 4.1 | 4.1 |
| polyline | derivative | 5.2 | 5.2 | 5.2 | 5.2 | 5.2 |
| polyline | bounds | 28.4 | 28.4 | 28.4 | 27.5 | 28.4 |
| circle | point_at | 16.1 | 16.2 | 16.2 | 16.2 | 16.2 |
| circle | derivative | 17.8 | 17.6 | 17.7 | 17.8 | 17.8 |
| circle | bounds | 59.5 | 59.5 | 59.6 | 94.2 | 59.5 |
| ellipse | point_at | 16.2 | 16.2 | 16.2 | 16.1 | 16.1 |
| ellipse | derivative | 17.7 | 17.8 | 17.8 | 17.7 | 17.7 |
| ellipse | bounds | 60.4 | 60.4 | 60.4 | 92.8 | 60.4 |
| trimmed | point_at | 14.5 | 14.7 | 14.7 | 14.6 | 14.7 |
| trimmed | derivative | 19.1 | 19.1 | 19.0 | 19.1 | 19.1 |
| trimmed | bounds | 44.0 | 44.0 | 44.0 | 78.8 | 44.0 |
| composite | point_at | 22.5 | 22.4 | 22.4 | 22.4 | 22.5 |
| composite | derivative | 36.4 | 36.4 | 36.4 | 36.4 | 36.4 |
| composite | bounds | 131.5 | 131.5 | 131.4 | 189.6 | 131.4 |
| bezier | point_at | 20.3 | 20.2 | 20.2 | 20.2 | 20.2 |
| bezier | derivative | 25.8 | 25.8 | 25.8 | 25.8 | 25.8 |
| bezier | bounds | 80.1 | 80.0 | 80.1 | 91.7 | 80.1 |
| bspline3 | point_at | 18.0 | 18.0 | 18.0 | 18.0 | 18.0 |
| bspline3 | derivative | 21.6 | 21.5 | 21.6 | 21.6 | 21.5 |
| bspline3 | bounds | 26.7 | 26.7 | 26.6 | 25.7 | 26.7 |
| bspline7 | point_at | 37.3 | 37.3 | 37.3 | 37.3 | 37.3 |
| bspline7 | derivative | 43.4 | 43.4 | 43.3 | 43.3 | 43.4 |
| bspline7 | bounds | 29.9 | 29.9 | 30.0 | 28.9 | 29.9 |
| nurbs | point_at | 23.4 | 23.4 | 23.4 | 23.4 | 23.3 |
| nurbs | derivative | 53.7 | 53.7 | 53.7 | 53.7 | 53.7 |
| nurbs | bounds | 25.2 | 25.2 | 25.2 | 24.2 | 25.2 |

| curve | budget px | segments | ns/segment | allocs (max) | achieved error px |
|---|---|---|---|---|---|
| line | 0.1 | 1.0 | 127 | 1 | 0.0000 |
| line | 0.25 | 1.0 | 56 | 1 | 0.0000 |
| line | 0.5 | 1.0 | 56 | 1 | 0.0000 |
| line | 1 | 1.0 | 57 | 1 | 0.0000 |
| polyline | 0.1 | 15.0 | 2 | 1 | 0.0000 |
| polyline | 0.25 | 15.0 | 2 | 1 | 0.0000 |
| polyline | 0.5 | 15.0 | 2 | 1 | 0.0000 |
| polyline | 1 | 15.0 | 2 | 1 | 0.0000 |
| circle | 0.1 | 216.0 | 152 | 1 | 0.0748 |
| circle | 0.25 | 136.0 | 156 | 1 | 0.1887 |
| circle | 0.5 | 96.0 | 160 | 1 | 0.3786 |
| circle | 1 | 72.0 | 166 | 1 | 0.6730 |
| ellipse | 0.1 | 212.0 | 176 | 1 | 0.0933 |
| ellipse | 0.25 | 132.0 | 173 | 1 | 0.2321 |
| ellipse | 0.5 | 92.0 | 177 | 1 | 0.4650 |
| ellipse | 1 | 64.0 | 171 | 1 | 0.8860 |
| trimmed | 0.1 | 104.0 | 175 | 1 | 0.0754 |
| trimmed | 0.25 | 72.0 | 181 | 1 | 0.1574 |
| trimmed | 0.5 | 48.0 | 191 | 1 | 0.3541 |
| trimmed | 1 | 40.0 | 197 | 1 | 0.5100 |
| composite | 0.1 | 151.0 | 255 | 1 | 0.0781 |
| composite | 0.25 | 97.0 | 270 | 1 | 0.1929 |
| composite | 0.5 | 73.0 | 288 | 1 | 0.3176 |
| composite | 1 | 57.0 | 300 | 1 | 0.5360 |
| bezier | 0.1 | 104.0 | 332 | 1 | 0.0759 |
| bezier | 0.25 | 68.0 | 346 | 1 | 0.1954 |
| bezier | 0.5 | 48.0 | 361 | 1 | 0.3562 |
| bezier | 1 | 36.0 | 380 | 1 | 0.7806 |
| bspline3 | 0.1 | 255.0 | 291 | 1 | 0.0948 |
| bspline3 | 0.25 | 160.0 | 290 | 1 | 0.2352 |
| bspline3 | 0.5 | 112.0 | 297 | 1 | 0.4740 |
| bspline3 | 1 | 81.0 | 303 | 1 | 0.8972 |
| bspline7 | 0.1 | 234.0 | 501 | 1 | 0.0916 |
| bspline7 | 0.25 | 146.0 | 503 | 1 | 0.2325 |
| bspline7 | 0.5 | 108.0 | 516 | 1 | 0.4701 |
| bspline7 | 1 | 74.0 | 531 | 1 | 0.9141 |
| nurbs | 0.1 | 217.0 | 393 | 1 | 0.0945 |
| nurbs | 0.25 | 137.0 | 403 | 1 | 0.2344 |
| nurbs | 0.5 | 95.0 | 411 | 1 | 0.4688 |
| nurbs | 1 | 67.0 | 412 | 1 | 0.9232 |

## Evaluation cost, arm64

| curve | metric | identity | translation | uniform-scale | rotation-scale | chain-offset |
|---|---|---|---|---|---|---|
| line | point_at | 5.6 | 5.6 | 5.6 | 5.6 | 5.6 |
| line | derivative | 5.9 | 5.9 | 5.9 | 5.9 | 5.9 |
| line | bounds | 15.5 | 15.5 | 15.5 | 15.3 | 15.5 |
| polyline | point_at | 3.6 | 3.6 | 3.6 | 3.6 | 3.6 |
| polyline | derivative | 4.3 | 4.3 | 4.3 | 4.3 | 4.3 |
| polyline | bounds | 18.1 | 18.1 | 18.2 | 18.2 | 18.1 |
| circle | point_at | 20.9 | 20.9 | 20.9 | 20.7 | 20.9 |
| circle | derivative | 21.8 | 21.8 | 21.8 | 22.1 | 22.0 |
| circle | bounds | 76.3 | 76.4 | 76.4 | 112.2 | 78.5 |
| ellipse | point_at | 21.1 | 21.0 | 21.1 | 20.9 | 20.8 |
| ellipse | derivative | 21.9 | 21.8 | 21.8 | 21.8 | 21.8 |
| ellipse | bounds | 76.8 | 76.3 | 76.3 | 112.3 | 76.2 |
| trimmed | point_at | 19.7 | 19.8 | 19.6 | 19.7 | 19.6 |
| trimmed | derivative | 24.9 | 24.9 | 24.9 | 24.9 | 24.9 |
| trimmed | bounds | 58.7 | 58.8 | 59.0 | 94.2 | 58.9 |
| composite | point_at | 29.0 | 28.9 | 29.0 | 28.9 | 29.1 |
| composite | derivative | 43.2 | 43.2 | 43.1 | 43.2 | 43.2 |
| composite | bounds | 190.7 | 190.6 | 190.4 | 244.4 | 190.7 |
| bezier | point_at | 23.0 | 23.0 | 22.9 | 22.9 | 22.9 |
| bezier | derivative | 23.6 | 23.6 | 23.6 | 23.6 | 23.6 |
| bezier | bounds | 103.3 | 103.2 | 103.1 | 130.2 | 103.2 |
| bspline3 | point_at | 26.2 | 26.1 | 26.2 | 26.1 | 26.1 |
| bspline3 | derivative | 30.0 | 30.0 | 30.0 | 30.0 | 30.0 |
| bspline3 | bounds | 14.8 | 14.8 | 14.8 | 14.9 | 14.8 |
| bspline7 | point_at | 122.6 | 122.3 | 122.5 | 122.4 | 122.5 |
| bspline7 | derivative | 91.9 | 91.9 | 91.9 | 91.9 | 91.9 |
| bspline7 | bounds | 18.7 | 18.7 | 18.7 | 18.8 | 18.8 |
| nurbs | point_at | 46.9 | 46.9 | 47.0 | 47.0 | 46.9 |
| nurbs | derivative | 78.9 | 78.9 | 79.4 | 79.3 | 79.3 |
| nurbs | bounds | 13.2 | 13.2 | 13.2 | 13.2 | 13.2 |

| curve | budget px | segments | ns/segment | allocs (max) | achieved error px |
|---|---|---|---|---|---|
| line | 0.1 | 1.0 | 276 | 1 | 0.0000 |
| line | 0.25 | 1.0 | 141 | 1 | 0.0000 |
| line | 0.5 | 1.0 | 137 | 1 | 0.0000 |
| line | 1 | 1.0 | 140 | 1 | 0.0000 |
| polyline | 0.1 | 15.0 | 4 | 1 | 0.0000 |
| polyline | 0.25 | 15.0 | 4 | 1 | 0.0000 |
| polyline | 0.5 | 15.0 | 4 | 1 | 0.0000 |
| polyline | 1 | 15.0 | 4 | 1 | 0.0000 |
| circle | 0.1 | 216.0 | 168 | 1 | 0.0748 |
| circle | 0.25 | 136.0 | 173 | 1 | 0.1887 |
| circle | 0.5 | 96.0 | 178 | 1 | 0.3786 |
| circle | 1 | 72.0 | 186 | 1 | 0.6730 |
| ellipse | 0.1 | 212.0 | 202 | 1 | 0.0933 |
| ellipse | 0.25 | 132.0 | 199 | 1 | 0.2321 |
| ellipse | 0.5 | 92.0 | 202 | 1 | 0.4650 |
| ellipse | 1 | 64.0 | 193 | 1 | 0.8860 |
| trimmed | 0.1 | 104.0 | 222 | 1 | 0.0754 |
| trimmed | 0.25 | 72.0 | 234 | 1 | 0.1574 |
| trimmed | 0.5 | 48.0 | 255 | 1 | 0.3541 |
| trimmed | 1 | 40.0 | 273 | 1 | 0.5100 |
| composite | 0.1 | 151.0 | 290 | 1 | 0.0781 |
| composite | 0.25 | 97.0 | 306 | 1 | 0.1929 |
| composite | 0.5 | 73.0 | 334 | 1 | 0.3176 |
| composite | 1 | 57.0 | 345 | 1 | 0.5360 |
| bezier | 0.1 | 104.0 | 367 | 1 | 0.0759 |
| bezier | 0.25 | 68.0 | 381 | 1 | 0.1954 |
| bezier | 0.5 | 48.0 | 399 | 1 | 0.3562 |
| bezier | 1 | 36.0 | 422 | 1 | 0.7806 |
| bspline3 | 0.1 | 255.0 | 353 | 1 | 0.0948 |
| bspline3 | 0.25 | 160.0 | 353 | 1 | 0.2352 |
| bspline3 | 0.5 | 112.0 | 367 | 1 | 0.4740 |
| bspline3 | 1 | 81.0 | 397 | 1 | 0.8972 |
| bspline7 | 0.1 | 234.0 | 1020 | 1 | 0.0916 |
| bspline7 | 0.25 | 146.0 | 1028 | 1 | 0.2325 |
| bspline7 | 0.5 | 108.0 | 1064 | 1 | 0.4701 |
| bspline7 | 1 | 74.0 | 1111 | 1 | 0.9141 |
| nurbs | 0.1 | 217.0 | 506 | 1 | 0.0945 |
| nurbs | 0.25 | 137.0 | 523 | 1 | 0.2344 |
| nurbs | 0.5 | 95.0 | 521 | 1 | 0.4688 |
| nurbs | 1 | 67.0 | 534 | 1 | 0.9232 |

## Viewport-weighted summary

The architectural entry `architectural-1e5` has the mix recorded in the lock (73.0% lines, 16.7% polylines, 5.7% trimmed curves,
2.8% circles, 0.9% ellipses, 0.9% B-splines). Weighting the per-type costs by it gives the `summary` rows of the CSV: the weighted
`point_at` cost per entity and the weighted cost of one tessellation at 0.5 px. Lines and polylines, three quarters of the drawing,
tessellate in tens of nanoseconds per vertex; the conics and splines, a fifth of it, are where the microseconds go. The cost profile
that `libviprs/maplibre-pvec#1` can publish is the per-type tables above plus these two rows.

## What this does not do

It changes no evaluator and no subdivision strategy by itself (P8.6 did, in the commits `docs/bench/TUNING.md` lists), measures no GPU rasterisation (EPIC M) and
no STEP evaluation (P8.2), and adds no curve type. P1.3 in this document is the prototype baseline only.
