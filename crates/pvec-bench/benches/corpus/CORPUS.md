# The phase 8 benchmark corpus

Phase 8 numbers mean nothing without a fixed input, so this is the one every later task (P8.2 to P8.7) measures on. The
lock is `corpus.lock.json`, the viewport set is `viewports.csv`, the generator is `../gen_corpus.rs` with the library in
`../../src`, and the test that holds all of it is `../../tests/issue_60_corpus.rs`. The lock and the viewport file are
written by the generator (`cargo bench -p pvec-bench --bench gen_corpus -- write`), never by hand.

## What is pinned

| | |
|---|---|
| fixture repository | `libviprs/pvec-gold-fixtures`, the EPIC O corpus (`libviprs/pvec-gold-fixtures#1`) |
| tag and commit | `corpus-r0-open`, `f94f7f22b7aaff36eaaf6d5501fafc42054f22f2` |
| bundle sha256 | `901370036bbffbbb8cb30471c2bf90f171b4f966c5bdfea6b140591cd840ca50` (the vendored `tests/corpus/r0-open.tar`, see `docs/CORPUS.md`) |
| generator | `pvec-bench-corpus/1` (change it when a model changes, so a stale lock cannot match) |
| entries | 32 in the lock (22 known-truth, 10 generated), 16 exclusions |

Nothing in the corpus is stored as a `.pvec`. An entry is either one of the `r0-open` fixtures run through the P2.4 canonical
generator, or a synthetic drawing from a model and a seed, and `pvec_bench::build(name)` rebuilds any of them from the
vendored bundle and the seeds alone. That is also the answer to the converter question: there is no proprietary converter image
anywhere in this path, no network and no process (a test scans the sources for `Command::new`, sockets and the word docker).
`tools/corpus-offline.sh` runs the rebuild inside a plain Rust image with `--network none`, which is the check that stands in
for a hosted job while hosted CI is off.

## The seven classes

Entity counts are by registry capability, as the lock records them. A full circle counts as `circle`, a partial one as
`trimmed_curve`, and the same for the ellipse.

### synthetic-known-truth

The 22 `r0-open` fixtures the P2.4 generator can hold exactly, each a handful of entities: `arc-seam-001`, `bezier-d3-001`,
`bspline-d3-001`, `bspline-d5-001`, `bulge-arc-001`, `bulge-arc-neg-001`, `circle-001`, `closed-by-endpoint-001`,
`closed-periodic-001`, `ellipse-001`, `elliptical-arc-001`, `large-coords-001`, `layer-state-001`, `line-001`,
`lwpolyline-closed-001`, `nonunit-weight-001`, `nurbs-d2-offconic-001`, `nurbs-d3-001`, `polyline-001`, `polyline2d-001`,
`repeated-knot-c0-001`, `tiny-radius-001`. Together they hold 26 entities in 16,928 bytes. Provenance is the vendored bundle,
every DWG in it is `PUBLIC_REDISTRIBUTABLE` in the manifest, and the evidence class of each is `KNOWN_TRUTH_EXACT`, carried
forward from the registry unchanged. These are correctness inputs. Their timings sit at the noise floor, so a benchmark can
replay their viewports but should not read a throughput from them.

### pathological

`pathological-1e3`: 1,000 entities, seed `0x0b01`. Arcs of radius 10^6 with a 10^-9 radian sweep, circles of radius 10^-6, a
sweep 10^-12 short of a full turn, cubics whose interior knot has multiplicity 3 (C0), splines with coincident control
points, polylines that revisit a vertex, segments 10^-9 long, repeated identical segments, three segments through one point,
and four splines of 10,000 control points (three cubics and a rational degree 7). Nothing is rounded, so the awkward values
survive. Counts: 498 line, 83 polyline, 83 circle, 166 trimmed curve, 169 B-spline, 1 rational B-spline.

The production writer refuses two kinds of pathological input outright, a zero-length segment and a polyline with two
coincident consecutive vertices (`HOT-25`, NP-06 `DegenerateLength`). They are not in the class because they cannot be
written, and the lock lists both under the exclusions with the writer's own message. The generator tries them on every
`write`, so the list cannot go stale.

### architectural

A plan on a grid of 8 m rooms (millimetres), 400 drawing groups per room: hatch lines (the bulk), furniture rectangles, wall
runs, doors (an arc and a leaf), windows, columns, curved walls, dimension lines, stair treads, a few splines and ellipses,
and a long structural axis per row and per column (a tenth of the entities at most) so that straddlers exist. The mix at 10^6
is 72.8% line, 16.9% polyline, 5.6% trimmed curve, 2.8% circle, 0.9% B-spline, 0.9% ellipse. This is a modelled mix, not a
measured one, and the evidence class says so (see below).

| entry | entities | bytes | seed |
|---|---|---|---|
| `architectural-1e3` | 1,000 | 55,640 | `0xa103` |
| `architectural-1e5` | 100,000 | 5,492,896 | `0xa105` |
| `architectural-1e6` | 1,000,000 | 55,052,464 | `0xa106` |
| `architectural-1e7` | 10,000,000 | 550,378,792 | `0xa107` |

### mep

`mep-1e5`: 100,000 entities, seed `0x3e05`, 6,756,960 bytes. Duct and pipe runs (Manhattan polylines and lines), fittings and
risers (circles), elbows (arcs), flexible duct (splines), oval sections (ellipses), hangers, and three long mains per row and
column. 46.4% line, 24.1% polyline, 15.8% circle, 8.0% trimmed curve, 3.3% B-spline, 2.3% ellipse.

### industrial

`industrial-1e5`: 100,000 entities, seed `0x1d05`, 6,731,160 bytes. Pipe racks (long runs), steel, vessels with elliptical heads,
instrument bubbles, elbows, tank outlines and cable splines. 35.1% line, 23.3% polyline, 19.9% trimmed curve (elbows and the
heads), 15.0% circle, 3.4% ellipse, 3.3% B-spline.

### huge-coordinate

`huge-coordinate-1e5`: 100,000 entities, seed `0x4c05`, 9,375,352 bytes, declared in metres. The origin sits at
(6,480,000.125, 5,900,000.25), so every |x| is above 10^6, and the detail is a millimetre (coordinates are rounded to 0.001,
and some segments are 0.003 by 0.002 long). It is a site plan: lines, closed boundaries, tree circles, curb arcs, road splines.
Doubles hold this with room to spare (the spacing at 6.5 million is about 10^-9), so the class is about what the transform
chain does to the numbers and not about the container.

### spline-heavy

`spline-heavy-1e5` and `spline-heavy-1e6` (seeds `0x5b05`, `0x5b06`; 24,413,992 and 244,437,392 bytes). 60% of the drawing
groups are one B-spline of degree 2 to 7 over 4 to 30 control points, 15% of those rational with weights between 0.5 and 2. At
10^6 that is 510,449 B-splines and 89,995 rational ones, 149,913 lines, 100,006 polylines, 99,722 trimmed curves and 49,915
ellipses. This is the class that stresses the advanced-geometry arenas.

## The scale ladder

The architectural plan is the ladder: nominal 10^3, 10^5, 10^6 and 10^7 entities, one model, four seeds, so a result can be
compared across tasks and across sizes. The 10^7 case is `architectural-1e7`. It is generated on demand and never stored or
rebuilt by the default tests (`on_demand_only` in the lock). Its digest, size and cost come from `measured.csv`, one row per
platform, written by `gen_corpus -- measure architectural-1e7`:

| platform | host | generation time | peak memory (VmHWM) | sha256 |
|---|---|---|---|---|
| linux-x86_64 (native) | MARS | 19.9 s | 5,224,920 KiB | `feccefd6...a4c4b` |
| linux-aarch64 (native) | mercury (Colima VM, 6 cpus) | 60.5 s | 5,226,140 KiB | `feccefd6...a4c4b` |

The artifact is 550,378,792 bytes. Peak memory is the whole shape list plus the writer's buffers, since the writer holds the
artifact in memory, so it is dominated by the output size and not by the generator. The two hosts are different machines, so the
times are not a comparison of the architectures.

## Bit for bit, on both platforms

Every synthetic entry reproduces bit for bit from its seed on arm64 and on native x86_64.

The generator draws numbers with splitmix64 and basic arithmetic and `round` only. No sine, cosine or square root is called,
angles are whole degrees times one constant, and bounds are computed from the control hull and the circle's box, so the C
library cannot change a bit. I checked that claim on three machines. The default test rebuilds every entry up to 10^5 and
compares the digest in the lock:

| platform | what ran | result |
|---|---|---|
| macOS arm64 (the MacBook, native Apple silicon) | `cargo test -p pvec-bench` and the ignored 10^6 test, release | pass |
| linux arm64 (mercury, native) | the 10^7 measure, plus `architectural-1e5` | same digests as x86_64 |
| native x86_64 (MARS) | `cargo test -p pvec-bench` and the ignored 10^6 test, release, `--network none` | pass |

The pass counts are in the comment on the pull request that carries this file.

## Evidence classes

An entry carries its EPIC O evidence class unchanged and nothing here promotes one. The 22 known-truth entries are
`KNOWN_TRUTH_EXACT` as the registry loads them (scope `READER_MAPPING_ONLY`, no oracle), which means two open DWG readers agree
with the authored mathematics and nothing about STEP. The ten generated entries have `evidence_class: null`: they were made by
this repository, they are not EPIC O fixtures, and a class would claim an independent source they do not have. No entry is
`REAL_WORLD_DIFFERENTIAL` or `DUAL_ORACLE_EQUIVALENT`, because the fixture repository has none yet (`pvec#100`,
`pvec-gold-fixtures#42`). When one arrives it enters with its class and is never promoted to source truth here.

The consequence for the three drawing classes is worth saying plainly: architectural, MEP and industrial are modelled mixes,
not real drawings. Their entity mixes are my assumptions, written down above and in the lock, and a later task that wants real
drawings has to take them from EPIC O when they exist. They are listed as exclusions (`architectural-real-world`,
`mep-real-world`, `industrial-real-world`, kind `REAL_WORLD_ABSENT`) so the gap is on the record.

## Exclusions

Every one of these is out of every performance baseline, and none is also an entry (a test holds that). The reasons are in the
lock.

| name | kind | why |
|---|---|---|
| `general-affine-001`, `shear-001`, `singular-001` | QUARANTINED | the corpus could not write the probe, so there is no file and no record |
| `near-singular-001` | NOT_EMITTED | the manifest says no curve may be emitted for it |
| `nested-blocks-001`, `nonuniform-scale-circle-001`, `nonuniform-scale-rotated-001`, `reflection-001`, `rotation-irrational-001`, `translation-001`, `uniform-scale-001` | MISSING_CAPABILITY | the container stores no instance transform (`affine_transform_2d`, pvec#120), so the generator refuses rather than emit the world-space result |
| `pathological-zero-length-line`, `pathological-coincident-vertices` | WRITER_REFUSAL | `HOT-25` / NP-06, the writer rejects degenerate length |
| `architectural-real-world`, `mep-real-world`, `industrial-real-world` | REAL_WORLD_ABSENT | no real-world entry exists in the fixture repository yet |

## Licence and redistribution

The known-truth entries are `PUBLIC_REDISTRIBUTABLE` in the `r0-open` manifest. The generated entries are produced by this
repository under its own licence (Apache-2.0 OR MIT) from a seed, and no byte of them is redistributed: the seed is the
distribution. Each entry's lock row repeats both statements.

## Viewports

`viewports.csv` records 200 viewports per entry (32 entries, 6,400 rows, columns `entry,index,x0,y0,x1,y1`). The first is
the whole drawing, for a cold open, and the rest cycle through half, a quarter, an eighth and a thirty-second of each side at
positions drawn from a stream seeded by the entry name. They are rounded to three decimals so the CSV round-trips exactly,
and a test recomputes the whole file from the bounds in the lock, so a later task's claim to replay the same viewport is
checkable by comparing against this file. P8.2 and P8.5 read it as is.

## Running it

```text
cargo bench -p pvec-bench --bench gen_corpus -- write              # rewrite the lock and the viewports
cargo bench -p pvec-bench --bench gen_corpus -- check              # rebuild every stored entry, compare digests
cargo bench -p pvec-bench --bench gen_corpus -- measure NAME       # one measured.csv row (Linux for peak memory)
cargo test -p pvec-bench --release -- --ignored                    # the 10^6 digests
tools/corpus-offline.sh                                            # the same, in a plain image with no network
```

## What this does not do

It runs no benchmark (P8.2 to P8.5), tunes nothing (P8.6), authors no DWG and runs no converter (EPIC O), and decides no
correctness oracle (phase 2).
