# Phase 1 benchmark summary

One comparison of STEP-HDF and every custom candidate on the **same four fixtures at the same corpus
revision, `pvec-p1-fixtures-r1`** (`00-fixtures.md`: 1,000, 50,000, 1,000,000 and 100,000 entities), with
the numbers each earlier spike produced. It feeds the eight ADRs in `docs/adr/`. It adds no measurement of
its own except two: a STEP-HDF run on the fourth fixture (`straddler`), which P1.4 did not include, and
the compression and checksum measurements that the compression ADR needs.

**Platform.** Every number in every table is **native x64** unless a cell says "wasm": MARS (Intel Core
i7-14700F), `rustc 1.99.1`, `--release`, one thread, in `linux/amd64` containers; wasm figures are wasm32
modules run under node 22 (V8) on the same machine. **No number here was measured on arm64**; the
requirement to label arm64 results is met by there being none, and an arm64 run of the benchmark corpus
(P8.1 onwards) remains to be done. MARS was at a load average of 160 to 335 during every run (recorded in
the raw data); timings are therefore CPU time, and the P1.3 geometry timings, taken before I recorded the
load, should be read with the same caution.

## The joint table

Columns: **file bytes** with the ratio to the Part 21 text of the same drawing; **1/16 viewport** is the
1/16 x 1/16 step of the committed script (the most selective one), bytes and ranges; **whole script** is
the nine steps summed, including the two that show the whole drawing; **decode to objects** is the cost
to build a `Vec<Entity>`, ns per entity, CPU time.

* HDF5 rows read the flat bounds dataset (and `roots`) and then only the rows they need, through the
  library's read log. Their bytes and ranges are **unmerged** (no coalescing gap), to match the custom
  rows, whose ranges merge only blocks that touch. P1.4 also gives the 64 KiB-merged numbers. The two
  compressed HDF5 rows use a 256 MiB chunk cache; with the library default (1 MiB) one viewport on
  `million` reads 4.7 GB from a 70 MB file (`01-4-step-hdf.md`).
* The custom rows read **512-entity blocks in Morton order through a directory**, a placeholder index
  (P1.6 replaces it), and `viewport bytes` means whole blocks. The HDF5 rows have no spatial index, which
  is the point of the comparison.
* **Decode to objects is not a clean comparison.** The HDF5 readers and the custom readers are separate
  implementations written for different purposes, and the HDF5 SoA reader (85 ns/entity on `million`) is
  faster at building objects than layout B's reader (177.8 ns). What the layouts spike showed is that the
  structure-of-arrays win is in array consumers (touch: 3.7 ns/entity for B against 72.1 for the
  per-entity baseline, 12.6 to 22.4 times), and that building objects shrinks it to 1.2 to 1.4 times. I do
  not claim a decode-speed advantage for the custom container over HDF5; the decision does not rest on one.

#### small-synthetic (1,000 entities, Part 21 baseline 315,128 bytes)

| candidate | file bytes (ratio to Part 21) | 1/16 viewport: bytes / ranges | whole script: bytes / ranges | decode to objects, ns/entity | notes |
|---|---|---|---|---|---|
| STEP-HDF tables (H1), contiguous | 170,188 (0.540) | 110,512 / 12 | 1,202,328 / 94 | 905 | HDF5 1.10.8; no-merge bytes and ranges |
| STEP-HDF tables (H1), shuffle + deflate 6, chunk 65,536 | 141,728 (0.450) | 137,353 / 7 | 1,282,271 / 43 | 1408 | 256 MiB chunk cache |
| HDF5 SoA (H2), contiguous | 112,008 (0.355) | 71,664 / 12 | 765,714 / 109 | 472 | flat bounds scan, no spatial index |
| HDF5 SoA (H2), shuffle + deflate 6, chunk 65,536 | 133,603 (0.424) | 116,355 / 7 | 1,138,231 / 52 | 1251 | 256 MiB chunk cache |
| A FlatBuffers SoA | 65,168 (0.207) | 65,024 / 1 | 585,216 / 9 | 105.4 | 512-entity Morton blocks and directory |
| B custom packed, offset tables | 65,472 (0.208) | 65,328 / 1 | 587,952 / 9 | 93.7 | 512-entity Morton blocks and directory |
| C rkyv SoA | 64,736 (0.205) | 64,584 / 1 | 581,256 / 9 | 101.9 | 512-entity Morton blocks and directory |
| baseline: FlatBuffers per entity | 90,752 (0.288) | 90,608 / 1 | 815,472 / 9 | 122.1 | 512-entity Morton blocks and directory |

#### spline-heavy (50,000 entities, Part 21 baseline 89,798,481 bytes)

| candidate | file bytes (ratio to Part 21) | 1/16 viewport: bytes / ranges | whole script: bytes / ranges | decode to objects, ns/entity | notes |
|---|---|---|---|---|---|
| STEP-HDF tables (H1), contiguous | 40,329,688 (0.449) | 4,727,756 / 37 | 141,988,136 / 399 | 737 | HDF5 1.10.8; no-merge bytes and ranges |
| STEP-HDF tables (H1), shuffle + deflate 6, chunk 65,536 | 32,139,662 (0.358) | 5,063,306 / 19 | 132,173,451 / 184 | 1783 | 256 MiB chunk cache |
| HDF5 SoA (H2), contiguous | 29,793,256 (0.332) | 3,589,737 / 30 | 101,723,006 / 362 | 397 | flat bounds scan, no spatial index |
| HDF5 SoA (H2), shuffle + deflate 6, chunk 65,536 | 20,849,561 (0.232) | 5,917,317 / 9 | 105,609,596 / 64 | 1727 | 256 MiB chunk cache |
| A FlatBuffers SoA | 28,000,544 (0.312) | 2,005,344 / 2 | 95,096,832 / 31 | 240.5 | 512-entity Morton blocks and directory |
| B custom packed, offset tables | 28,216,016 (0.314) | 2,020,624 / 2 | 95,827,680 / 31 | 197.9 | 512-entity Morton blocks and directory |
| C rkyv SoA | 27,979,840 (0.312) | 2,003,872 / 2 | 95,026,616 / 31 | 203.1 | 512-entity Morton blocks and directory |
| baseline: FlatBuffers per entity | 29,636,480 (0.330) | 2,122,336 / 2 | 100,645,224 / 31 | 328.2 | 512-entity Morton blocks and directory |

#### million (1,000,000 entities, Part 21 baseline 365,814,152 bytes)

| candidate | file bytes (ratio to Part 21) | 1/16 viewport: bytes / ranges | whole script: bytes / ranges | decode to objects, ns/entity | notes |
|---|---|---|---|---|---|
| STEP-HDF tables (H1), contiguous | 162,091,844 (0.443) | 44,036,240 / 56 | 700,130,652 / 1005 | 124 | HDF5 1.10.8; no-merge bytes and ranges |
| STEP-HDF tables (H1), shuffle + deflate 6, chunk 65,536 | 89,603,057 (0.245) | 34,625,929 / 33 | 489,239,011 / 274 | 379 | 256 MiB chunk cache |
| HDF5 SoA (H2), contiguous | 100,478,313 (0.275) | 42,297,754 / 36 | 534,690,511 / 644 | 85 | flat bounds scan, no spatial index |
| HDF5 SoA (H2), shuffle + deflate 6, chunk 65,536 | 70,011,564 (0.191) | 40,193,926 / 13 | 458,788,466 / 95 | 252 | 256 MiB chunk cache |
| A FlatBuffers SoA | 65,372,368 (0.179) | 938,232 / 10 | 164,999,192 / 164 | 199.6 | 512-entity Morton blocks and directory |
| B custom packed, offset tables | 65,813,472 (0.180) | 944,504 / 10 | 166,108,504 / 164 | 177.8 | 512-entity Morton blocks and directory |
| C rkyv SoA | 64,958,672 (0.178) | 932,296 / 10 | 163,952,992 / 164 | 182.8 | 512-entity Morton blocks and directory |
| baseline: FlatBuffers per entity | 91,215,200 (0.249) | 1,308,448 / 10 | 230,325,128 / 164 | 241.1 | 512-entity Morton blocks and directory |

#### straddler (100,000 entities, Part 21 baseline 35,306,923 bytes)

| candidate | file bytes (ratio to Part 21) | 1/16 viewport: bytes / ranges | whole script: bytes / ranges | decode to objects, ns/entity | notes |
|---|---|---|---|---|---|
| STEP-HDF tables (H1), contiguous | 16,247,684 (0.460) | 5,761,092 / 34 | 82,527,532 / 325 | 107 | HDF5 1.10.8; no-merge bytes and ranges |
| STEP-HDF tables (H1), shuffle + deflate 6, chunk 65,536 | 9,654,461 (0.273) | 7,496,008 / 9 | 73,306,150 / 55 | 405 | 256 MiB chunk cache |
| HDF5 SoA (H2), contiguous | 10,068,859 (0.285) | 4,856,511 / 23 | 58,326,233 / 226 | 63 | flat bounds scan, no spatial index |
| HDF5 SoA (H2), shuffle + deflate 6, chunk 65,536 | 7,241,650 (0.205) | 6,973,022 / 6 | 63,494,244 / 44 | 243 | 256 MiB chunk cache |
| A FlatBuffers SoA | 6,548,560 (0.185) | 327,776 / 4 | 19,965,984 / 63 | 107.2 | 512-entity Morton blocks and directory |
| B custom packed, offset tables | 6,594,176 (0.187) | 329,720 / 4 | 20,101,136 / 63 | 93.4 | 512-entity Morton blocks and directory |
| C rkyv SoA | 6,506,992 (0.184) | 325,664 / 4 | 19,839,088 / 63 | 98.7 | 512-entity Morton blocks and directory |
| baseline: FlatBuffers per entity | 9,136,560 (0.259) | 459,248 / 4 | 27,861,000 / 63 | 158.8 | 512-entity Morton blocks and directory |

Reading the table: on every fixture the custom layouts are 0.18 to 0.31 of the Part 21 size, HDF5 SoA is
0.19 to 0.42, and the table form (H1) 0.25 to 0.54. The viewport columns are where they separate: on
`million` the 1/16 viewport reads 42,297,754 bytes in HDF5 SoA and 944,504 in layout B (45 times), and
the whole script 534.7 MB against 166.1 MB. The three custom SoA layouts are within 1.3% of each other on
file bytes and viewport bytes on every fixture (their decode columns differ by up to 12%). P1.4's straddler run (`01-4-data/hdf-straddler.txt`, `hdf2-straddler.txt`) was added for
this table and uses the same driver as the other three fixtures.

## Browser footprint

| candidate | wasm module | dependencies | notes |
|---|---|---|---|
| B custom packed | 29,365 bytes | 0 | `million` touch 5.2 / 5.5 ns per entity in wasm |
| C rkyv | 34,855 bytes | 23 crates | touch 4.3 / 4.9 ns |
| A FlatBuffers | 60,511 bytes | 2 crates | touch 9.7 / 8.8 ns |
| baseline FlatBuffers per entity | 65,596 bytes | 2 crates | touch 89.9 / 96.1 ns |
| HDF5, pure-Rust reader `hdf5-reader` 0.9.1 | 483,228 bytes size-optimised (603,578 default; 152,668 gzip) | about 40 crates | arrays 1,097 to 1,130 MiB/s; variable-length path 10 MiB/s; oldest-format compounds mis-parsed |
| HDF5 C library | no wasm build | libhdf5 3,855,744 bytes plus 34 shared libraries | the Rust binding does not compile for wasm32 |

## Spatial index and progressive tiers

The index comparison and the Tier 0 coverage are in `01-6-spatial-index.md` and `01-7-tier0.md`; the
numbers the ADRs quote are copied from there. In one line each: the packed Hilbert R-tree reads the fewest
bytes over the interactive steps on all four fixtures and the stress case (29.08 MB on `million` against
30.58 MB for the next, 46.45 MB for the quadtree); a global Tier 0 is 90.51% of the packed artifact on
`million` with 0 entities in neither class.

## Compression and checksums (feeds ADR 0006)

Layout B's blocks per fixture, independent per-block compression, `spikes/compress-checksum`, native x64.
`million` (1,954 blocks, mean 33,621 bytes):

whole-stream reference (one deflate stream over all blocks, no random access): level 1 40842192 bytes (0.622), level 6 35607726 bytes (0.542)

| codec | pre-filter | compressed bytes | ratio to raw | compress MB/s | decompress MB/s |
|---|---|---|---|---|---|
| deflate level 1 | none | 41008296 | 0.624 | 182 | 379 |
| deflate level 6 | none | 36716000 | 0.559 | 53 | 481 |
| LZ4 block | none | 46925070 | 0.714 | 689 | 4381 |
| zstd level 1 | none | 43228526 | 0.658 | 469 | 1254 |
| zstd level 3 | none | 37958243 | 0.578 | 334 | 1289 |
| deflate level 1 | shuffle8 | 48720731 | 0.742 | 318 | 478 |
| deflate level 6 | shuffle8 | 47720049 | 0.726 | 86 | 506 |
| LZ4 block | shuffle8 | 50960112 | 0.776 | 1666 | 5102 |
| zstd level 1 | shuffle8 | 48746596 | 0.742 | 798 | 1524 |
| zstd level 3 | shuffle8 | 48359956 | 0.736 | 538 | 1518 |

The shuffle pre-filter makes this corpus larger in every codec. The fixtures round coordinates to three
decimals and are plan-like, so this is not a finding about real drawings. In wasm32 on the same machine
(node 22), decompression of `million`'s blocks, MB/s:

| deflate1 | 41016120 bytes | decompress 287 MB/s | result ok |
| deflate1-shuf | 48728555 bytes | decompress 313 MB/s | result ok |
| lz4 | 46932894 bytes | decompress 3796 MB/s | result ok |
| lz4-shuf | 50967936 bytes | decompress 1589 MB/s | result ok |
| crc32 (crc32fast) | 3982 MB/s | 3d8a6ace57d |
| crc32c | 2881 MB/s | 3d37bef24e6 |
| xxh3-64 | 12856 MB/s | -636e70d7351d1993 |
| fnv-1a 64 | 1333 MB/s | -7fbb1203e5a93070 |
| sha-256 (first 8 bytes) | 507 MB/s | 2b0eb96cc211105e |

Checksums over the blocks, native:

| checksum | MB/s over the blocks (native) | ns per block |
|---|---|---|
| crc32 (crc32fast) | 16165 | 2080 |
| crc32c | 13067 | 2573 |
| xxh3-64 | 13649 | 2463 |
| fnv-1a 64 | 1304 | 25774 |
| sha-256 (first 8 bytes) | 2467 | 13627 |

All of the above is in `01-8-data/compress-checksum.txt` for the four fixtures, with the wasm runs for
`spline-heavy` and `million`. Round trips are asserted for every codec and filter in the tool, a 64 MB
decompression bomb claiming 1 KB is refused for deflate and LZ4 without being expanded, and every
checksum matches its published test vector in `spikes/compress-checksum/tests`.

## The ADRs

| ADR | decision | status |
|---|---|---|
| 0001 | A profile aligned to named ISO 10303-42 constructs, validated through STEP interchange; no conformance claim | Accepted |
| 0002 | One numeric policy: absolute, relative and ULP tolerance with a named scale term, twenty rules, counts as numbers | Accepted |
| 0003 | A custom block container with a directory; STEP-HDF and HDF5 rejected as the container, kept as a baseline | Accepted |
| 0004 | Layout B: type-batched SoA with an offset-table arena | Accepted |
| 0005 | A packed Hilbert R-tree, single payload ownership, small leaves sized in bytes, buckets as a valve | Accepted |
| 0006 | Per-block compression with a codec id and declared length, XXH3-64 per block, SHA-256 per artifact; codec not final | Accepted |
| 0007 | Tier 0 as in P1.7, delivered per index leaf; previews are display fidelity | Accepted |
| 0008 | Kind id plus must-understand flag per block, required capabilities in the header, unknown required rejects | Accepted |

## What I did not measure, and what to distrust

* arm64, anywhere. HTTP transfers, anywhere: ranges and bytes are counted, not downloaded.
* The fixtures are generated: plan-like, coordinates rounded to three decimals, no real DWG data and none of
  O1.5's or O1.6's probes (`pvec-gold-fixtures` held only a README). Every compression and layout margin
  that depends on coordinate statistics (the shuffle result above all) is to be re-run on P8.1's corpus.
* Decode and timing numbers move 10 to 20% between passes, and by a factor of three on one outlier cell in
  P1.5; read orders of magnitude, not digits.
* Decode to objects, as above, compares different implementations.
* `pvec-numeric`'s constants are starting values, loose by five orders of magnitude over measured
  evaluation error; the oracle comparison in phase 2 is where they get tested.
