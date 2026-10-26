# Layout, index and compression benchmark (P8.3)

This measures the shipped container against the alternatives it beat in phase 1, on the pinned corpus, so that the P8.7
layout and index freeze is defended with production numbers and not spike numbers. It measures and reports only: nothing
here changes a layout, an index or a codec, and nothing re-opens the phase 1 selection. The shipped half is
`crates/pvec-bench/benches/layout.rs`; the alternatives that are not in the container run in the spikes on the same files
(`spikes/spatial-index corpus-index` for the index models and the block size sweep, `spikes/codec-sweep` for the codecs);
`tools/layout-bench.sh` drives all three on one host. The rows are in `docs/bench/layout.csv` (tidy, one number per row,
columns `table,entry,platform,arm,param,metric,value,note`), and `crates/pvec-bench/tests/issue_62_layout.rs` holds this
document to that file.

## Inputs and platforms

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`. Every arm runs on the same corpus
  digest and on the same file of each entry: the shipped arms read the `.pvec` the generator writes (no compression), the
  spikes read the shape dump of that same file. Entries: the six drawing entries of 10^5 entities and up, the 10^3 entries, and
  the mandatory `spline-heavy-1e5` / `spline-heavy-1e6` (they stress the arenas) and `huge-coordinate-1e5` (it stresses `f64` range).
  `architectural-1e7` is not part of this task: it is generated on demand and its other arms (Part 21, HDF) cannot run at that size.
* **native x64** is MARS (`linux/amd64` container, rustc 1.99.1, release). **arm64** is mercury (Apple silicon, the Colima VM,
  `linux/arm64`, same compiler). Every row names its platform and no verdict reads across the two. Both hosts were shared and
  loaded (load averages 5 to 14), so read throughput as ratios between arms of one table and not as absolute speed.
* `perf` is not available in the containers, so instructions and cache misses per entity are rows with a note and no number.
  Where the issue asks for them they are reported missing, not interpolated.

## What each arm is

* **Hot blocks.** `soa` is the shipped type-batched SoA block, scanned through `HotBlockView`. `record` is one variable-length
  record per entity back to back (tag, id, style, a count, then the numbers, unaligned little endian). `aos` is a row struct per kind
  with the vertices interleaved. All three produce the same checksum over the same entities (the harness asserts it), and the work
  is the same: touch every number once. Entities per second are over the hot entities; MiB per second over each layout's own bytes.
* **Advanced curves.** `arena` is the shipped record block and arenas with offset tables, read through `AdvancedView::curve`.
  `variable` is one variable-length record per spline. Same checksum, same work.
* **Index.** `shipped` is measured on the real reader: a fresh reader over the file for each of the 200 recorded viewports, open
  and directory read included, the planned blocks, the straddler payloads and the advanced set when it is in view, reads at most
  64 KiB apart merged. The alternatives are the P1.6 models from `spikes/spatial-index` (loose quadtree; packed Hilbert R-tree at
  512/64 and 64/32; hierarchical cells with 12-byte and 28-byte duplicated references; long-entity buckets at leaf level 7 and 8),
  which are an I/O simulation under that spike's byte model (8 bytes per `f64` and 5 of framing), with the same 64 KiB merge. They
  are checked against brute force for every viewport. The mid-zoom class is the eighth-of-a-side viewports of the recording (index
  2, 6, 10 and so on, 50 of the 200).
* **Block size.** The packed R-tree model with leaves of 16, 64, 256 and 1024 KiB of payload, against the shipped file, on the
  total bytes of all 200 viewports. The shipped file's hot blocks are about 0.9 KiB (64 entities per directory cell), below the
  swept range; there is no parameter in the writer to cut them larger, which is why the sweep is the model and not a rewritten file.
* **Compression.** `spikes/codec-sweep` takes the geometry blocks (hot, records, arenas) of the uncompressed file and compresses
  each block alone (as the container does) and then all of them as one buffer, with no compression, LZ4 (`lz4_flex`, the shipped
  codec in its safe mode), and Zstd at levels 3, 9 and 19. Zstd is a C library and is not in the container, so it lives in the
  spike. Times are the best of three runs (one for Zstd 19 on the biggest input).

## Thresholds

Declared in `tests/issue_62_layout.rs` and `src/layout_bench.rs` before the first run. Failure if:

* the shipped SoA immediate path decodes under 3x the entities per second of the per-entity-record arm on the 10^6 architectural entry;
* the shipped index needs more range reads than the best alternative for the same recorded mid-zoom viewports, or more than 1.25x its bytes;
* index plus duplicated references exceed 15% of the file on any entry;
* the shipped block size is more than 10% off the best measured arm on total viewport bytes at the 10^5 and 10^6 scales;
* LZ4 (the chosen setting, `CodecPolicy::RECOMMENDED`) costs more to decode than the range-read time it saves on a 25 Mbit/s link. The
  crossover is computed and printed: LZ4 saves `1 - ratio` of the bytes and decodes at `d` MiB/s, so it wins on any link slower than
  `(1 - ratio) * d * 8.388608` Mbit/s.

A missed threshold is a result and is recorded below. P8.7 decides what to do about each one.

## Results

- soa-3x-record on linux-aarch64: PASS (architectural-1e6: SoA 3.780e8 entities/s against 9.731e7 for per-entity records, 3.88x (needs 3x))
- index-vs-best-alternative on linux-aarch64: FAIL (7 of 7 entries over the limit: architectural-1e5 (shipped 4.10 ranges and 725780 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.64 ranges and 6548435 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 4.82 ranges and 1271064 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 4.72 ranges and 1411261 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 4.82 ranges and 3121878 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 3.68 ranges and 22239314 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 6.24 ranges and 221854632 bytes against the best alternatives' 8.34 and 5887375))
- index-share-15pct on linux-aarch64: PASS (9 entries, none above 15% of the file)
- block-size-within-10pct on linux-aarch64: FAIL (2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 267742768 bytes against 163488853, 63.8% over); architectural-1e6 (shipped 2213891512 bytes against 1078894209, 105.2% over))
- compression-crossover-25mbit on linux-aarch64: PASS (LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 3691 Mbit/s (link 25 Mbit/s))
- soa-3x-record on linux-x86_64: FAIL (architectural-1e6: SoA 3.095e8 entities/s against 1.539e8 for per-entity records, 2.01x (needs 3x))
- index-vs-best-alternative on linux-x86_64: FAIL (7 of 7 entries over the limit: architectural-1e5 (shipped 4.10 ranges and 725780 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.64 ranges and 6548435 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 4.82 ranges and 1271064 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 4.72 ranges and 1411261 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 4.82 ranges and 3121878 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 3.68 ranges and 22239314 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 6.24 ranges and 221854632 bytes against the best alternatives' 8.34 and 5887375))
- index-share-15pct on linux-x86_64: PASS (9 entries, none above 15% of the file)
- block-size-within-10pct on linux-x86_64: FAIL (2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 267742768 bytes against 163488853, 63.8% over); architectural-1e6 (shipped 2213891512 bytes against 1078894209, 105.2% over))
- compression-crossover-25mbit on linux-x86_64: PASS (LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 4132 Mbit/s (link 25 Mbit/s))

### What the verdicts say

**Finding 4 is stated as total viewport I/O, which is range count plus bytes; it is not a duplication score.** The shipped index asks for
fewer range reads than every alternative on all seven entries (4.1 against 4.7 at best on `architectural-1e5`, 6.6 against 8.8 on
`architectural-1e6`, 3.7 against 4.1 on `spline-heavy-1e5`), and it moves more bytes than the best alternative on all seven, by a
factor of 3.1 on `architectural-1e5` and 5.4 on `architectural-1e6`, 5.9 on `huge-coordinate-1e5`, and 24 to 38 on the two spline-heavy entries. That is the
second half of the threshold failing and the first half passing. Three things make the bytes:

* the reader coalesces planned blocks across gaps of up to 64 KiB (`ReaderConfig::gap_threshold`), and the hot blocks are about
  0.9 KiB each, so a request pulls in the blocks between the wanted ones. It buys the range count and pays the bytes;
* every open reads the whole spatial directory in one range (77 KB at 10^5, 767 KB at 10^6, 1.4% of the file), which is a
  fixed cost per viewport on a cold reader, where the models count only the index records a query touches;
* the advanced curve set is read whole whenever it is in view (`docs/STREAMING.md` 2.8), and in `spline-heavy-1e5` and `-1e6`
  that is nearly the entire file: 22.2 MB and 221.9 MB for one mid-zoom viewport, against 0.9 and 5.9 MB for the best model.

**The index and the duplicated references are a small part of the file.** Index plus straddler lists come to between 0.13% and 1.58% of
every file (the table below), so the 15% limit is never near. The shipped design keeps one owner for each payload, and the only
duplicated references are the straddler lists, 1.5 KB to 4.6 KB. The cell models, which duplicate a reference in every cell an
entity touches, come out at 2.0 to 2.6 references per entity and 3.6 MB to 64 MB of index (`index_bytes` in the CSV), which is the
reason they also lose on bytes and ranges in the table above.

| entry | index bytes | straddler lists | file bytes | share | mean hot block bytes |
|---|---|---|---|---|---|
| pathological-1e3 | 688 | 3828 | 1116040 | 0.40% | 1074 |
| architectural-1e3 | 880 | 0 | 55640 | 1.58% | 857 |
| architectural-1e5 | 76864 | 1544 | 5492896 | 1.43% | 925 |
| mep-1e5 | 75040 | 4400 | 6756960 | 1.18% | 1006 |
| industrial-1e5 | 74992 | 4400 | 6731160 | 1.18% | 921 |
| huge-coordinate-1e5 | 71392 | 1544 | 9375352 | 0.78% | 1176 |
| spline-heavy-1e5 | 31168 | 0 | 24413992 | 0.13% | 926 |
| architectural-1e6 | 766960 | 4576 | 55052464 | 1.40% | 924 |
| spline-heavy-1e6 | 309568 | 0 | 244437392 | 0.13% | 930 |

**SoA against per-entity records: 2.0x on x64, 3.9x on arm64.** The shipped path decodes 309 million entities a second on the 10^6
architectural entry on x64 against 154 million for records, which is 2.01x and under the 3x limit, and 378 million against 97
million on arm64, 3.9x. At 10^5 the x64 ratio is 3.9x, so the gap narrows as the working set outgrows the cache: the record arm
streams one sequential buffer, which the prefetcher likes, and the SoA arm streams five or more columns. The AoS arm is faster than
the shipped SoA on the architectural entries (404 against 310 million a second at 10^6 on x64) and about level with it on arm64 (385 against 378); its
rows keep each entity's numbers together, which a checksum-everything pass rewards. This is a pass that touches every number; a
viewport that wants only the bounds or only the x column would reward SoA, and this table does not measure that.

**The advanced arena path is the slow one.** Decoding splines through `AdvancedView::curve` runs at 6.7 to 12 million entities a
second on x64, against 34 to 258 million for one variable record per spline: 5x to 21x slower. Each call re-validates the record
and the offset table entries before it hands out slices, so the cost is the safety checks and not the memory layout. It is the
largest ratio in this document and it is on the path the spline-heavy entries live on.

**The shipped block size is far from the sweep.** The total bytes of all 200 viewports are 268 MB shipped against 163 MB for the best
swept arm (16 KiB blocks) at 10^5, and 2,214 MB against 1,079 MB at 10^6: 64% and 105% over. Smaller blocks win throughout the sweep
(16 KiB best, 1024 KiB worst), so a block of 0.9 KiB should win more, and does not, because the reader's 64 KiB gap
coalescing and the per-viewport directory read cost more than the smaller blocks save. The sweep does not contain the shipped size, so the
conclusion is about the reader and the directory and not about a block size of 16 KiB.

**LZ4 pays for itself by a wide margin.** The lowest crossover over the 18 entries and scopes is 3,691 Mbit/s on arm64 and 4,132
Mbit/s on x64, a hundred and fifty times the 25 Mbit/s link, because LZ4 decodes at 1.5 to 3 GiB/s and saves 37% to 46% of the
geometry bytes. Zstd at 3 saves more (ratio 0.40 to 0.56) and decodes at 0.4 to 1.5 GiB/s, still far past the link; Zstd 19 takes
4 to 8 MiB/s to compress, which is a write-side cost. Per-block compression costs ratio against whole-file (LZ4 0.625 against
0.539 on `architectural-1e6`; Zstd 3 0.556 against 0.401), because 0.9 KiB blocks do not give a compressor much to work with.

## Decode throughput, millions of entities per second

| entry | platform | SoA (shipped) | per-entity record | AoS | arena (shipped) | per-entity variable |
|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 671.5 | 170.0 | 1033.1 | 12.2 | 257.5 |
| architectural-1e5 | aarch64 | 400.2 | 107.3 | 682.3 | 6.3 | 74.2 |
| architectural-1e6 | x86_64 | 309.5 | 153.9 | 403.8 | 12.3 | 125.1 |
| architectural-1e6 | aarch64 | 378.0 | 97.3 | 385.4 | 6.6 | 77.7 |
| mep-1e5 | x86_64 | 564.6 | 111.1 | 539.3 | 10.6 | 137.3 |
| mep-1e5 | aarch64 | 261.1 | 65.6 | 296.5 | 5.6 | 75.8 |
| industrial-1e5 | x86_64 | 540.9 | 114.9 | 662.0 | 9.7 | 114.1 |
| industrial-1e5 | aarch64 | 270.6 | 72.8 | 363.0 | 5.3 | 75.7 |
| huge-coordinate-1e5 | x86_64 | 472.7 | 96.9 | 485.4 | 7.9 | 83.2 |
| huge-coordinate-1e5 | aarch64 | 300.0 | 67.3 | 334.6 | 5.1 | 58.5 |
| spline-heavy-1e5 | x86_64 | 641.0 | 102.4 | 883.4 | 6.7 | 48.0 |
| spline-heavy-1e5 | aarch64 | 310.1 | 67.7 | 362.0 | 4.5 | 45.9 |
| spline-heavy-1e6 | x86_64 | 328.7 | 98.2 | 399.2 | 6.7 | 33.9 |
| spline-heavy-1e6 | aarch64 | 280.4 | 79.2 | 346.7 | 5.0 | 48.6 |
| pathological-1e3 | x86_64 | 1054.6 | 263.0 | 1459.8 | 0.8 | 15.6 |
| pathological-1e3 | aarch64 | 366.7 | 125.2 | 814.0 | 0.5 | 8.0 |

## Mid-zoom viewport I/O, native x64: ranges / bytes, mean over the 50 mid-zoom viewports

| entry | platform | shipped | quadtree | rtree-512-64 | rtree-64-32 | cells-ref12 | cells-ref28 | buckets-7 | buckets-8 |
|---|---|---|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 4.1 / 726 KB | 8.5 / 496 KB | 5.1 / 314 KB | 4.7 / 234 KB | 19.4 / 958 KB | 13.4 / 597 KB | 5.3 / 285 KB | 6.3 / 310 KB |
| architectural-1e6 | x86_64 | 6.6 / 6.5 MB | 19.3 / 1.8 MB | 8.8 / 1.5 MB | 10.7 / 1.3 MB | 52.1 / 3.5 MB | 25.9 / 3.0 MB | 9.0 / 1.2 MB | 10.2 / 1.3 MB |
| mep-1e5 | x86_64 | 4.8 / 1.3 MB | 12.6 / 722 KB | 5.6 / 588 KB | 5.9 / 451 KB | 25.0 / 1.5 MB | 18.3 / 958 KB | 8.0 / 492 KB | 9.4 / 525 KB |
| industrial-1e5 | x86_64 | 4.7 / 1.4 MB | 10.1 / 633 KB | 5.7 / 618 KB | 5.8 / 452 KB | 22.3 / 1.4 MB | 15.0 / 902 KB | 7.6 / 484 KB | 8.5 / 509 KB |
| huge-coordinate-1e5 | x86_64 | 4.8 / 3.1 MB | 10.0 / 816 KB | 5.5 / 753 KB | 5.9 / 531 KB | 26.1 / 1.6 MB | 18.8 / 1.1 MB | 9.0 / 641 KB | 10.1 / 655 KB |
| spline-heavy-1e5 | x86_64 | 3.7 / 22.2 MB | 13.4 / 1.4 MB | 4.1 / 1.4 MB | 5.4 / 934 KB | 17.6 / 1.7 MB | 17.1 / 1.4 MB | 11.0 / 1.1 MB | 11.4 / 1.2 MB |
| spline-heavy-1e6 | x86_64 | 6.2 / 221.9 MB | 33.1 / 6.9 MB | 8.3 / 7.2 MB | 12.3 / 5.9 MB | 39.1 / 8.2 MB | 36.9 / 8.1 MB | 19.9 / 6.3 MB | 23.2 / 6.2 MB |

## Block size sweep: total bytes of all 200 viewports, native x64

| entry | platform | shipped | rtree-16 | rtree-64 | rtree-256 | rtree-1024 |
|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 268 MB | 163 MB | 221 MB | 359 MB | 572 MB |
| architectural-1e6 | x86_64 | 2214 MB | 1079 MB | 1177 MB | 1537 MB | 2491 MB |
| spline-heavy-1e5 | x86_64 | 4494 MB | 678 MB | 748 MB | 951 MB | 1440 MB |
| huge-coordinate-1e5 | x86_64 | 772 MB | 299 MB | 366 MB | 563 MB | 959 MB |

## Compression, native x64, `architectural-1e6` and `spline-heavy-1e6`

| entry | scope | codec | ratio | compress MiB/s | decompress MiB/s | crossover Mbit/s (x64) |
|---|---|---|---|---|---|---|
| architectural-1e6 | block | none | 1.000 | 7872 | 7802 | - |
| architectural-1e6 | block | lz4 | 0.625 | 762 | 2014 | 6340 |
| architectural-1e6 | block | zstd3 | 0.556 | 144 | 468 | 1746 |
| architectural-1e6 | block | zstd9 | 0.549 | 70 | 472 | 1784 |
| architectural-1e6 | block | zstd19 | 0.504 | 8 | 455 | 1890 |
| architectural-1e6 | file | none | 1.000 | 7100 | 6101 | - |
| architectural-1e6 | file | lz4 | 0.539 | 712 | 2260 | 8747 |
| architectural-1e6 | file | zstd3 | 0.401 | 370 | 1148 | 5774 |
| architectural-1e6 | file | zstd9 | 0.362 | 95 | 1247 | 6675 |
| architectural-1e6 | file | zstd19 | 0.325 | 4 | 1151 | 6521 |
| spline-heavy-1e6 | block | none | 1.000 | 10828 | 10826 | - |
| spline-heavy-1e6 | block | lz4 | 0.578 | 747 | 3090 | 10936 |
| spline-heavy-1e6 | block | zstd3 | 0.450 | 299 | 1187 | 5482 |
| spline-heavy-1e6 | block | zstd9 | 0.428 | 66 | 1246 | 5981 |
| spline-heavy-1e6 | block | zstd19 | 0.414 | 7 | 1025 | 5043 |
| spline-heavy-1e6 | file | none | 1.000 | 8289 | 4603 | - |
| spline-heavy-1e6 | file | lz4 | 0.562 | 753 | 2699 | 9919 |
| spline-heavy-1e6 | file | zstd3 | 0.399 | 356 | 1450 | 7307 |
| spline-heavy-1e6 | file | zstd9 | 0.374 | 90 | 1552 | 8149 |
| spline-heavy-1e6 | file | zstd19 | 0.347 | 4 | 1173 | 6421 |

The same tables for every other entry, and everything for arm64, are in `docs/bench/layout.csv`.

## What this does not do

It applies no result (P8.6), introduces no layout or index, does not re-open the phase 1 selection and changes no conformance
test or the semantic profile. The bytes of the alternative index models come from a byte model, not from files; that is the
phase 1 method, and the one place the comparison with the shipped arm is not like with like.
