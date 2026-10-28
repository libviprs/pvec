# Layout, index and compression benchmark (P8.3)

This measures the shipped container against the alternatives it beat in phase 1, on the pinned corpus, so that the P8.7
layout and index freeze is defended with production numbers and not spike numbers. It measures and reports only: nothing
here changes a layout, an index or a codec, and nothing re-opens the phase 1 selection. This file holds the re-run of P8.6 (pvec#65) at the final
configuration of the tuning pass (`docs/bench/TUNING.md` says what changed); the numbers in it are those of that run. This file holds the re-run of P8.6 (pvec#65) at the final
configuration of the tuning pass (`docs/bench/TUNING.md` says what changed); the numbers in it are those of that run. The shipped half is
`crates/pvec-bench/benches/layout.rs`; the alternatives that are not in the container run in the spikes on the same files
(`spikes/spatial-index corpus-index` for the index models and the block size sweep, `spikes/codec-sweep` for the codecs);
`tools/layout-bench.sh` drives all three on one host. The rows are in `docs/bench/layout.csv` (tidy, one number per row,
columns `table,entry,platform,arm,param,metric,value,note`), and `crates/pvec-bench/tests/issue_62_layout.rs` holds this
document to that file.

## Inputs and platforms

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`. Every arm runs on the same corpus
  digest and on the same file of each entry: the `shipped` arms read the `.pvec` the generator writes with no compression, the
  spikes read the shape dump of that same file. Entries: the six drawing entries of 10^5 entities and up, the 10^3 entries, and
  the mandatory `spline-heavy-1e5` / `spline-heavy-1e6` (they stress the arenas) and `huge-coordinate-1e5` (it stresses `f64` range).
  `architectural-1e7` is not part of this task: it is generated on demand and its other arms (Part 21, HDF) cannot run at that size.
* **native x64** is MARS (`linux/amd64` container, rustc 1.99.1, release). **arm64** is mercury (Apple silicon, the Colima VM,
  `linux/arm64`, same compiler). Every row names its platform and no verdict reads across the two. Both hosts were shared and
  lightly loaded in the re-run (load averages 1.6 to 2.2, one cpu of it another job's), so read throughput as ratios between arms of one table and not as absolute speed.
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

- soa-3x-record on linux-aarch64: PASS (architectural-1e6: SoA 7.713e8 entities/s against 1.231e8 for per-entity records, 6.27x (needs 3x))
- index-vs-best-alternative on linux-aarch64: FAIL (7 of 7 entries over the limit: architectural-1e5 (shipped 2.56 ranges and 2421696 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.94 ranges and 7804076 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 3.36 ranges and 3638682 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 3.38 ranges and 3283938 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 3.36 ranges and 4801017 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 7.14 ranges and 3921415 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 17.82 ranges and 12219822 bytes against the best alternatives' 8.34 and 5887375))
- index-share-15pct on linux-aarch64: PASS (9 entries, none above 15% of the file)
- block-size-within-10pct on linux-aarch64: FAIL (2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 633335736 bytes against 163488853, 287.4% over); architectural-1e6 (shipped 2609576624 bytes against 1078894209, 141.9% over))
- compression-crossover-25mbit on linux-aarch64: PASS (LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 5785 Mbit/s (link 25 Mbit/s))
- soa-3x-record on linux-x86_64: PASS (architectural-1e6: SoA 5.166e8 entities/s against 1.597e8 for per-entity records, 3.24x (needs 3x))
- index-vs-best-alternative on linux-x86_64: FAIL (7 of 7 entries over the limit: architectural-1e5 (shipped 2.56 ranges and 2421696 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.94 ranges and 7804076 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 3.36 ranges and 3638682 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 3.38 ranges and 3283938 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 3.36 ranges and 4801017 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 7.14 ranges and 3921415 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 17.82 ranges and 12219822 bytes against the best alternatives' 8.34 and 5887375))
- index-share-15pct on linux-x86_64: PASS (9 entries, none above 15% of the file)
- block-size-within-10pct on linux-x86_64: FAIL (2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 633335736 bytes against 163488853, 287.4% over); architectural-1e6 (shipped 2609576624 bytes against 1078894209, 141.9% over))
- compression-crossover-25mbit on linux-x86_64: PASS (LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 6359 Mbit/s (link 25 Mbit/s))

### What the verdicts say

This is the re-run of P8.6 at the final configuration; the first run is in the history of `main` and the changes in between are in `docs/bench/TUNING.md`. Three
things differ from the first run in how the shipped file is built, and they explain the numbers below: a leaf holds 32,768 entities (it was 64, so the mean hot
block is now 383 KB and not 0.9 KB), the advanced curves are cut in chunks of 256 KiB that a viewport reads only where they touch it (it read the whole set), and
the default file is compressed (deflate for hot blocks and Tier 0, LZ4 for records and arenas). The `shipped` rows below are still the file with no compression, so the
index and block size comparisons stay about layout; the `default` rows of `layout.csv` are the file the writer makes.

**Finding 4 is stated as total viewport I/O, which is range count plus bytes; it is not a duplication score.** The shipped index asks for fewer range reads than the best
alternative on 5 of 7 entries (architectural-1e5 2.6 against 4.7, architectural-1e6 6.9 against 8.8, mep-1e5 3.4 against 5.6, and so on) and for more on the two spline-heavy entries
(spline-heavy-1e5 7.1 against 4.1, spline-heavy-1e6 17.8 against 8.3). It moves more bytes than the best alternative on all seven, by a factor of
10.3 on `architectural-1e5`, 6.4 on `architectural-1e6`, 8.1 on `mep-1e5`, 7.3 on `industrial-1e5`, 9.0 on `huge-coordinate-1e5`, 4.2 on `spline-heavy-1e5`, 2.1 on `spline-heavy-1e6`. In the first run the factors were 3.1, 5.4, 2.8, 3.1, 5.9, 23.8, 37.7 in the same order. The bytes got much better where the advanced chunks are read
(`spline-heavy`) and worse where a leaf is read, because a viewport reads whole leaves of about 0.4 MB and the models read records of 16 to 512 KiB. That is the second half of the threshold
failing; the first half now passes on 5 entries and fails on 2.

**The index and the duplicated references are a small part of the file.** Index plus straddler lists come to between 0.000% and 0.354% of every file (the table below), so the
15% limit is never near. The shipped design keeps one owner for each payload, and the only duplicated references are the straddler lists.

| entry | index bytes | straddler lists | file bytes | share | mean hot block bytes |
|---|---|---|---|---|---|
| pathological-1e3 | 112 | 3828 | 1112144 | 0.35% | 7934 |
| architectural-1e3 | 112 | 0 | 49272 | 0.23% | 11766 |
| architectural-1e5 | 304 | 0 | 4852280 | 0.01% | 296172 |
| architectural-1e6 | 1600 | 0 | 48636496 | 0.00% | 382937 |
| mep-1e5 | 256 | 0 | 6064752 | 0.00% | 463769 |
| industrial-1e5 | 256 | 0 | 5998728 | 0.00% | 446983 |
| huge-coordinate-1e5 | 256 | 0 | 8613040 | 0.00% | 539030 |
| spline-heavy-1e5 | 208 | 0 | 21119504 | 0.00% | 279756 |
| spline-heavy-1e6 | 736 | 0 | 211624408 | 0.00% | 430451 |

**SoA against per-entity records: 3.24x on x64 and 6.27x on arm64 in this run, passing.** The shipped path decodes 517 million entities a second on the 10^6 architectural entry on
x64 against 160 million for records, and 771 million against 123 million on arm64. It was 2.01x on x64 in the first run: the bigger leaf is what moved it. The x64 ratio sits close to the limit and
moves with host load: the median of seven back-to-back runs on mars with nothing of mine next to them is 3.15x, with single runs from 2.95x to 3.20x, two of the seven under 3x.
arm64 is 4.86x median of seven (4.71x to 5.46x). The AoS arm is slower than the shipped SoA on the architectural entry on x64 (437 against 517 million a second); its rows keep each entity's numbers together,
which a checksum-everything pass rewards. This is a pass that touches every number; a viewport that wants only the bounds or only the x column would reward SoA, and this table does not measure that.

**The advanced arena path is the slow one.** Decoding splines through `AdvancedView::curve` runs 7.0, 6.9 million entities a second on x64 on the two spline entries, against 61, 37 million for one variable record per spline.
Each call re-validates the record and the offset table entries before it hands out slices, so the cost is the safety checks and not the memory layout. It was not changed.

**The shipped block size is still far from the sweep, and further than before.** The total bytes of all 200 viewports, uncompressed, are 633 MB shipped against 163 MB for the best swept arm (16 KiB blocks) at 10^5,
and 2,610 MB against 1,079 MB at 10^6: 287% and 142% over (it was 64% and 105%). The cause is the 32,768 entity leaf: a viewport reads whole leaves. The bytes the file stores for the same
viewports (the `default` rows) are 963 MB at 10^6, under the best arm's uncompressed 1,079 MB, and whether the verdict should count stored bytes is a decision for the freeze: the tuning did not change the metric.

**LZ4 pays for itself by a wide margin.** The lowest crossover over the 18 entries and scopes is 6359 Mbit/s on x64 and 5785 Mbit/s on arm64,
far past the 25 Mbit/s link. The table below is the codec sweep of the spikes on the same files.

## Decode throughput, millions of entities per second

| entry | platform | SoA (shipped) | per-entity record | AoS | arena (shipped) | per-entity variable |
|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 1291.5 | 172.5 | 1060.2 | 12.2 | 245.6 |
| architectural-1e5 | aarch64 | 1617.7 | 123.9 | 773.8 | 7.8 | 107.7 |
| architectural-1e6 | x86_64 | 516.6 | 159.7 | 436.6 | 13.0 | 129.2 |
| architectural-1e6 | aarch64 | 771.3 | 123.1 | 570.6 | 8.5 | 100.0 |
| mep-1e5 | x86_64 | 1080.4 | 113.3 | 560.9 | 11.2 | 158.6 |
| mep-1e5 | aarch64 | 1295.5 | 87.1 | 397.8 | 7.5 | 105.2 |
| industrial-1e5 | x86_64 | 1112.4 | 116.2 | 645.4 | 10.0 | 126.2 |
| industrial-1e5 | aarch64 | 1350.2 | 90.1 | 494.3 | 6.9 | 97.1 |
| huge-coordinate-1e5 | x86_64 | 888.3 | 98.4 | 505.7 | 8.6 | 88.0 |
| huge-coordinate-1e5 | aarch64 | 1035.8 | 77.8 | 387.8 | 6.1 | 69.4 |
| spline-heavy-1e5 | x86_64 | 1478.2 | 104.4 | 895.1 | 7.0 | 60.6 |
| spline-heavy-1e5 | aarch64 | 1388.6 | 83.7 | 460.9 | 5.4 | 48.5 |
| spline-heavy-1e6 | x86_64 | 824.5 | 101.0 | 446.8 | 6.9 | 36.8 |
| spline-heavy-1e6 | aarch64 | 728.5 | 82.3 | 368.8 | 5.4 | 51.2 |
| pathological-1e3 | x86_64 | 2165.6 | 259.0 | 1464.6 | 0.8 | 15.5 |
| pathological-1e3 | aarch64 | 1292.6 | 186.1 | 1085.0 | 0.7 | 11.3 |

## Mid-zoom viewport I/O, native x64: ranges / bytes, mean over the 50 mid-zoom viewports

| entry | platform | shipped | quadtree | rtree-512-64 | rtree-64-32 | cells-ref12 | cells-ref28 | buckets-7 | buckets-8 |
|---|---|---|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 2.6 / 2.4 MB | 8.5 / 496 KB | 5.1 / 314 KB | 4.7 / 234 KB | 19.4 / 958 KB | 13.4 / 597 KB | 5.3 / 285 KB | 6.3 / 310 KB |
| architectural-1e6 | x86_64 | 6.9 / 7.8 MB | 19.3 / 1.8 MB | 8.8 / 1.5 MB | 10.7 / 1.3 MB | 52.1 / 3.5 MB | 25.9 / 3.0 MB | 9.0 / 1.2 MB | 10.2 / 1.3 MB |
| mep-1e5 | x86_64 | 3.4 / 3.6 MB | 12.6 / 722 KB | 5.6 / 588 KB | 5.9 / 451 KB | 25.0 / 1.5 MB | 18.3 / 958 KB | 8.0 / 492 KB | 9.4 / 525 KB |
| industrial-1e5 | x86_64 | 3.4 / 3.3 MB | 10.1 / 633 KB | 5.7 / 618 KB | 5.8 / 452 KB | 22.3 / 1.4 MB | 15.0 / 902 KB | 7.6 / 484 KB | 8.5 / 509 KB |
| huge-coordinate-1e5 | x86_64 | 3.4 / 4.8 MB | 10.0 / 816 KB | 5.5 / 753 KB | 5.9 / 531 KB | 26.1 / 1.6 MB | 18.8 / 1.1 MB | 9.0 / 641 KB | 10.1 / 655 KB |
| spline-heavy-1e5 | x86_64 | 7.1 / 3.9 MB | 13.4 / 1.4 MB | 4.1 / 1.4 MB | 5.4 / 934 KB | 17.6 / 1.7 MB | 17.1 / 1.4 MB | 11.0 / 1.1 MB | 11.4 / 1.2 MB |
| spline-heavy-1e6 | x86_64 | 17.8 / 12.2 MB | 33.1 / 6.9 MB | 8.3 / 7.2 MB | 12.3 / 5.9 MB | 39.1 / 8.2 MB | 36.9 / 8.1 MB | 19.9 / 6.3 MB | 23.2 / 6.2 MB |

## Block size sweep: total bytes of all 200 viewports, native x64

| entry | platform | shipped | rtree-16 | rtree-64 | rtree-256 | rtree-1024 |
|---|---|---|---|---|---|---|
| architectural-1e5 | x86_64 | 633 MB | 163 MB | 221 MB | 359 MB | 572 MB |
| architectural-1e6 | x86_64 | 2610 MB | 1079 MB | 1177 MB | 1537 MB | 2491 MB |
| spline-heavy-1e5 | x86_64 | 1230 MB | 678 MB | 748 MB | 951 MB | 1440 MB |
| huge-coordinate-1e5 | x86_64 | 1130 MB | 299 MB | 366 MB | 563 MB | 959 MB |

## Compression, native x64, `architectural-1e6` and `spline-heavy-1e6`

| entry | scope | codec | ratio | compress MiB/s | decompress MiB/s | crossover Mbit/s (x64) |
|---|---|---|---|---|---|---|
| architectural-1e6 | block | none | 1.000 | 3464 | 12030 | - |
| architectural-1e6 | block | lz4 | 0.580 | 768 | 2420 | 8532 |
| architectural-1e6 | block | zstd3 | 0.399 | 428 | 1516 | 7645 |
| architectural-1e6 | block | zstd9 | 0.374 | 129 | 1680 | 8829 |
| architectural-1e6 | block | zstd19 | 0.295 | 11 | 1262 | 7471 |
| architectural-1e6 | file | none | 1.000 | 20294 | 6836 | - |
| architectural-1e6 | file | lz4 | 0.579 | 780 | 2199 | 7760 |
| architectural-1e6 | file | zstd3 | 0.384 | 397 | 1325 | 6840 |
| architectural-1e6 | file | zstd9 | 0.345 | 117 | 1393 | 7660 |
| architectural-1e6 | file | zstd19 | 0.290 | 5 | 1235 | 7356 |
| spline-heavy-1e6 | block | none | 1.000 | 9863 | 11670 | - |
| spline-heavy-1e6 | block | lz4 | 0.602 | 734 | 3352 | 11196 |
| spline-heavy-1e6 | block | zstd3 | 0.445 | 340 | 1591 | 7408 |
| spline-heavy-1e6 | block | zstd9 | 0.416 | 62 | 1671 | 8187 |
| spline-heavy-1e6 | block | zstd19 | 0.392 | 8 | 1317 | 6721 |
| spline-heavy-1e6 | file | none | 1.000 | 7427 | 3930 | - |
| spline-heavy-1e6 | file | lz4 | 0.600 | 718 | 2388 | 8010 |
| spline-heavy-1e6 | file | zstd3 | 0.433 | 321 | 1295 | 6155 |
| spline-heavy-1e6 | file | zstd9 | 0.409 | 83 | 1362 | 6753 |
| spline-heavy-1e6 | file | zstd19 | 0.382 | 4 | 1136 | 5884 |

The same tables for every other entry, and everything for arm64, are in `docs/bench/layout.csv`.

## What this does not do

It applies no result (P8.6 did, and is in `TUNING.md`), introduces no layout or index, does not re-open the phase 1 selection and changes no conformance
test or the semantic profile. The bytes of the alternative index models come from a byte model, not from files; that is the
phase 1 method, and the one place the comparison with the shipped arm is not like with like.
