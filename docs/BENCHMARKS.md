# Benchmarks

This page consolidates the phase 8 results. It adds no measurement and amends none: every row is copied from the
report named in its first column, where the run, the command and the caveats are. Each row gives the threshold the
issue declared and the number that was measured (both in the last column, as the report words them), the platform, and
the verdict. A red row is a result and stays red here; the failures are tracked in pvec#192.

## Inputs

* Corpus: `crates/pvec-bench/benches/corpus/corpus.lock.json`, sha256 `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b`. Every number below was measured on that lock. The chunk ordering change of pvec#159 later moved 9 of its 10 generated entries, so pvec#196 added a second lock, `corpus.post159.lock.json`, for the current corpus, and kept this one because the reports cite it. The corpus is not frozen; see its row in [FREEZE-v1.md](FREEZE-v1.md).
* Platforms: `linux-x86_64` is native x64 (mars, a `linux/amd64` container, release build). `linux-aarch64` is arm64 (mercury, Colima on Apple silicon, release build). No row reads across the two, and no emulated result is reported.
* Reports: [P8.2](bench/STEP_COMPARISON.md), [P8.3](bench/LAYOUT.md), [P8.4](bench/GEOMETRY.md).

## Verdicts

| phase | threshold | platform | verdict | declared limit and measured number |
|---|---|---|---|---|
| P8.2 | `cold-open-10x` | linux-aarch64 (arm64) | PASS | architectural-1e5: Part 21 read 9717.8 ms against .pvec directory-ready 0.406 ms, 23921.2x (needs 10x) |
| P8.2 | `viewport-bytes-20pct` | linux-aarch64 (arm64) | PASS | architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286372662 byte Part 21 file (limit 20%) |
| P8.2 | `compressed-size-110pct` | linux-aarch64 (arm64) | PASS | 30 entries compared, none above 10% over STEP-HDF |
| P8.2 | `viewport-p95-vs-hdf` | linux-aarch64 (arm64) | PASS | 30 entries compared, .pvec p95 never above STEP-HDF's |
| P8.2 | `wasm-500kib` | linux-aarch64 (arm64) | PASS | decode artefact 83.4 KiB compressed (limit 500 KiB) |
| P8.2 | `step-hdf-footprint-reported` | linux-aarch64 (arm64) | PASS | browser footprint 149.1 KiB compressed |
| P8.2 | `cold-open-10x` | linux-x86_64 (native x64) | PASS | architectural-1e5: Part 21 read 7490.9 ms against .pvec directory-ready 0.504 ms, 14869.7x (needs 10x) |
| P8.2 | `viewport-bytes-20pct` | linux-x86_64 (native x64) | PASS | architectural-1e6: the largest recorded viewport moves 55056288 bytes, 19.23% of the 286373450 byte Part 21 file (limit 20%) |
| P8.2 | `compressed-size-110pct` | linux-x86_64 (native x64) | PASS | 31 entries compared, none above 10% over STEP-HDF |
| P8.2 | `viewport-p95-vs-hdf` | linux-x86_64 (native x64) | FAIL | 1 of 31 entries where .pvec p95 is above STEP-HDF's, as the ratio: spline-heavy-1e6 (10.732) |
| P8.2 | `wasm-500kib` | linux-x86_64 (native x64) | PASS | decode artefact 83.4 KiB compressed (limit 500 KiB) |
| P8.2 | `step-hdf-footprint-reported` | linux-x86_64 (native x64) | PASS | browser footprint 149.1 KiB compressed |
| P8.3 | `soa-3x-record` | linux-aarch64 (arm64) | PASS | architectural-1e6: SoA 3.780e8 entities/s against 9.731e7 for per-entity records, 3.88x (needs 3x) |
| P8.3 | `index-vs-best-alternative` | linux-aarch64 (arm64) | FAIL | 7 of 7 entries over the limit: architectural-1e5 (shipped 4.10 ranges and 725780 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.64 ranges and 6548435 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 4.82 ranges and 1271064 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 4.72 ranges and 1411261 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 4.82 ranges and 3121878 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 3.68 ranges and 22239314 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 6.24 ranges and 221854632 bytes against the best alternatives' 8.34 and 5887375) |
| P8.3 | `index-share-15pct` | linux-aarch64 (arm64) | PASS | 9 entries, none above 15% of the file |
| P8.3 | `block-size-within-10pct` | linux-aarch64 (arm64) | FAIL | 2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 267742768 bytes against 163488853, 63.8% over); architectural-1e6 (shipped 2213891512 bytes against 1078894209, 105.2% over) |
| P8.3 | `compression-crossover-25mbit` | linux-aarch64 (arm64) | PASS | LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 3691 Mbit/s (link 25 Mbit/s) |
| P8.3 | `soa-3x-record` | linux-x86_64 (native x64) | FAIL | architectural-1e6: SoA 3.095e8 entities/s against 1.539e8 for per-entity records, 2.01x (needs 3x) |
| P8.3 | `index-vs-best-alternative` | linux-x86_64 (native x64) | FAIL | 7 of 7 entries over the limit: architectural-1e5 (shipped 4.10 ranges and 725780 bytes against the best alternatives' 4.74 and 234020); architectural-1e6 (shipped 6.64 ranges and 6548435 bytes against the best alternatives' 8.78 and 1223623); mep-1e5 (shipped 4.82 ranges and 1271064 bytes against the best alternatives' 5.64 and 450653); industrial-1e5 (shipped 4.72 ranges and 1411261 bytes against the best alternatives' 5.68 and 451958); huge-coordinate-1e5 (shipped 4.82 ranges and 3121878 bytes against the best alternatives' 5.46 and 531023); spline-heavy-1e5 (shipped 3.68 ranges and 22239314 bytes against the best alternatives' 4.06 and 933592); spline-heavy-1e6 (shipped 6.24 ranges and 221854632 bytes against the best alternatives' 8.34 and 5887375) |
| P8.3 | `index-share-15pct` | linux-x86_64 (native x64) | PASS | 9 entries, none above 15% of the file |
| P8.3 | `block-size-within-10pct` | linux-x86_64 (native x64) | FAIL | 2 of 2 scales more than 10% off the best arm: architectural-1e5 (shipped 267742768 bytes against 163488853, 63.8% over); architectural-1e6 (shipped 2213891512 bytes against 1078894209, 105.2% over) |
| P8.3 | `compression-crossover-25mbit` | linux-x86_64 (native x64) | PASS | LZ4 wins on any link slower than its crossover; the lowest of 18 crossovers is 4132 Mbit/s (link 25 Mbit/s) |
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

## Not measured

* P8.5, progressive coverage (`bench/PROGRESSIVE.md`): not measured. The benchmark is not merged, so no stage time, coverage or preview error is claimed anywhere in the documentation.
* P8.6, tuning (`bench/TUNING.md`): not measured. Nothing has been tuned against the measurements above, so the defaults are the ones the format shipped with.

The 4 hour nightly fuzz run of the hardening task (P7.8) is also outstanding (pvec#190); the shorter runs are in [FUZZING.md](FUZZING.md).
