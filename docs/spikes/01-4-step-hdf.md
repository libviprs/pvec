# Spike P1.4: ISO/TS 10303-26 STEP-HDF as a container candidate

Code: `spikes/step-hdf/` (writer, readers, measurement driver), `spikes/step-hdf-wasm/` (a wasm32 reader).
Raw output of every run quoted here is committed in `docs/spikes/01-4-data/`, and the commands are
`spikes/step-hdf/run.sh`, `run2.sh` and `spikes/step-hdf-wasm/run-wasm.sh`.

**Platform.** Every number is **native x64**: MARS (Intel Core i7-14700F), `rustc 1.99.1`, release
builds, inside `linux/amd64` containers, libhdf5 1.10.8 (Debian bookworm) through `hdf5-metno` 0.15.0.
Nothing was run on arm64. MARS is shared and its load average was between 160 and 290 during every
run (recorded at the head of each output file), so I report **CPU time** next to wall time. In every
row of the decode tables the two agree, so the load did not inflate them, but I would
not read a single digit of precision into a nanosecond figure.

**Corpus.** Fixture revision `pvec-p1-fixtures-r1` (`00-fixtures.md`): `small-synthetic` (1,000
entities), `spline-heavy` (50,000, 58% splines), `million` (1,000,000), plus `straddler` (100,000, for
P1.6, not used here). The shared viewport script is the one `pvec_fixtures::viewport_script()` commits.

## What I built, and what I did not

I did not have the licensed text of ISO/TS 10303-26. I built two HDF5-backed representations of the
fixtures, which is what the issue asks me to measure, and I make **no claim that either conforms to
Part 26**. The sentence the repository allows is the one in `ISO_PROFILE.md`, and it is about Part 42.

* **H1, the entity-table layout.** One HDF5 compound dataset per EXPRESS entity type
  (`CARTESIAN_POINT`, `DIRECTION`, `VECTOR`, `AXIS2_PLACEMENT_2D`, `LINE`, `CIRCLE`, `ELLIPSE`,
  `TRIMMED_CURVE`, `POLYLINE`, `B_SPLINE_CURVE_WITH_KNOTS`, `COMPOSITE_CURVE_SEGMENT`,
  `COMPOSITE_CURVE`, `GEOMETRIC_CURVE_SET`), one row per instance, references as row numbers, and
  lists as HDF5 variable-length arrays. This is the shape I understand the STEP-HDF mapping to have,
  with the same entity forms as the Part 21 baseline. A rational spline is one table with a flag instead of a complex entity instance.
* **H2, HDF5 as a plain container.** The same data as type-batched structure-of-arrays datasets with
  offset arrays instead of variable-length types, one flat bounds dataset (`bbox`, four `f64` per
  entity) so a viewport query has something to scan, and a `roots` dataset. This is the best case for
  HDF5 that I could build without writing a spatial index of my own.

Neither has a spatial index: HDF5 does not provide one. Both have the bounds table added as an
extension, which is not part of any STEP mapping, and I say so wherever it matters.

Correctness gate: every reader decodes every fixture. H2 reproduces `Drawing::checksum()` bit for bit
on all three fixtures (`79a8dfecc088fc8a` for `million`). H1 does **not**, and that is a result: STEP's
line is an origin, a unit direction and a length, so the end point is rebuilt as `origin + length *
direction` and 2,099 of the 1,000,000 entities come back with the last bits changed (the other 997,901 are exact). The worst
deviation is 1.137e-13 on `million` and 5.7e-14 on `small-synthetic`, which is well inside the numeric
policy's tolerance (NP-04) but means the checksum differs. The test
`h1_round_trips_within_the_rounding_of_the_step_line_form` asserts both facts, so a future change that
made H1 exact would fail it and make me revisit this paragraph.

## 1. File size against the Part 21 baseline

Bytes, and ratio to the Part 21 text of the same drawing (`small` 315,128; `spline-heavy` 89,798,481;
`million` 365,814,152). "chunk" is rows per chunk; every compressed row uses the shuffle filter and
deflate. Full table of ten configurations per fixture and layout: `01-4-data/hdf-small.txt`,
`hdf-spline.txt` (all ten) and the `million` subset in `hdf-million.txt`.

| fixture | layout | contiguous | contiguous, newest format | chunk 65,536 + shuffle + deflate 6 | chunk 4,096 + shuffle + deflate 6, newest |
|---|---|---|---|---|---|
| small-synthetic | H1 | 170,188 (0.540) | 167,756 (0.532) | 141,728 (0.450) | 107,886 (0.342) |
| small-synthetic | H2 | 112,008 (0.355) | 108,337 (0.344) | 133,603 (0.424) | 87,415 (0.277) |
| spline-heavy | H1 | 40,329,688 (0.449) | 40,314,968 (0.449) | 32,139,662 (0.358) | 32,831,570 (0.366) |
| spline-heavy | H2 | 29,793,256 (0.332) | 29,790,824 (0.332) | 20,849,561 (0.232) | 21,364,198 (0.238) |
| million | H1 | 162,091,844 (0.443) | 162,093,508 (0.443) | 89,603,057 (0.245) | 92,772,472 (0.254) |
| million | H2 | 100,478,313 (0.275) | 100,475,881 (0.275) | 70,011,564 (0.191) | 70,615,194 (0.193) |

For comparison, the P1.5 custom layout B is 65,813,472 bytes on `million` (0.180 of Part 21), so even the
best compressed HDF5 file is 6% larger than an uncompressed custom layout, and the uncompressed one is
53% larger. Chunking a small file costs more than it saves: the 1,000-entity H2 file grows from 112,008 to
154,224 bytes at chunk 4,096 because of the chunk index and the per-dataset headers.

**Chunking and compression, spline-heavy H2** (the dataset the settings matter most for), bytes:
contiguous 29,793,256; chunk 4,096 30,430,504; chunk 65,536 30,483,688; chunk 4,096 + deflate 1
19,594,326; chunk 4,096 + shuffle + deflate 1 21,798,801; + deflate 6 21,379,519; chunk 65,536 +
shuffle + deflate 6 20,849,561; + deflate 9 20,802,645. Two things to read from it. Shuffle makes the
file *bigger* here (deflate 1 alone is 19.6 MB against 21.8 MB with shuffle); I did not investigate why.
And going
from deflate 6 to 9 buys 0.2% at nearly five times the write time (0.6 s to 2.8 s). The codec choice itself
belongs to P4.6 and P8.3; this table only says HDF5's deflate path is not obviously the winner.

## 2. HDF5 metadata overhead

Two numbers, because one of them is misleading for H1.

* **File minus the logical payload** (uncompressed configurations): H2 `million` 10,624 bytes
  (0.01%), `spline-heavy` 10,624, `small-synthetic` 11,863 (11.8% of a 100 KB payload). H1 `million`
  11,294,726 bytes (7.5%), because every variable-length array pays a global-heap header.
* **File minus the sum of dataset storage sizes**: H2 `million` 10,624 bytes; H1 `million`
  **47,081,856 bytes, 31.2% of the payload**. This one is not "metadata" in the usual sense: HDF5 reports
  a variable-length dataset's storage as the 16-byte references only, so every variable-length element
  (polyline point lists, spline knots and control-point references, composite segments) lands in this
  difference. It is the cost of the variable-length mapping, which is why H1 is 1.6 times the size of H2
  uncompressed.

Chunked datasets add an index per dataset: H2 `million` at chunk 4,096 + newest format is 28,691 bytes of
non-dataset space, at chunk 65,536 it is 52,565.

## 3. Random read of a viewport-sized subset

Method: the log file driver (`H5Pset_fapl_log`, location-of-reads flag) records every read libhdf5 issues
to the file. A query scans the flat bounds dataset (and the `roots` dataset), keeps the entities whose
bounds overlap the viewport, then reads exactly the rows they need, level by level through references
for H1. The file is opened fresh for each step (cold HDF5 metadata cache, warm OS page cache). The visible
set is checked against ground truth computed from the fixture for every step, and every returned
entity is checked against the original, so a wrong answer cannot hide in a fast number.

`million`, uncompressed, contiguous, bytes read by the library and the ranges after merging reads whose
gap is at most 64 KiB (what an HTTP client would plausibly coalesce), CPU milliseconds:

| step | visible | H2 bytes | H2 ranges | H2 cpu ms | H1 bytes | H1 ranges | H1 cpu ms |
|---|---|---|---|---|---|---|---|
| cold open (whole drawing) | 999,990 | 100,488,889 | 1 | 336.6 | 162,491,140 | 1 | 991.8 |
| zoom in (1/4 x 1/4) | 63,045 | 51,049,337 | 39 | 50.6 | 58,991,080 | 79 | 215.4 |
| pan right | 63,001 | 50,994,272 | 39 | 58.9 | 59,115,240 | 80 | 197.5 |
| zoom in (1/16 x 1/16) | 4,031 | 42,306,202 | 21 | 13.4 | 44,076,872 | 43 | 44.8 |
| zoom out (whole drawing) | 999,990 | 100,488,889 | 1 | 237.3 | 162,491,140 | 1 | 1,172.1 |

The decisive row is the last-but-one. A viewport that shows 4,031 entities (0.4% of the drawing) makes
the library read **42.3 MB** with H2 and 44.1 MB with H1, 42% and 27% of the file. The cause is plain:
the bounds scan alone is 32 MB (four `f64` per entity) plus 8 MB for `roots`, and HDF5 offers no spatial
index to avoid it. The bytes that belong to the visible entities are a small remainder. The P1.5 custom
layout B, whose directory and Morton-ordered blocks do the selection, reads **944,504 bytes in 10 ranges**
for the same viewport (`01-5-layouts.md`), 45 times less. Per-step tables for all fixtures and all
three tested configurations are in the data directory; the totals over the whole script (nine steps)
are 542,442,099 bytes for H2 contiguous and 711,906,336 for H1, against 166,108,504 for layout B.
(The two documents count ranges slightly differently: here they are reads merged at a 64 KiB gap, there
they come from the block directory. The comparison is on bytes.)

**Compression multiplies the problem unless the chunk cache is raised.** With the default 1 MiB chunk
cache and 65,536-row chunks, one selective viewport on `million` re-reads and re-decompresses chunks
over and over: the 1/4 x 1/4 step reads **4,697,300,511 bytes** from a 70 MB H2 file (2,004,195,309
for H1) and takes 14.9 s of CPU. With a 256 MiB chunk cache the same step reads 46,434,701 bytes in
10 ranges and 222.8 ms of CPU. Smaller chunks help without the cache: 4,096 rows and the newest format
read 35,871,319 bytes for that step. These are the numbers a reader would meet if it used stock
settings, and the first one is a trap rather than a property of the format. All three variants are in
`hdf-million.txt` and `hdf2-million.txt`.

The same shape holds at the other sizes. `spline-heavy`, H2 contiguous: cold open 29,797,264 bytes, 1/16
viewport (297 visible) 3,595,945 bytes in 13 ranges. `small-synthetic`: every step reads most of an
87 to 170 KB file in one merged range, so it cannot discriminate.

## 4. Remote range-read practicality

The same reads, as ranges per viewport and bytes per viewport (table above and the data files). For
`million` H2 contiguous the 1/16 step is **21 ranges and 42.6 MB** after a 64 KiB merge, and **36 ranges
and 42.3 MB** with no merge. Raw library reads for that step number 72. Two practical facts:

* A remote reader cannot know which ranges to ask for without first reading dataset headers that sit
  scattered through the file: the library issued 72 reads for that one step, which merged to 21 ranges.
  With layout B the directory is one fixed offset.
* The bytes dominate, not the request count. 42.3 MB for a 4,031-entity viewport is 34 seconds at
  10 Mbit/s before any decoding, which is the failure the epic's streaming goal exists to avoid. I have
  not measured an actual HTTP transfer; this is arithmetic on measured bytes.

## 5. Browser and WASM footprint

**Real build, pure Rust.** `libhdf5` is a C library linking 34 further shared libraries on Debian. Its Rust
binding does not compile for `wasm32-unknown-unknown` (`hdf5-metno-sys` fails: `libc::FILE`, `size_t` and
others do not exist on that target; the build output is in `01-4-data/hdf-wasm-c-fail.txt`), and I have no
wasm build of libhdf5 itself (I did not try Emscripten). A pure-Rust, read-only reader exists: `hdf5-reader` 0.9.1. I compiled it for
`wasm32-unknown-unknown` with a small decoding harness (`spikes/step-hdf-wasm`):

| build | bytes | gzip -9 |
|---|---|---|
| default release profile | 603,578 | 187,652 |
| `opt-level=s`, LTO, `panic=abort`, stripped | 483,228 | 152,668 |

For comparison the P1.5 candidates are 29,365 bytes (B) to 65,596 bytes (per-entity baseline). Instantiation
in Node 22 (V8) takes about 1.0 to 1.1 ms for the size-optimised module (one run of the default module took
5.5 ms, so call it 1 to 6 ms).

Throughput, measured by decoding the file image from wasm memory and folding every value into a hash that
the native reader also computes (the hashes agree on every file, which is the correctness check), CPU time
under Node:

| file | bytes | wasm (two builds) | native libhdf5, same work |
|---|---|---|---|
| `million` H2 contiguous | 100,478,313 | 85.3 to 87.4 ms (1,097 to 1,130 MiB/s) | 102.5 ms |
| `million` H2, shuffle + deflate 6 | 70,085,198 | 415.3 to 454.4 ms (147 to 161 MiB/s) | 331.1 ms |
| `spline-heavy` H2 contiguous | 29,793,256 | 27.6 to 28.7 ms | 31.9 ms |
| `spline-heavy` H2, shuffle + deflate 6 | 20,826,710 | 126.0 to 137.8 ms | 104.3 ms |
| `million` H1, newest format | 162,093,508 | **15,169 ms (10 MiB/s)** | 90.6 ms |
| `spline-heavy` H1, newest format | 40,314,968 | 383.8 ms | 20.5 ms |

So the primitive-array layout decodes in wasm about as fast as the C library does natively. The
variable-length path does not: `hdf5-reader` re-parses a global-heap collection for every reference it
resolves. The cost per polyline is about the same on both fixtures (4,889 polylines in 383.8 ms is 78
microseconds each, 200,221 in 15,169 ms is 76), so the 200,221 polylines of `million` account for the 15
seconds, 167 times the native library. I did not profile it further. It is a property of this crate's API in
this version, and the variable-length path is precisely the part of H1 that a STEP-style mapping needs.

**A reader defect worth recording.** For files written with libhdf5's default (oldest) format,
`hdf5-reader` 0.9.1 mis-parses compound datatypes: it reports every member at byte offset 0, a member of
size 17, and `CARTESIAN_POINT: invalid data: compound field 'y' range exceeds record size`, and fails on
`B_SPLINE_CURVE_WITH_KNOTS` with `unsupported datatype class 15` (output in `01-4-data/hdf-probe.txt`).
The same files written with the newest format parse correctly. So a pure-Rust browser reader works only if
the writer is pinned to the newest file-format version, which in turn means libhdf5 1.10 or later on the
writing side. H2's datasets are primitive arrays and are unaffected; its `roots` dataset is a compound and is
the one place it would meet this.

## 6. Decode complexity

* **Reader lines of code** (non-blank, non-comment, native, the part after the `reading` marker): H1 227,
  H2 178. The wasm harness is 75. For comparison, layout B's whole reader is a few hundred lines
  with no dependency (`01-5-layouts.md`).
* **Dependencies, native**: the `hdf5-metno` tree is 36 crates (a proc-macro derive among them) plus the
  C library, `libhdf5_serial.so` at 3,855,744 bytes, which `ldd` shows linking 34 further shared
  libraries (libcurl, openssl, gnutls, krb5 and others, through the Debian build).
* **Dependencies, pure Rust**: the `hdf5-reader` tree is 40 crates, no C, no build scripts (its
  documentation says so), and `memmap2` and `rayon` are compiled in even for wasm.
* **Decoding to objects**, `Vec<Entity>` (CPU, median of three, `01-4-data/hdf2-*.txt`):

| fixture | H1 contiguous | H2 contiguous | H1 chunk 65,536 + shuffle + deflate 6 | H2 same |
|---|---|---|---|---|
| small-synthetic | 905 ns/entity | 472 | 1,408 | 1,251 |
| spline-heavy | 737 | 397 | 1,783 | 1,727 |
| million | 124 | 85 | 379 | 252 |

Peak resident set for `million`: 402,248 KiB for H1 and 380,244 KiB for H2 (including the decoded
objects; the process starts at about 8.7 MiB). The per-entity cost falls with fixture size and with the
share of simple entities (`spline-heavy` is 58% splines); I did not separate the two effects.

## 7. Progressive-stream representation

Expressible, by convention, not by the format. In these files the contiguous datasets sit in the order
they were written, so writing the cheap kinds first puts them in a file prefix, and a reader that knows the
dataset addresses can fetch that prefix alone. I measured it with the log driver: reading the stand-in for Tier 0
(roots, lines, circles, arcs and polylines) from `million` H2 touches 25 reads, 56,164,336 bytes, in 4
ranges at a 64 KiB merge, with the highest address at 90,147,434 of 100,478,313 bytes (89.7%). On
`spline-heavy` the same datasets sit in the first 11.4% of the file (3,395,962 bytes). The `million`
number is high because my bounds dataset (32 MB) is written before the polylines; written last, the same
datasets would end about 32 MB earlier, near 58 MB. The point is that the prefix is the writer's choice, nothing in HDF5 knows about
tiers, previews or replacement identities (P1.7, P3.6), and a streaming reader still has to discover
dataset addresses from headers spread through the file. I did not test appending a refinement tier to a
file in flight; HDF5's single-writer-multiple-reader mode is a file-system mechanism, not an HTTP one.

## 8. GPU-friendly extraction

Task: produce a packed buffer of `[x0, y0, x1, y1]` for every line segment of the drawing (the first thing a
renderer wants), `million`, 599,431 lines, CPU, median of three:

| layout | contiguous | chunk 65,536 + shuffle + deflate 6 | copies and transforms |
|---|---|---|---|
| H2 | 16.1 ms (26.9 ns/segment) | 80.1 ms (133.7 ns/segment) | one library read into the user buffer (plus decompression when chunked); the array is already the packed form up to the `f32` narrowing |
| H1 | 48.0 ms (80.1 ns/segment) | 291.5 ms (486.4 ns/segment) | six tables read whole (`GEOMETRIC_CURVE_SET`, `TRIMMED_CURVE`, `LINE`, `VECTOR`, `DIRECTION`, `CARTESIAN_POINT`), a four-hop reference chase per line, then `origin + length * direction` per end point |

H1 is 3.0 times slower contiguous and 3.6 times slower compressed, and its result differs from the
source by up to 1.1e-13. The `spline-heavy` ratios are 11 times (421.9 against 37.8 ns/segment) and
18 times (3,845.7 against 209.5).

## Decision: reject as the PVec container

**Reject** ISO/TS 10303-26 STEP-HDF, and HDF5 as a plain container, as the PVec v1 physical container.
The call is provisional until P1.8 puts the container ADR next to P1.5's and P1.6's tables, but the numbers
are not close.

The deciding numbers, all native x64, `million`:

1. **Viewport I/O.** 42.3 MB read for a 1/16 x 1/16 viewport showing 4,031 entities, with HDF5's best
   case (H2, contiguous, a flat bounds scan, no spatial index). The custom layout reads 0.94 MB. That is
   a 45-fold gap on the streaming goal the epic exists for, and it comes from HDF5 not having a spatial
   index, not from a setting I failed to find.
2. **Size.** 100.5 MB for H2 against 65.8 MB for layout B uncompressed, and 162.1 MB for the faithful table
   mapping (H1). Compressed H2 (70.0 MB) only nearly catches up, with the chunk-cache trap above.
3. **Browser.** 483 to 604 KB of wasm for a reader that works only on newest-format files, against 29 KB for
   B, and the variable-length path the STEP mapping needs decodes at 10 MiB/s.
4. **Exactness.** The STEP-style line form loses the last bits of 2,099 of 1,000,000 entities (worst
   1.1e-13). Harmless under NP-04, but it means the container could not hold the source numbers verbatim.

What I would keep: the **semantic** lessons (entity forms, what a STEP round trip does to a line), which are
already in `ISO_PROFILE.md` and P6.2's scope, and HDF5 as a *benchmark baseline* in P8.2, where it can be
re-measured against the finished container. HDF5 stays out of v1 as an export target, per the issue.

**The threshold that would reverse it.** Reverse to "adopt-partially" (HDF5 as the storage layer under a
PVec-defined block directory and spatial index) if both hold on a rerun against revision r2 or later:
(a) an HDF5 file with Morton-ordered entities and one chunk per block, plus a per-chunk bounds dataset, reads
at most twice the custom layout's bytes for the 1/16 viewport on `million` (that is at most 1.9 MB, against
the 42.3 MB measured here); and (b) a maintained pure-Rust reader decodes variable-length data at no worse
than 100 MiB/s in wasm and reads oldest-format compound types correctly. I did not build (a); it is the
experiment that would change my mind, and it turns HDF5 into a block store with metadata overhead
(28,691 to 52,565 bytes per file here) and a third-party reader in the browser bundle.

## What I did not measure

* arm64. Nothing was run there.
* An actual HTTP range-request transfer. Ranges and bytes come from the library's read log, merged at a
  stated gap; transfer times are not measured.
* The spatially sorted, chunk-per-block HDF5 variant described in the reversal threshold.
* HDF5 1.12 and 1.14 behaviour (the container has 1.10.8), the `ros3` driver, and the h5py reader.
* wasm SIMD, and any wasm disassembly. Observed vectorisation is not reported for libhdf5 (a C library)
  or for the pure-Rust reader's wasm output.
* Conformance of H1 to the Part 26 text, which I did not have.
* The `h1` reader panics on library errors (`expect`) instead of returning them; the one test that feeds it
  a truncated file only requires that it not return a plausible drawing.
* Decode numbers carry a run-to-run noise of a few percent at least; small-fixture numbers (under a
  millisecond) are noise-dominated.
* `hdf-spline.txt`, `hdf-million.txt` and `hdf-small.txt` also contain decode and extraction lines from before
  I replaced a slow H2 gather that built a map per entity; the tables above use the second pass (`hdf2-*.txt`).
