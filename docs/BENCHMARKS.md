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

## Not measured

* The 4 hour nightly fuzz run of the hardening task (P7.8, pvec#190) is not measured here: the shorter runs are in [FUZZING.md](FUZZING.md), and the ones at the final tuning configuration are in `bench/TUNING.md`.
* `bench/PROGRESSIVE.md` (P8.5) and `bench/TUNING.md` (P8.6) are copied above (the P8.5 rows) and linked; the freeze decision about every red row is P8.7 (pvec#66), and it is not measured or made on this page.
