# Phase 1 fixture set

One corpus, shared by P1.4 (STEP-HDF), P1.5 (layouts), P1.6 (spatial index) and P1.7 (Tier 0), so
their tables can be read line by line against each other. Generator: `spikes/fixtures` (std only,
deterministic, splitmix64 seeded per fixture). **Corpus revision: `pvec-p1-fixtures-r1`.** A table
that cites a different revision is not comparable with this one.

The geometry is plain data in the vocabulary of `ISO_PROFILE.md` (line, circle, arc, ellipse arc,
polyline, clamped B-spline with optional weights, composite of lines and arcs). It carries no layer,
style or text, because those are independent components and sit outside the geometry oracle.
Coordinates of points are rounded to three decimals, like drawing units; angles, radii derived from
them, knots and weights keep their full mantissa.

`pvec-gold-fixtures` has no probe set yet (the repository holds its README only), so O1.5's
pathological probes and O1.6's transform and block probes are not part of this revision. When they
land they become revision r2 and every table is re-run against it.

| fixture | entities | extent | lines | circles | arcs | ellipses | polylines | splines | composites | content hash (FNV-1a) | Part 21 bytes |
|---|---|---|---|---|---|---|---|---|---|---|---|
| small-synthetic | 1,000 | 6000 x 6000 | 594 | 41 | 66 | 34 | 215 | 32 | 18 | `4d58cf5bcaa5f9ed` | 315,128 |
| spline-heavy | 50,000 | 20000 x 20000 | 12,490 | 481 | 1,470 | 484 | 4,889 | 29,201 | 985 | `c04919188aa6370a` | 89,798,481 |
| million | 1,000,000 | 100000 x 100000 | 599,431 | 40,151 | 79,920 | 30,033 | 200,221 | 30,141 | 20,103 | `79a8dfecc088fc8a` | 365,814,152 |
| straddler | 100,000 | 32000 x 32000 | 60,163 | 3,968 | 7,942 | 2,947 | 19,878 | 3,064 | 2,038 | `e72e4e20546834b2` | 35,306,923 |

Produced on MARS (native x64, glibc, `rustc 1.99.1`) with `tools/spike.sh fixtures 'cargo run -q
--release --bin fixture-info -- --p21 /tmp/p21'`. The hash covers every entity's id, kind and
float bit patterns. It depends on the platform's `sin` and `cos`, so it is only guaranteed on the
same libm; a different hash on another platform means different arcs, not a bug.

* **small-synthetic**: 3 x 3 rooms, the plan-like mix (60% lines, 20% polylines, 8% arcs).
* **spline-heavy**: 10 x 10 rooms, 58% splines of degree 2 to 5 with 6 to 60 control points, a
  quarter of them rational.
* **million**: 50 x 50 rooms, 400 entities each, the plan-like mix. This is the 10^6 fixture.
* **straddler**: 16 x 16 rooms plus four entities that each span the whole extent (a full-width line,
  a full-height line, a diagonal and the circumscribing circle). For P1.6's worst-case straddler.

Entities are numbered in generation order (room by room), which has the spatial locality of a drawing
exported in drawing order, not of a spatially sorted file.

## Part 21 baseline

`fixtures::p21::write_part21` writes each drawing with exactly the entity forms of the profile and
one `GEOMETRIC_CURVE_SET` over all curves; the byte counts above are the baseline for every "vs Part
21" ratio. It omits the AP242 product and context boilerplate (a few kilobytes, identical for every
layout). No independent STEP reader parsed it in this spike; round-trip validation is P2.2 and P6.3.

## The viewport script

`viewport_script()` is committed in code and replays identically against every candidate: cold open
(whole drawing), zoom in to 1/4 x 1/4, pan right four times by 1/8 of the extent, zoom in to 1/16 x
1/16, zoom out to 1/4 x 1/4, zoom out to the whole drawing. Rectangles are fractions of the extent,
so the same script applies to every fixture, in source space.
