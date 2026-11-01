# Benchmarks

This page consolidates the phase 8 results, as re-measured after the P8.6 tuning pass (`bench/TUNING.md` has the before and after). It adds no measurement and amends none: every row is copied from the
report named in its first column, where the run, the command and the caveats are. Each row gives the threshold the
issue declared and the number that was measured (both in the last column, as the report words them), the platform, and
the verdict. A red row is a result and stays red here; the failures are tracked in pvec#192.

## Inputs

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`. Every number below was measured on that lock. The chunk ordering change of pvec#159 later moved 9 of its 10 generated entries, so pvec#196 added a second lock, `corpus.post159.lock.json`, for the current corpus, and kept this one because the reports cite it. The corpus is not frozen; see its row in [FREEZE-v1.md](FREEZE-v1.md).
* Platforms: `linux-x86_64` is native x64 (mars, a `linux/amd64` container, release build). `linux-aarch64` is arm64 (mercury, Colima on Apple silicon, release build). No row reads across the two, and no emulated result is reported.
* Reports: [P8.2](bench/STEP_COMPARISON.md), [P8.3](bench/LAYOUT.md), [P8.4](bench/GEOMETRY.md), [P8.5](bench/PROGRESSIVE.md); the tuning pass that moved them is [P8.6](bench/TUNING.md).

## Verdicts

| phase | threshold | platform | verdict | declared limit and measured number |
|---|---|---|---|---|
| P8.2 | `cold-open-10x` | linux-aarch64 (arm64) | PASS | architectural-1e5: Part 21 read 9717.8 ms against .pvec directory-ready 0.004 ms, 2535287.0x (needs 10x) |
| P8.2 | `viewport-bytes-20pct` | linux-aarch64 (arm64) | PASS | architectural-1e6: the largest recorded viewport moves 17828713 bytes, 6.23% of the 286372662 byte Part 21 file (limit 20%) |
| P8.2 | `compressed-size-110pct` | linux-aarch64 (arm64) | PASS | 30 entries compared, none above 10% over STEP-HDF |
| P8.2 | `viewport-p95-vs-hdf` | linux-aarch64 (arm64) | PASS | 30 entries compared, .pvec p95 never above STEP-HDF's |
| P8.2 | `wasm-500kib` | linux-aarch64 (arm64) | PASS | decode artefact 83.4 KiB compressed (limit 500 KiB) |
| P8.2 | `step-hdf-footprint-reported` | linux-aarch64 (arm64) | PASS | browser footprint 149.1 KiB compressed |
| P8.2 | `cold-open-10x` | linux-x86_64 (native x64) | PASS | architectural-1e5: Part 21 read 7490.9 ms against .pvec directory-ready 0.005 ms, 1449206.8x (needs 10x) |
| P8.2 | `viewport-bytes-20pct` | linux-x86_64 (native x64) | PASS | architectural-1e6: the largest recorded viewport moves 17828713 bytes, 6.23% of the 286373450 byte Part 21 file (limit 20%) |
| P8.2 | `compressed-size-110pct` | linux-x86_64 (native x64) | PASS | 31 entries compared, none above 10% over STEP-HDF |
| P8.2 | `viewport-p95-vs-hdf` | linux-x86_64 (native x64) | PASS | 31 entries compared, .pvec p95 never above STEP-HDF's |
| P8.2 | `wasm-500kib` | linux-x86_64 (native x64) | PASS | decode artefact 83.4 KiB compressed (limit 500 KiB) |
| P8.2 | `step-hdf-footprint-reported` | linux-x86_64 (native x64) | PASS | browser footprint 149.1 KiB compressed |
| P8.3 | `soa-3x-record` | linux-aarch64 (arm64) | PASS | architectural-1e6: SoA 7.713e8 entities/s against 1.231e8 for per-entity records, 6.27x (needs 3x) |
| P8.3 | `index-vs-best-alternative` | linux-aarch64 (arm64) | FAIL | 7 of 7 entries over the limit: architectural-1e5 (shipped 2.56 ranges and 2421696 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.94 ranges and 7804076 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 3.36 ranges and 3638682 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 3.38 ranges and 3283938 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 3.36 ranges and 4801017 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 7.14 ranges and 3921415 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 17.82 ranges and 12219822 bytes against the best alternatives' 8.34 and 5887375) |
| P8.3 | `index-share-15pct` | linux-aarch64 (arm64) | PASS | 9 entries, none above 15% of the file |
| P8.3 | `block-size-within-10pct` | linux-aarch64 (arm64) | FAIL | 2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 633335736 bytes against 163488853, 287.4% over); architectural-1e6 (shipped 2609576624 bytes against 1078894209, 141.9% over) |
| P8.3 | `compression-crossover-25mbit` | linux-aarch64 (arm64) | PASS | LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 5785 Mbit/s (link 25 Mbit/s) |
| P8.3 | `soa-3x-record` | linux-x86_64 (native x64) | PASS | architectural-1e6: SoA 5.166e8 entities/s against 1.597e8 for per-entity records, 3.24x (needs 3x) |
| P8.3 | `index-vs-best-alternative` | linux-x86_64 (native x64) | FAIL | 7 of 7 entries over the limit: architectural-1e5 (shipped 2.56 ranges and 2421696 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.94 ranges and 7804076 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 3.36 ranges and 3638682 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 3.38 ranges and 3283938 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 3.36 ranges and 4801017 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 7.14 ranges and 3921415 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 17.82 ranges and 12219822 bytes against the best alternatives' 8.34 and 5887375) |
| P8.3 | `index-share-15pct` | linux-x86_64 (native x64) | PASS | 9 entries, none above 15% of the file |
| P8.3 | `block-size-within-10pct` | linux-x86_64 (native x64) | FAIL | 2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 633335736 bytes against 163488853, 287.4% over); architectural-1e6 (shipped 2609576624 bytes against 1078894209, 141.9% over) |
| P8.3 | `compression-crossover-25mbit` | linux-x86_64 (native x64) | PASS | LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 6359 Mbit/s (link 25 Mbit/s) |
| P8.4 | `bspline3-point-at-150ns` | linux-aarch64 (arm64) | FAIL | point_at on the reference cubic 26.2 ns (limit 150 ns), P1.3 prototype 24.1 ns |
| P8.4 | `tessellate-one-allocation` | linux-aarch64 (arm64) | PASS | at most 1 allocations over 40 rows |
| P8.4 | `conic-halving-1.5x` | linux-aarch64 (arm64) | PASS | the largest growth over 4 halvings is 1.44x, ellipse from 1 px to 0.5 px: 64.0 to 92.0 segments (limit 1.5x) |
| P8.4 | `offset-1.2x-and-accuracy` | linux-aarch64 (arm64) | PASS | the dearest offset costs 1.00x (synthetic composite: 29.1 ns against 29.0 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere |
| P8.4 | `error-budget-never-exceeded` | linux-aarch64 (arm64) | PASS | 40 rows, none above its budget |
| P8.4 | `bspline3-point-at-150ns` | linux-x86_64 (native x64) | PASS | point_at on the reference cubic 18.0 ns (limit 150 ns), P1.3 prototype 18.7 ns |
| P8.4 | `tessellate-one-allocation` | linux-x86_64 (native x64) | PASS | at most 1 allocations over 40 rows |
| P8.4 | `conic-halving-1.5x` | linux-x86_64 (native x64) | PASS | the largest growth over 4 halvings is 1.44x, ellipse from 1 px to 0.5 px: 64.0 to 92.0 segments (limit 1.5x) |
| P8.4 | `offset-1.2x-and-accuracy` | linux-x86_64 (native x64) | PASS | the dearest offset costs 1.01x (synthetic trimmed: 14.7 ns against 14.5 ns; limit 1.2x); accuracy inside P1.2's tolerance everywhere |
| P8.4 | `error-budget-never-exceeded` | linux-x86_64 (native x64) | PASS | 40 rows, none above its budget |
| P8.5 | `directory-300ms` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 25mbit-60ms: 0.1865 s (limit 0.3 s) |
| P8.5 | `tier0-1500ms` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 25mbit-60ms: 0.7943 s (limit 1.5 s) |
| P8.5 | `tier0-4s-5mbit` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 5mbit-200ms: 3.5373 s (limit 4 s) |
| P8.5 | `coverage-complete` | linux-aarch64 (arm64) | PASS | 27 entry and arm rows, coverage 1.0 on every viewport |
| P8.5 | `preview-2px` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 25mbit-60ms: 1.7802 px (limit 2 px) |
| P8.5 | `preview-identity` | linux-aarch64 (arm64) | PASS | 27 rows, every preview replaces a named entity |
| P8.5 | `refine-5s` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 25mbit-60ms: 4.6357 s (limit 5 s) |
| P8.5 | `pan-discard-20pct` | linux-aarch64 (arm64) | PASS | architectural-1e6 on 25mbit-60ms: 0.0872 of decoded bytes (limit 0.2 of decoded bytes) |
| P8.5 | `repeat-spread-20pct` | linux-aarch64 (arm64) | PASS | 108 stage times, the widest spread 16.7% (limit 20%) |
| P8.5 | `directory-300ms` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 25mbit-60ms: 0.1864 s (limit 0.3 s) |
| P8.5 | `tier0-1500ms` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 25mbit-60ms: 0.7923 s (limit 1.5 s) |
| P8.5 | `tier0-4s-5mbit` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 5mbit-200ms: 3.5353 s (limit 4 s) |
| P8.5 | `coverage-complete` | linux-x86_64 (native x64) | PASS | 27 entry and arm rows, coverage 1.0 on every viewport |
| P8.5 | `preview-2px` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 25mbit-60ms: 1.7802 px (limit 2 px) |
| P8.5 | `preview-identity` | linux-x86_64 (native x64) | PASS | 27 rows, every preview replaces a named entity |
| P8.5 | `refine-5s` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 25mbit-60ms: 4.6193 s (limit 5 s) |
| P8.5 | `pan-discard-20pct` | linux-x86_64 (native x64) | PASS | architectural-1e6 on 25mbit-60ms: 0.0872 of decoded bytes (limit 0.2 of decoded bytes) |
| P8.5 | `repeat-spread-20pct` | linux-x86_64 (native x64) | FAIL | 5 of 108 stage times vary by more than 20%: architectural-1e3 local directory_s (161.2%); architectural-1e3 local refine_first_s (60.8%); architectural-1e3 local refine_full_s (60.8%); architectural-1e3 local tier0_s (65.1%); architectural-1e5 local directory_s (38.1%) |

## Rasters (pvec#220)

The first measurements of ADR 0018, which the ADR had only as arithmetic. They are new rows, not copies, and they sit beside the P8.5 report they extend. Rows: `bench/raster.csv`, produced by `tools/progressive-bench.sh mars OUT architectural-1e6 architectural-1e6+r1 architectural-1e6+r10`, and read by `crates/pvec-bench/tests/issue_220_raster_bench.rs`.

* Platform: `linux-x86_64`, native x64 on MARS (`linux/amd64` container `pvec-rust:1.99.1`, rustc 1.99.1, release build, 28 cores), 2026-10-31. Not re-run on arm64 (mercury): the arm64 column is empty and the verdicts below are x86_64 only.
* Conditions: the host is shared. The load average was 10.14, 45.54, 68.60 when the run started and 48.33, 120.52, 93.93 when it ended, with other jobs (and my own test runs) on the machine. The network arms are modelled as in P8.5 and the CPU part is measured, so the modelled numbers move little with load and the `local` CPU numbers move a lot; they are on the `local` rows of the CSV, not in the verdicts. Three repeat runs, median of the p95 over the 200 recorded viewports of `architectural-1e6`, the same windows for all three files.
* Fixture: `architectural-1e6` (the 1,000,000 entity corpus entry) with 0, 1 or 10 scanned sheets laid over its drawing. A sheet is 8192 by 6144 px at a 512 px tile edge, five levels, 257 tiles, bilevel PNG tiles of 33 KB from the test-only tiler (a stored deflate stream, so a tile is far heavier than the few hundred bytes a real bilevel scan compresses to: the Tier 0 and file-size numbers are on the heavy side). The coarsest level is inline in each index block.

| measurement | 0 rasters | 1 raster | 10 rasters | note |
|---|---|---|---|---|
| cold open bytes (`bytes_directory`, mean per viewport) | 19,952 | 20,456 | 24,344 | +504 bytes for the first raster, +4,392 for ten: 432 bytes of directory per raster (one index block and five chunk blocks, 72 bytes each with the checksum), and 72 once for the block of the placement stream |
| cold open time, 25 Mbit/s, 60 ms (`directory_s`, p95) | 0.18642 s | 0.18658 s | 0.18783 s | the P8.5 `directory-300ms` limit is 0.3 s |
| complete Tier 0, 25 Mbit/s (`tier0_s`, p95) | 0.79247 s | 0.80526 s | 0.86992 s | limit 1.5 s |
| complete Tier 0, 5 Mbit/s, 200 ms (`tier0_s`, p95) | 3.53546 s | 3.59962 s | 3.92290 s | `tier0-4s-5mbit`, limit 4 s |
| Tier 0 bytes (`bytes_tier0`, mean per viewport) | 739,071 | 778,699 | 976,859 | |
| raster index bytes in the Tier 0 plan of the fit-to-view | 0 | 39,628 | 237,768 | six of the ten index blocks fit the 262,144 byte cap |
| index bytes per tile (kind 9 block less its inline tiles, over every entry) | | 24.44 | 24.44 | the ADR estimated about 24 |

Requests per viewport on an A0 sheet at 300 dpi (14,043 by 9,933 px, 512 px tiles, 12,800 bytes a tile, full resolution, one device pixel per image pixel), viewports every 256 px over the sheet, `plan_raster` with the 64 KiB gap and the 16 MiB ceiling:

| screen | order | viewports | mean ranges | p95 | max |
|---|---|---|---|---|---|
| 1080p (1920 by 1080) | Hilbert | 1,680 | 2.31 | 4 | 5 |
| 1080p | row-major | 1,680 | 3.00 | 3 | 3 |
| 4K (3840 by 2160) | Hilbert | 1,240 | 3.82 | 5 | 5 |
| 4K | row-major | 1,240 | 5.00 | 5 | 5 |

The verdicts, re-run on `architectural-1e6+r10` (`raster_verdicts`, ids `directory-300ms-raster` and `tier0-4s-5mbit-raster`):

| phase | threshold | platform | verdict | declared limit and measured number |
|---|---|---|---|---|
| pvec#220 | `directory-300ms-raster` | linux-x86_64 (native x64) | PASS | architectural-1e6+r10 on 25mbit-60ms: 0.1878 s (limit 0.3 s) |
| pvec#220 | `tier0-4s-5mbit-raster` | linux-x86_64 (native x64) | PASS | architectural-1e6+r10 on 5mbit-200ms: 3.9229 s (limit 4 s) |

What I take from them. The Tier 0 cap of 256 KB (`RASTER_TIER0_CAP`) is kept: at the cap the 5 Mbit/s verdict passes with 77 ms to spare under ten sheets, and a bigger cap would take that headroom first. With ten sheets only six overviews are inside the cap; a real scan's overview is much smaller than this fixture's, so the cap bites later in practice, and a file whose overviews do not fit is still correct (the rest are fetched by the raster's own plan). `max_plan_decoded_bytes` (LIMIT-16, 256 MiB) is kept: the plan test shows it picks the level and the number of tiles, and what the decoder and the textures cost in a browser is not measured here (libviprs/maplibre-pvec#102 owns that). The Hilbert order lowers the mean requests per viewport by 23% at 1080p and 24% at 4K against row-major, and has the same p95 at 4K, but it has a worse tail at 1080p (p95 4 and max 5 against 3): I read that as worth keeping, with the tail as the cost.

## Not measured

* The 4 hour nightly fuzz run of the hardening task (P7.8, pvec#190) is not measured here: the shorter runs are in [FUZZING.md](FUZZING.md), and the ones at the final tuning configuration are in `bench/TUNING.md`.
* `bench/PROGRESSIVE.md` (P8.5) and `bench/TUNING.md` (P8.6) are copied above (the P8.5 rows) and linked; the freeze decision about every red row is P8.7 (pvec#66), and it is not measured or made on this page.

## Writer memory

Peak resident set of the facade writer against the entity count (issue #235). `memory` is
`PvecWriter::finish`, `spill` is `with_spill` (64 MiB budget, a spill root on local disk) and `finish_to` into
a sink that only hashes. Two-point polylines, ids from 0, release build, one configuration to a process
(`crates/pvec/examples/writer_memory.rs`), MARS native x86_64, `rust:1.99.1`, load average about 8 on 24
cores. The artifact is byte-identical between the two modes (same SHA-256 in every row pair).

| entities | writer | mode | peak RSS | artifact |
|---|---|---|---|---|
| 1,000,000 | plain | memory | 305 MiB | 2.86 MB |
| 1,000,000 | plain | spill | 166 MiB | 2.86 MB |
| 1,000,000 | indexed, Tier 0 | memory | 561 MiB | 3.06 MB |
| 1,000,000 | indexed, Tier 0 | spill | **85 MiB** | 3.06 MB |
| 4,000,000 | plain | memory | 1,210 MiB | 11.4 MB |
| 4,000,000 | plain | spill | 597 MiB | 11.4 MB |
| 4,000,000 | indexed, Tier 0 | memory | 2,160 MiB | 12.5 MB |
| 4,000,000 | indexed, Tier 0 | spill | **83 MiB** | 12.5 MB |

The indexed writer is the bounded one: 85 MiB at one million entities and 83 MiB at four million, against
2,160 MiB in memory, so doubling the entities twice moves the peak by less than 10 percent. The plain
writer without cells keeps every entity of one geometry type in one block, which is the whole drawing, so
its spill peak still grows with the count (597 MiB at four million, about half of the in-memory writer). A
writer that has to stay bounded is the indexed one (`docs/CONTAINER.md`, 4.8). Time: the spill path
finishes 4,000,000 indexed entities in 9.1 s against 3.8 s in memory.
