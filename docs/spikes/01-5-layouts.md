# Spike P1.5: custom physical layouts

Code: `spikes/layouts/` (throwaway). Driver: `tools/layouts.sh`. Raw output of every table below is
committed in `docs/spikes/01-5-data/` (`.run2` files are a second full pass). Fixtures: corpus
revision **`pvec-p1-fixtures-r1`**, the four drawings of `docs/spikes/00-fixtures.md`, so this
document reads line by line against P1.4's table once that lands.

**Every number here is native x64**: an Intel Core i7-14700F on MARS, `rustc 1.99.1`, `--release`,
single thread, inside a `linux/amd64` container (and node 22 for the wasm runs). Nothing was run on
arm64, so nothing is labelled arm64.

## What was measured

Four encodings of the same entities, behind one `Codec` trait and one file container, so the only
thing that differs between rows is the block payload:

| id | candidate | notes |
|---|---|---|
| **A** | FlatBuffers block, type-batched SoA | `schema/pvec_soa.fbs`, `flatc` 25.12.19, `flatbuffers` crate 25.12.19, read through the verifying `root_as_block`. Two readers over the same bytes: `A` uses the generated iterators, `A raw` sums the vectors' byte view |
| **B** | custom packed binary with an offset table | SoA sections for immediate geometry, a byte-offset arena for splines and composites. No dependencies, no `unsafe` |
| **C** | rkyv 0.8 zero-copy archive of SoA structs | validated with bytecheck on `access` |
| **base** | FlatBuffers, one table per entity | `schema/pvec_obj.fbs`, a union of per-kind tables. The "one serialized object per entity" shape of panel finding 6 |

**Why C is rkyv.** It is the Rust-native zero-copy alternative to FlatBuffers: no schema compiler, no
generated code, validation built in. It answers whether a Rust-first stack gains anything over
FlatBuffers for the same SoA shape. Its cost is that the format is defined by a crate version, not by a
language-neutral schema, which matters to the browser reader (EPIC M). I considered an Arrow-style
columnar layout and a Cap'n Proto one; neither adds a design point that A, B and C do not already cover
(Arrow IPC is SoA with a heavier envelope, Cap'n Proto is FlatBuffers-shaped with a different schema
compiler), and each needs a toolchain the others do not.

**Shared framing.** Entities are sorted by Morton key of their bbox centre (16 bits per axis) and
chunked into blocks of 512 in that order. The file is a 32 byte header, 16-byte aligned blocks and a
56-byte-per-block directory (offset, length, count, bbox). The block assignment is a placeholder
borrowed from P1.6's territory, only there so viewport access is defined; the block size is a
parameter, not a result. A viewport reads every block whose bbox it overlaps, and neighbouring blocks
merge into one range.

**Workload.** *Touch* walks every value of every entity and adds the bit patterns of the floats (plus
ids, degrees and composite part tags) with wrapping integer addition, which is order independent. It
is what a consumer that packs arrays for the GPU does to each value, without the packing. *Materialise*
builds owned `Vec<Entity>`. Every decoder must reproduce the fixtures exactly and agree on the touch
sum (`tests/roundtrip.rs`: decoded entities equal the source bit for bit, `Drawing::checksum()` matches,
touch sums match the sum computed from the source), and the same sums come out of the wasm builds.

## Encoded size

File bytes (header, blocks, directory), ratio to the Part 21 baseline in brackets.

| fixture | Part 21 bytes | B | A | C | base |
|---|---|---|---|---|---|
| small-synthetic (1,000) | 315,128 | 65,472 (0.208) | 65,168 (0.207) | 64,736 (0.205) | 90,752 (0.288) |
| spline-heavy (50,000) | 89,798,481 | 28,216,016 (0.314) | 28,000,544 (0.312) | 27,979,840 (0.312) | 29,636,480 (0.330) |
| million (1,000,000) | 365,814,152 | 65,813,472 (0.180) | 65,372,368 (0.179) | 64,958,672 (0.178) | 91,215,200 (0.249) |
| straddler (100,000) | 35,306,923 | 6,594,176 (0.187) | 6,548,560 (0.186) | 6,506,992 (0.184) | 9,136,560 (0.259) |

The three SoA layouts are within 1.3% of each other on every fixture, because the payload is f64
coordinates and no layout can shrink those. The per-entity baseline costs 39% more on the small
fixture, 39% on `million`, and 5% on `spline-heavy`, where the payload dominates.

Bytes per entity by kind, `million`, whole file included, blocks in id order (`measure kinds`):

| kind | entities | B | A | C | base |
|---|---|---|---|---|---|
| line | 599,431 | 36.7 | 36.3 | 36.8 | 60.2 |
| circle | 40,151 | 28.7 | 28.3 | 28.8 | 52.2 |
| arc | 79,920 | 44.7 | 44.3 | 44.8 | 68.2 |
| ellipse | 30,033 | 60.7 | 60.4 | 60.8 | 80.3 |
| polyline | 200,221 | 128.7 | 128.2 | 128.8 | 156.1 |
| spline | 30,141 | 237.9 | 230.6 | 231.0 | 267.5 |
| composite | 20,103 | 212.3 | 176.4 | 176.9 | 279.7 |

B pays for its arena records on the two advanced kinds: 7.3 bytes more per spline than A, 35.9 more per
composite. That is the padding below, and it is the one place B is measurably worse on size.

### Padding in B

Reported from the encoded blocks, not estimated (`million`; other fixtures in the raw data):

* section alignment and block tails: 39,000 bytes in total, 0.04 bytes per entity;
* spline records: 6 bytes of padding per curve (a 16 byte record header holds a degree byte, a rational
  flag and two 4-byte counts), 180,846 bytes over 30,141 curves; the control points, knots and weights
  that follow are 8-byte aligned without further padding and contiguous inside the record;
* composite records: 4 bytes after the part count and 4 after each part tag, 441,400 bytes over 20,103
  composites (22 bytes per composite). A one-byte tag and a packed part count would remove most of it;
  I did not, because it would not change the decision.

## Cold open and viewport access

Cold open reads the 32 byte header and then the directory, so **2 ranges**, and the byte count is the
same for every candidate: 144 bytes (small), 5,520 (spline-heavy), 109,456 (million), 11,008
(straddler). The directory is 56 bytes per block; at 512 entities per block that is 0.11 bytes per entity.

The committed script (`pvec_fixtures::viewport_script()`) replayed on `million`. Ranges and blocks are
identical for every candidate; bytes differ by block size:

| step | blocks hit | ranges | B bytes | A bytes | C bytes | base bytes |
|---|---|---|---|---|---|---|
| cold open (whole drawing) | 1,954 | 1 | 65,704,008 | 65,262,912 | 64,849,216 | 91,105,744 |
| zoom in (1/4 x 1/4) | 189 | 31 | 6,382,168 | 6,339,944 | 6,299,920 | 8,840,512 |
| pan right (1/8) | 194 | 32 | 6,522,808 | | | |
| pan right | 189 | 31 | 6,346,408 | | | |
| pan right | 169 | 21 | 5,671,392 | | | |
| pan right | 94 | 16 | 3,161,816 | | | |
| zoom in (1/16 x 1/16) | 28 | 10 | 944,504 | 938,232 | 932,296 | 1,308,448 |
| zoom out (1/4 x 1/4) | 169 | 21 | 5,671,392 | | | |
| zoom out (whole drawing) | 1,954 | 1 | 65,704,008 | | | |
| **script total** | | **164** | **166,108,504** | **164,999,192** | **163,952,992** | **230,325,128** |

The 1/4 x 1/4 view needs 31 ranges and about 6.3 MB because the Morton blocks are not perfectly
aligned to the viewport; the script total is 164 ranges and about 2.5 times the file, since the cold
open and the final zoom out each read it whole. The per-step tables for every candidate and fixture are
in `docs/spikes/01-5-data/native-tables.txt`. Block assignment and index layout are P1.6's to decide,
so I treat these as the cost of one reasonable assignment, not as a result about the layout. The
layout-dependent part is the last four columns: B, A and C read within 1.3% of each other, the baseline
reads 39% more.

## Decode cost

Native, ns per entity, median of 7 runs, `touch`. Two full passes (run 1 / run 2); the second pass is
`native-tables.run2.txt`.

| fixture | B | A (iterators) | A raw | C | base |
|---|---|---|---|---|---|
| million, whole file | 3.7 / 3.6 | 5.2 / 5.7 | 5.5 / 5.9 | 4.7 / 3.2 | 72.1 / 71.7 |
| million, viewport script | 3.2 / 3.2 | 4.8 / 4.9 | 4.8 / 4.6 | 3.1 / 2.7 | 71.9 / 71.2 |
| spline-heavy, whole file | 18.2 / 58.4 | 16.7 / 16.9 | 16.2 / 17.2 | 14.0 / 20.3 | 136.7 / 139.8 |
| spline-heavy, viewport script | 19.4 / 16.3 | 13.4 / 13.8 | 13.1 / 13.2 | 12.0 / 12.2 | 124.7 / 116.0 |
| straddler, whole file | 2.0 / 2.0 | 2.4 / 3.9 | 2.3 / 2.4 | 1.2 / 1.4 | 69.7 / 81.9 |
| small-synthetic, whole file | 2.8 / 2.9 | 3.3 / 3.3 | 3.0 / 2.9 | 1.7 / 1.4 | 97.1 / 63.6 |

Read the noise before the ranking. The two passes disagree by up to 3x on one cell (B on spline-heavy,
18.2 against 58.4, which I take to be a scheduling outlier), and small-synthetic is 1,000 entities,
about 3 microseconds of work per pass. What survives both passes is the large effect and one medium one:

* **SoA against per-entity objects: a ratio of 12.6x to 22.4x on `million`** (72.1 ns against 3.7 for
  B is 19.5x; against A 5.2 is 13.9x; against C 4.7 is 15.3x; second pass 71.7 against 3.6, 5.7 and 3.2
  is 19.9x, 12.6x and 22.4x). On `spline-heavy`, where the work per entity is larger, it is 7x to 10x
  (136.7 against 18.2, 16.7 and 14.0 in the first pass).
* B and C are indistinguishable on `million` (3.6 to 3.7 against 3.2 to 4.7); A is 1.4 to 1.6x slower
  than B (5.2 and 5.7 against 3.7 and 3.6). The A reader over the vectors' raw byte view is no faster
  than the generated iterators, which tells me the generated iterators already compile to a vector loop
  (see below).

**The advantage is for consumers that read arrays.** *Materialise*, which builds owned objects,
shrinks it to 1.2x to 1.4x on `million` (B 177.8 / 197.1 ns, A 199.6 / 194.8, C 182.8 / 180.0, base
241.1 / 244.5), because allocation and per-object construction dominate for every layout. If the
reader's job is to hand packed arrays to the GPU or the wasm evaluator, SoA wins by an order of
magnitude; if it is to produce `Vec<Entity>`, the layout barely matters.

Encode time, `million`: B 56 ms, A 65, C 69, base 147 (second pass 57, 67, 71, 146).

### Peak memory

`VmHWM` of a fresh process reading one file (`measure mem`). The process reads the file and copies it
into an aligned buffer, so its high-water mark is twice the file before any decoding; the rows give what
decoding adds to that.

| fixture | mode | B | A | C | base |
|---|---|---|---|---|---|
| million (file 64,270 / 63,840 / 63,436 / 89,077 kB) | touch | +0 kB | +0 | +0 | +0 |
| million | materialise | +107,948 kB | +109,252 | +109,332 | +84,020 |
| spline-heavy (file 27,554 / 27,344 / 27,324 / 28,941 kB) | touch | +0 | +0 | +0 | +0 |
| spline-heavy | materialise | +9,008 | +9,692 | +10,004 | +7,624 |

A zero-copy touch adds nothing beyond the file for every candidate. Materialising adds about 1.7 times
the file size for the SoA layouts and 0.9 times for the baseline on `million`. I did not investigate
why the baseline's is smaller.

## WASM (real wasm32 builds)

Each candidate is its own `cdylib` for `wasm32-unknown-unknown` (`cargo build --release --lib
--no-default-features --features cand-x`, symbols stripped), run under node 22 on MARS. The host
allocates in linear memory, copies the encoded file in, and calls the touch export. All five candidates
built; none failed. Every returned value equals the native result for the same file (checked on all four fixtures; node prints the
signed i64, the later run in the repository prints it unsigned).

| candidate | wasm bytes | external crates | `million` ns/entity (run 1 / run 2) | MB/s |
|---|---|---|---|---|
| B | 29,365 | 0 | 5.2 / 5.5 | 12,598 / 11,908 |
| C | 34,855 | 23 (rkyv, bytecheck, rend, munge, and proc-macro crates at build time) | 4.3 / 4.9 | 15,028 / 13,241 |
| A (both readers) | 60,511 | 2 (flatbuffers, bitflags) | 9.7 / 8.8 (raw view 9.0 / 8.5) | 6,773 / 7,442 |
| base | 65,596 | 2 | 89.9 / 96.1 | 1,015 / 950 |

SoA against per-entity objects in wasm on `million`: 89.9 ns against 5.2 for B is **17x** (96.1 against
5.5, 17x again); against A 9.7 it is 9x. On `spline-heavy`: B 35.7 / 38.8, A 35.1 / 36.5, C 32.9 / 28.9,
base 211.7 / 184.7 (5x to 6x). Compile plus instantiate is 0.2 to 1.6 ms for every module, so
instantiation is not a differentiator at these sizes. Bundle cost is: B is half of A's 60.5 kB.

The wasm bytes are the whole module for that candidate including the file container and `Aligned`
buffer helper, so the difference between candidates is real but the floor is not zero.

## Observed vectorisation

Native release build, baseline `x86-64` target (SSE2, no AVX2), `objdump -d` of the binary, per
`#[inline(never)]` summing function (`spikes/layouts/count_simd.sh`, raw output in
`01-5-data/vectorisation.txt`):

| loop | instructions | `paddq`/`paddd` | instructions touching xmm | wide moves |
|---|---|---|---|---|
| B `sum_f64` | 45 | 4 | 10 | 2 |
| B `sum_u32` | 47 | 4 | 13 | 0 |
| A iterators `sum_iter` | 130 | 6 | 16 | 4 |
| A raw view `sum_raw` | 71 | 4 | 10 | 2 |
| A ids `sum_ids` | 150 | 6 | 22 | 2 |
| C `sf` | 44 | 4 | 10 | 2 |
| C `si` | 46 | 4 | 13 | 0 |
| base `touch` (whole function, not a loop) | 1,372 | 28 | 106 | 46 |

Every SoA summing loop compiled to a packed 64-bit add, so on x64 the vectorisation is real for all
three SoA layouts and shows in the 12x to 22x ratio above; B and C produce the same 44 to 47
instruction loop. I **did not inspect the wasm**: the default `wasm32-unknown-unknown` target has no
`simd128`, so I expect scalar loops, but I did not disassemble the modules to confirm it.

## Hostile input

`tests/roundtrip.rs` feeds each decoder 400 mutated blocks (truncation, four random bit flips, a
`u32::MAX` word written over a count or offset, pure noise) and requires no panic, in debug builds with
overflow checks. All pass. In B every offset is checked against the block length, section alignment and
element size before it slices, offset tables must be monotone, start at zero and end exactly at their
arena, arena records are re-checked against their own headers with checked arithmetic, and no `unsafe`
or file-derived indexing exists in `packed.rs`. A and the baseline go through the FlatBuffers verifier,
C through bytecheck. 400 mutations per candidate is a smoke test, not a fuzz campaign; P7.8 owns that.

## Recommendation: B for the internal data layout

Type-batched SoA sections for immediate geometry plus an offset-table arena of self-describing records
for splines and composites, as in `spikes/layouts/src/packed.rs`.

The numbers that decide it, B against the runner-up A (FlatBuffers SoA), `million`, native x64:

| | B | A | margin |
|---|---|---|---|
| bulk touch, native | 3.7 / 3.6 ns | 5.2 / 5.7 ns | A is 1.4x to 1.6x slower |
| bulk touch, wasm | 5.2 / 5.5 ns | 9.7 / 8.8 ns | A is 1.7x to 1.9x slower |
| wasm module | 29,365 bytes | 60,511 bytes | A is 2.1x larger |
| external crates | 0 | 2 | |
| file size | 65,813,472 | 65,372,368 | B is 0.7% larger |
| viewport script bytes | 166,108,504 | 164,999,192 | B reads 0.7% more |

**C against B is a tie on the numbers I could resolve** (native 3.2 to 4.7 against 3.6 to 3.7, wasm 4.3
to 4.9 against 5.2 to 5.5, sizes within 1.3%), so the choice between them is not a performance one. I
choose B because it costs 0 external crates against 23, its wasm module is 29 kB against 35 kB, and,
above all, because the spec has to own the layout. The epic's rule is that the semantic API is normative
and the encoding is replaceable; a layout defined by a crate version (C) or by a third-party schema
toolchain (A) puts the contract somewhere else, while B is a page of specification and a few hundred
lines to read it. B also keeps each spline's control points, knots and weights contiguous in one
record, which neither A nor C does (their arenas are per-kind vectors); I did not measure what that
buys for single-curve random access, so I treat it as a structural advantage, not a measured one.

What B costs: the reader is ours, so its hostile-input handling is ours to fuzz (smoke-tested here,
fuzzed in P7.8), and it has the padding noted above (6 bytes per spline, 22 per composite, fixable).

**Threshold that would reverse it.** Choose A if either of these happens: the browser reader decodes
in TypeScript directly instead of in the Rust evaluator compiled to WASM (so a language-neutral schema
with generated readers is worth 1.7x to 1.9x of wasm throughput and 2x of module size); or B's reader
fails fuzzing in P7.8 in a way that cannot be closed with bounds checks the verifier-based candidates
get for free. Choose C only if its dependency tree is accepted into the browser bundle policy, since it
buys nothing measurable over B here. Revisit B against either if P8.3's corpus shows B's wasm touch more
than 25% behind the faster of them (it is 12% to 21% behind C here).

## What I did not measure

* Anything on arm64.
* wasm SIMD (`simd128`), and the wasm disassembly. Wasm materialise.
* Real range requests: latency, HTTP overhead, coalescing beyond adjacency. The ranges and bytes are
  computed from the directory, not fetched.
* Block sizes other than 512, and any assignment other than Morton order of bbox centres.
* Single-curve random access, which is where B's contiguous records should matter.
* Compression (P4.6 and P8.3 own the codec benchmark) and quantised or f32 coordinates (the policy
  keeps f64 canonical).
* A fuzz campaign (400 mutations per candidate only).
* The cost of the TypeScript path for A.

Run-to-run noise is up to 3x on single cells and a few percent on the large ones; the second pass is in
the repository so a reader can check.

## Notes on how it was produced

`flatc` 25.12.19 came from the upstream release binary installed in the derived image
(`spikes/layouts/Dockerfile`); the generated code is committed in `spikes/layouts/src/gen/`.
The measurements were taken before three behaviour-neutral clean-ups (a dereference instead of a `clone`
on a `Copy` struct in the baseline, `is_empty` in B's validation, and two lint allowances that keep the
hot loops as measured instead of the newer std spellings clippy prefers). Tests and the wasm build were
re-run after them.
