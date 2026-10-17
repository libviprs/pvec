# Spike P1.6: spatial index and payload ownership models

Code: `spikes/spatial-index/` (throwaway). Driver: `tools/spike.sh spatial-index '/tmp/target/release/measure <fixture>'`.
Raw output of every run quoted here is in `docs/spikes/01-6-data/`. Fixtures: corpus revision
**`pvec-p1-fixtures-r1`** (`00-fixtures.md`), the same four drawings as P1.4 and P1.5, plus one stress
case that I generated inside the spike and that is **not part of the corpus** (see "Stress case").

**Every number is native x64**: MARS (Intel Core i7-14700F), `rustc 1.99.1`, `--release`, one thread, in a
`linux/amd64` container. Nothing was run on arm64. MARS was at a load average of 220 to 335 during the
runs (recorded at the head of every data file), so timings are CPU time, and I would not trust a
timing difference below a factor of about two.

## What this is, and what it is not

It is a **simulation of I/O**, not a file format. Each model assigns every entity a payload offset and
length, builds its index in memory, states how many bytes that index would occupy on disk, and for a
viewport lists the byte ranges a remote reader would fetch (index records first, then payload). Ranges are
merged the way P1.4 merges them: reads at most 64 KiB apart become one request, and I give the
no-merge numbers beside them wherever the difference matters. Nothing is written to disk and no HTTP
request is made. The viewport script is the committed `viewport_script()`, in source space throughout.

**Correctness gate.** Every query result is checked against brute force: all entities whose bounds overlap
the viewport must be among the fetched ones, for the nine script steps plus four extra rectangles (poking
outside the world, hugging an edge, a single point, entirely outside). The measurement binary asserts it
for every model, fixture and step, and `tests/models.rs` does so on the small fixture and on a fixture with
full-extent entities added. A model that lost a visible entity cannot produce a number.

**Payload byte model.** A payload byte count per entity, calibrated to layout B's measured bytes per entity
on `million` (`01-5-layouts.md`): 8 bytes per `f64`, 5 bytes of id and framing, and for splines and
composites B's record header and padding. Against B's measured per-kind numbers it is within 5% for
every kind on `million` and `straddler`:

| fixture | kind | model bytes/entity | layout B on million | difference |
|---|---|---|---|---|
| straddler | line | 37.0 | 36.7 | +0.8% |
| straddler | circle | 29.0 | 28.7 | +1.0% |
| straddler | arc | 45.0 | 44.7 | +0.7% |
| straddler | ellipse | 61.0 | 60.7 | +0.5% |
| straddler | polyline | 125.3 | 128.7 | -2.7% |
| straddler | spline | 241.3 | 237.9 | +1.4% |
| straddler | composite | 221.7 | 212.3 | +4.4% |
| million | line | 37.0 | 36.7 | +0.8% |
| million | circle | 29.0 | 28.7 | +1.0% |
| million | arc | 45.0 | 44.7 | +0.7% |
| million | ellipse | 61.0 | 60.7 | +0.5% |
| million | polyline | 124.9 | 128.7 | -2.9% |
| million | spline | 240.2 | 237.9 | +1.0% |
| million | composite | 220.6 | 212.3 | +3.9% |

so the absolute byte counts below are the bytes of a layout like B, and the comparison between models is
not affected by that choice: all models use the same payload sizes.

**Index record sizes** are my own and stated per model below; they are the main thing a real design could
shrink. Bounds are stored as `f32` on disk in the size model but compared as `f64` in the simulation, which
makes the false-positive numbers slightly optimistic.

## The candidates

Source-space bounds throughout; nothing depends on a display zoom.

* **M1, loose quadtree (ancestor ownership).** An entity is stored once, in the node whose cell is the
  smallest that is still at least as large as the entity, found from the entity's centre. Node bounds are
  loose (the cell grown by half a cell each side) so an entity that sticks out of its cell is still inside
  its node's bounds. The unit of I/O is a node's whole payload; the index record is 32 bytes per node;
  depth capped at level 10.
* **M2, packed Hilbert R-tree.** Entities sorted by Hilbert key of their bounds centre, cut into leaves of
  `C` entities, internal levels of fanout `F`; every entity stored once; 32 bytes per entry. The unit of I/O
  is a leaf's payload. Two settings: `C`=512 / `F`=64 (the block size P1.5 used) and `C`=64 / `F`=32.
* **M3, hierarchical cells with duplicated compact references.** The payload is stored once, in Hilbert
  order. An entity goes to the level whose cell is at least as large as the entity, so it overlaps at most
  2 x 2 cells, and a reference to it is written in each of them (duplication of at most 4). A directory of
  non-empty cells (16 bytes each) is followed by the reference lists. Two settings: a 12-byte reference
  (payload offset, length, id) and a 28-byte reference that also carries the entity's bounds, so the reader
  can drop non-visible entities before fetching any payload.
* **M4, leaf cells plus long-entity buckets.** An entity no longer than a leaf cell is owned by the leaf
  cell holding its centre (cell bounds loose by half a cell). A longer entity goes to the bucket of its size
  class, `ceil(log2(side / cell))`, sorted by Hilbert key and cut into chunks of 64 with a bounds table that
  is always read, so a long entity never inflates a leaf and is fetched only when its chunk touches the
  viewport. Two settings: leaf level 7 and 8 (cells of 1/128 and 1/256 of the world side).
* **Excluded: a candidate keyed on display zoom** (a tile pyramid in the style of MVT, where an entity is
  assigned to the zoom levels at which it is drawn). I report it as such and exclude it without a
  measurement: the issue requires source-space bounds throughout, a zoom-keyed assignment makes payload
  placement depend on a display scale that P5.5's source-space queries do not have, and a viewport at an
  intermediate zoom cannot be answered without choosing one.

## Results

### A. Index size, duplication and build time

| model | million index bytes (x payload) | straddler index bytes (x payload) | duplication (million) | build cpu ms (million) |
|---|---|---|---|---|
| M1 loose quadtree | 23,640,000 (0.3611) | 3,071,360 (0.4683) | 1.000 | 280 |
| M2 R-tree 512/64 | 63,616 (0.0010) | 6,496 (0.0010) | 1.000 | 50 |
| M2 R-tree 64/32 | 516,256 (0.0079) | 51,744 (0.0079) | 1.000 | 50 |
| M3 refs 12 B | 39,221,080 (0.5992) | 4,622,640 (0.7048) | 1.996 | 457 |
| M3 refs 28 B (bounds) | 71,156,984 (1.0870) | 8,050,160 (1.2273) | 1.996 | 475 |
| M4 buckets, leaf 7 | 265,712 (0.0041) | 252,656 (0.0385) | 1.000 | 42 |
| M4 buckets, leaf 8 | 1,091,792 (0.0167) | 645,808 (0.0985) | 1.000 | 56 |

Reference duplication is a factor per candidate, as the issue asks: 1.000 for M1, M2 and M4 (every entity
is stored and referenced once), and **1.996 on `million`** (2.00 to 2.32 across the fixtures, never above the
bound of 4) for M3. No candidate is rejected below for duplication alone.

### B. The whole viewport script, bytes / ranges

Nine steps, including the cold open and the final zoom-out, which both show the whole drawing and so read
everything.

| model | small-synthetic | spline-heavy | million | straddler | stress-long |
|---|---|---|---|---|---|
| M1 loose quadtree | 996,757 / 9 | 81,444,891 / 111 | 224,653,520 / 279 | 29,187,134 / 94 | 86,877,134 / 94 |
| M2 R-tree 512/64 | 586,152 / 9 | 86,213,900 / 40 | 162,172,460 / 82 | 18,527,416 / 46 | 45,975,584 / 57 |
| M2 R-tree 64/32 | 538,080 / 9 | 73,313,126 / 64 | 161,029,202 / 97 | 17,024,794 / 51 | 41,930,060 / 62 |
| M3 refs 12 B | 1,007,912 / 9 | 83,590,414 / 174 | 258,162,166 / 282 | 31,523,868 / 171 | 91,296,383 / 204 |
| M3 refs 28 B (bounds) | 1,318,735 / 9 | 83,445,462 / 186 | 332,846,269 / 300 | 39,161,988 / 182 | 63,505,499 / 188 |
| M4 buckets, leaf 7 | 616,568 / 9 | 79,084,206 / 117 | 162,702,029 / 147 | 19,713,356 / 65 | 44,349,504 / 72 |
| M4 buckets, leaf 8 | 591,784 / 9 | 79,402,307 / 128 | 163,683,825 / 156 | 19,275,067 / 89 | 43,911,215 / 96 |

### C. The interactive steps only

The same script without the two whole-drawing views, which cost the same for any model that stores the
payload once (and are dominated by the payload). Bold is the smallest bytes in the column.

| model | small-synthetic | spline-heavy | million | straddler | stress-long |
|---|---|---|---|---|---|
| M1 loose quadtree | 0.76 / 7 | 23.12 / 109 | 46.45 / 277 | 9.93 / 92 | 54.80 / 92 |
| M2 R-tree 512/64 | 0.46 / 7 | 29.66 / 38 | 31.12 / 80 | 5.40 / 44 | 20.02 / 55 |
| M2 R-tree 64/32 | **0.41 / 7** | **16.71 / 62** | **29.08 / 95** | **3.80 / 49** | **15.89 / 60** |
| M3 refs 12 B | 0.77 / 7 | 23.10 / 172 | 48.80 / 280 | 9.16 / 169 | 56.06 / 202 |
| M3 refs 28 B (bounds) | 1.01 / 7 | 19.25 / 184 | 59.61 / 298 | 9.94 / 180 | 21.35 / 186 |
| M4 buckets, leaf 7 | 0.48 / 7 | 22.19 / 115 | 31.25 / 145 | 6.09 / 63 | 17.90 / 70 |
| M4 buckets, leaf 8 | 0.46 / 7 | 22.51 / 126 | 30.58 / 154 | 4.87 / 87 | 16.68 / 94 |

### D. The 1/16 x 1/16 viewport

| model | small-synthetic (7 visible) | spline-heavy (297 visible) | million (4031 visible) | straddler (443 visible) | stress-long (593 visible) |
|---|---|---|---|---|---|
| M1 loose quadtree | 108,714 / 1, fp 0.788 | 1,204,783 / 8, fp 0.624 | 1,172,806 / 18, fp 0.127 | 290,988 / 11, fp 0.318 | 6,700,988 / 11, fp 0.776 |
| M2 R-tree 512/64 | 65,128 / 1, fp 0.986 | 1,459,530 / 3, fp 0.884 | 656,620 / 7, fp 0.537 | 336,098 / 4, fp 0.892 | 1,454,490 / 5, fp 0.895 |
| M2 R-tree 64/32 | 54,914 / 1, fp 0.964 | 689,526 / 5, fp 0.710 | 522,114 / 8, fp 0.300 | 153,432 / 5, fp 0.636 | 795,452 / 6, fp 0.597 |
| M3 refs 12 B | 105,796 / 1, fp 0.588 | 1,185,722 / 25, fp 0.535 | 1,226,898 / 28, fp 0.086 | 221,304 / 16, fp 0.133 | 6,936,889 / 22, fp 0.764 |
| M3 refs 28 B (bounds) | 140,458 / 1, fp 0.000 | 594,142 / 22, fp 0.000 | 1,212,249 / 33, fp 0.000 | 245,305 / 16, fp 0.000 | 814,964 / 17, fp 0.000 |
| M4 buckets, leaf 7 | 67,592 / 1, fp 0.974 | 1,018,913 / 9, fp 0.699 | 720,247 / 9, fp 0.391 | 370,056 / 6, fp 0.653 | 1,178,708 / 7, fp 0.612 |
| M4 buckets, leaf 8 | 65,608 / 1, fp 0.982 | 1,089,263 / 10, fp 0.737 | 766,216 / 13, fp 0.294 | 342,333 / 6, fp 0.708 | 1,150,985 / 7, fp 0.665 |

### E. Query CPU time on `million`

| model | cold open | zoom in 1/4 | zoom in 1/16 |
|---|---|---|---|
| M1 loose quadtree | 18483 | 1358 | 132 |
| M2 R-tree 512/64 | 599 | 48 | 5 |
| M2 R-tree 64/32 | 642 | 48 | 6 |
| M3 refs 12 B | 104844 | 7652 | 472 |
| M3 refs 28 B (bounds) | 108020 | 7573 | 484 |
| M4 buckets, leaf 7 | 1368 | 127 | 16 |
| M4 buckets, leaf 8 | 4087 | 323 | 64 |

(Cold open touches every cell, so M3's walk over up to 4^10 cell coordinates per level is what dominates
its 105 to 108 ms; the others walk a tree or a sorted array.)

### Worst-case units, and the full-extent entity

Per candidate, the biggest single unit a query can be forced to read, and the unit that homes the longest
entity. On `straddler` the longest entity is a full-width line (side 32,000 of a 33,396 world) among the
four full-extent entities of that fixture. The numbers are from the "Worst-case units" lists in the data
files.

| candidate | `straddler` unit holding the longest entity | what it costs |
|---|---|---|
| M1 loose quadtree | root node (the four full-extent entities) | 140 bytes of payload, read by every query |
| M2 R-tree 512/64 | a leaf whose bounds are 0.958 of the world side | 35,390 bytes of payload, read by every query that touches the world |
| M2 R-tree 64/32 | the same leaf, at the smaller size | 4,806 bytes |
| M3 refs 12 B / 28 B | level-0 cell | reference list of 48 / 112 bytes, the payload itself is fetched only if referenced (and, with bounds, only if visible) |
| M4 buckets, leaf 7 / 8 | the size-class bucket | 140 bytes of payload, plus a chunk table of 10,336 / 18,336 bytes read by every query |

On the corpus fixtures the full-extent entities are cheap kinds, so the cost in bytes is small for every
candidate; the effect that matters is how a model's *structure* reacts to them, which the stress case
isolates.

### Stress case: expensive long entities

Corpus revision r1 contains no *expensive* long entity, so I added one for this spike only and I say so
wherever it appears. `stress-long` is `straddler` plus 2,000 polylines of 200 vertices each that run the
full width of the extent with a small random wobble (3,205 bytes each, 6.41 MB in total, 49% of the
payload). It is a stand-in for gridlines, contour lines or long cable runs, which a real drawing has. It is
not data: it is generated by `with_long_polylines` and would be a new corpus entry (r2) if it were kept.

* **M1 pays for it on every query.** All 2,000 land in the root node (side about equal to the world): root
  payload 6,410,140 bytes. A 1/16 viewport showing 593 entities reads **6.70 MB** under M1, 8.4 times
  what M2 64/32 reads (0.80 MB). This is the root congestion the epic's finding describes, measured.
  `tests/models.rs` checks it directly: a 10 x 10 viewport in a corner fetches at least the whole root.
* **M2 degrades gracefully**: the long polylines enlarge a few leaves to 0.958 of the world side, but at leaf 64
  the 1/16 view still reads 0.80 MB (fp 0.60).
* **M3 with bounds handles it**: 0.81 MB, no false positives. M3 without bounds does not (6.94 MB): a
  reference to a long entity sits in the coarse cell every viewport touches, and without the bounds the
  reader must fetch the payload to find out it is not visible.
* **M4 handles it by construction**: 1.15 to 1.18 MB.

Over the interactive steps: M2 64/32 15.89 MB, M4 leaf 8 16.68 MB, M4 leaf 7 17.90 MB, M2 512/64 20.02 MB,
M3 with bounds 21.35 MB, M1 54.80 MB, M3 without bounds 56.06 MB.

## Reading the results

* **Total viewport I/O decides, and it separates the models into two groups.** Over the interactive
  steps M1 and M3 read 1.4 to 3.5 times as much as M2 64/32 on `spline-heavy`, `million`, `straddler` and
  `stress-long`, with two exceptions: M3 with bounds is only 15.2% above on `spline-heavy` and 34% above
  on `stress-long`. M4 is 5.0% to 33% above M2 64/32.
* **M2 with small leaves wins every column of table C.** The next model reads more by: 12.0% on
  `small-synthetic` (M2 512/64), 15.2% on `spline-heavy` (M3 with bounds), 5.2% on `million` (M4 leaf 8),
  27.9% on `straddler` (M4 leaf 8), 5.0% on `stress-long` (M4 leaf 8). Over the whole script (table B), where
  two of the nine steps read the whole drawing and so dilute every difference, M2 64/32 is still smallest on
  every fixture, and the next model reads more by 8.9%, 7.9%, 0.7%, 8.8% and 4.7% in the same column order
  (M2 512/64, M4 leaf 7, M2 512/64, M2 512/64, M4 leaf 8). The 0.7% on `million` I would not call decisive.
* **Leaf size matters more than model family.** M2 512/64 is the worst single-ownership model on
  `spline-heavy` (86.2 MB against 73.3 MB for 64/32), because its largest leaf holds 320,902 bytes of payload.
  The setting that wins is in entities per leaf here, but the quantity that should be tuned is **leaf bytes**
  (a leaf of about 16 to 64 KB), because the payload size per entity varies 8-fold between a line and a
  spline. I tested only entity counts, so that is a hypothesis for P5.1, not a result.
* **Ranges.** M2 512/64 needs the fewest ranges (82 on `million` against 97 for 64/32, 147 for M4 leaf 7,
  282 to 300 for M3); M3 needs the most, and its no-merge count is far worse (14,740 reads on `million`
  against 298 for M2 64/32), which matters if the transport does not coalesce.
* **Why M3 loses, by I/O and not by duplication.** On `million` its index is 39.2 MB (12-byte references,
  duplication 2.0) or 71.2 MB (28-byte), against 0.5 MB for M2 64/32. A viewport reads only the cells it
  covers, but the cold open and the final zoom-out read the entire index on top of the payload (104.7 MB and
  136.6 MB for a drawing whose payload is 65.5 MB), and the small viewports pay for many cells that each
  hold a handful of references. The with-bounds variant drops every false positive and still reads more:
  1.21 MB for the 4,031-entity viewport against 0.52 MB. So M3's cost is the reference *bytes and count*,
  not the duplicate payload (there is none by construction). A leaner reference (4 bytes and an entity
  offset table) would shrink the index, and I did not build it; it is the one design change that could
  move M3 up, and the reversal threshold below names it.
* **Index size.** M2 needs 63,616 bytes (0.1% of the payload) at 512/64 and 516,256 (0.8%) at 64/32 on
  `million`; M4 265,712 (0.4%) at leaf 7; M1 23.6 MB (36%).
* **Build and query time.** Building takes 42 to 56 ms for M2 and M4, 280 ms for M1, 457 to 475 ms for M3
  (CPU, one million entities). Queries take 5 to 48 microseconds for M2 on the 1/4 and 1/16 viewports, 16 to 323
  for M4, 132 to 1,358 for M1 and 472 to 7,652 for M3. None of it is a problem; M3's cold open is the
  outlier.

## Decision

**Selected: M2, the packed Hilbert R-tree with single payload ownership**, leaf capacity in the 64-entity
region (tune it by leaf bytes in P5.1; I measured 64 and 512 entities).

* **Margin over the runner-up.** The runner-up is **M4** (leaf cells plus long-entity buckets). Over the
  interactive steps its best setting reads more than M2 64/32 by 13.0% on `small-synthetic`, 32.8% on
  `spline-heavy`, 5.2% on `million`, 27.9% on `straddler` and 5.0% on `stress-long`; over the whole script
  by 10.0%, 7.9%, 1.0%, 13.2% and 4.7%. M2 also needs fewer ranges than M4 (97 against 147 on `million`) and
  a smaller index than M4 leaf 8 (516 KB against 1.09 MB), though a larger one than M4 leaf 7 (266 KB).
* **Payload ownership.** Store the payload once, owned by exactly one leaf. On these fixtures bounded
  duplication of references (M3) buys nothing: it never wins a column, and it costs 0.6 to 1.2 times the
  payload in index. Long entities are handled acceptably by M2 alone (1.45 MB for the stress 1/16 view at
  leaf 512, 0.80 MB at leaf 64), but M4's bucket idea is the right safety valve and is cheap to keep.
* **Not decided here.** The block assignment P1.5 used (Morton order, flat directory) is a degenerate
  M2: sorted blocks and a directory with no tree. Its script total on `million` was 166.1 MB against 162.2 MB for
  M2 512/64 here, which is consistent given the different byte models; I did not run its exact bytes
  through these models.

**The threshold that would reverse it.** Switch to M4 if, on the P8.3 benchmark corpus, M4's bytes over the
interactive viewport steps come within 5% of M2's while M4 needs fewer ranges, or if more than 1% of leaves
in a real drawing have bounds wider than 25% of the world side (the failure that M4's buckets exist to
isolate; on `stress-long` the widest leaf is 0.958 and M2 still wins, so I do not expect this on the
fixtures). Switch to M3 only if a reference of 4 bytes plus an entity-offset table brings its interactive
bytes to within 5% of M2's with fewer than 2 times its ranges; I did not build that variant. Re-run all of it
if the corpus revision changes, because every margin above is a few percent.

## What I did not measure

* arm64. Nothing was run there.
* A real transfer, or any real file. Bytes and ranges come from a model of record sizes and payload
  sizes. HTTP overhead per request, TLS, and caching are not modelled; only merging at a 64 KiB gap and
  at no gap.
* Leaf size by bytes, block sizes other than 64 and 512 entities, quadtree depth other than 10, cell levels
  other than 10 (M3) and 7 and 8 (M4), and fanouts other than 32 and 64.
* Dynamic updates. All structures are built once.
* The leaner M3 reference noted above.
* Exact-geometry visibility: "visible" means bounds overlap, the convention of P1.4 and P1.5. A diagonal
  line whose bounds cover a viewport it does not cross is counted as visible.
* Tier 0 interaction (P1.7). Tier assignment and previews are not modelled here; the progressive-tier ADR
  combines this with that spike's numbers.
* The ground-truth check proves no visible entity is missing, not that the cheapest possible set was
  read; the false-positive columns are the measure of the latter.
