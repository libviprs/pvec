# Tuning log: the index and viewport rows (P8.6, pvec#65 and pvec#192)

This is the part of the P8.6 log that belongs to the viewport and the index. It is a fragment on purpose: it has the rows I
added on `tune-index`, the sweeps behind them and the parameters I measured and left alone, so it merges into the
log of `feat/65-tune-192` as rows, not as a second document.

## Rules

The rule of this log is that a parameter with no measurement behind it is not tuned here. A row names a metric id that P8.2 to P8.5
measured, the numbers before and after on both platforms, the digest of the corpus they were taken on and the result of
the conformance and security re-run. Tuning does not relax a P1.2 tolerance, a P4.8 limit, the decompression expansion
ratio, a checksum, a conformance test (none is deleted or weakened) or the Tier 0 coverage requirement.

Numbers are native x64 (mars, `linux-x86_64`) and native arm64 (mercury, `linux-aarch64`), never read across the two. Both
hosts were shared, so a time is the median of three runs taken back to back, and a byte or request count is exact.

## Changes

| # | parameter | old | new | metric | before | after | corpus digest | re-run |
|---|---|---|---|---|---|---|---|---|
| 1 | `rule: a viewport fetches only the advanced chunks whose record block touches it, in one plan (ReadPlan::advanced_chunks, ContainerReader::advanced_chunk_blocks, the chunk table kept per reader)` | the whole advanced set whenever its union touched the viewport, one plan per chunk | the chunks whose bounds touch the viewport, with the hot blocks' plan, the table worked out once | `viewport-p95-vs-hdf`, `index-vs-best-alternative` | spline-heavy-1e6, 1 MiB chunks: x64 p95 164.1 ms, p50 141.5 ms, 116.7 MB at p95, 255 requests at most; arm64 p95 203.4 ms, p50 175.5 ms, same bytes and requests | x64 p95 102.3 ms, p50 29.1 ms, 62.3 MB at p95, 36 requests at most; arm64 p95 131.3 ms, p50 36.5 ms, same bytes and requests | `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b` | pass: P2.5 10, P2.6 10, P2.8 10, P2.7 28 and 2, P4.8 25, P7.8 matrix 5, P7.8 fuzz 27 (x64 mars and arm64 mercury, `index-final`) |
| 2 | `pvec_container::writer::ADVANCED_CHUNK_BYTES` | 1048576 | 262144 | `viewport-p95-vs-hdf`, `index-vs-best-alternative` | spline-heavy-1e6 after row 1, 1 MiB chunks: x64 p95 102.3 ms, 62.3 MB at p95, file 118,910,464 bytes; arm64 p95 131.3 ms, same bytes | x64 p95 82.0 ms, p50 24.3 ms, 45.6 MB at p95, 66 requests at most, file 122,753,728 bytes (+3.2%); arm64 p95 103.8 ms, p50 30.8 ms, same bytes | `9df09435a433a1d2ba605a10bc4c59d6f8997f39a116c12aad64f93a5625e25b` | pass: P2.5 10, P2.6 10, P2.8 10, P2.7 28 and 2, P4.8 25, P7.8 matrix 5, P7.8 fuzz 27 (x64 mars and arm64 mercury, `index-final`) |

Row 1 is a rule and not a number, so the test that holds the defaults to this log skips it; `issue_148_chunked_advanced` (the plan
names exactly the chunks that touch the viewport) and `issue_192_chunk_blocks` (the six blocks of a chunk, one plan for
the lot, the table built once) hold it. Row 2 is held by `issue_192_chunk_default` and by the default test of `pvec-bench`.

What each row bought, on the other files of the corpus (x64 / arm64, base is the tip of `feat/65-tune-192`, after is
`index-final`, same host and day, `tools/tune-check.sh`):

| metric | base x64 | after x64 | base arm64 | after arm64 |
|---|---|---|---|---|
| spline-heavy-1e5 viewport p95 (ms) | 18.33 | 12.07 (-34.1%) | 25.57 | 15.11 (-40.9%) |
| spline-heavy-1e5 bytes at p95 | 11,968,154 | 7,215,449 (-39.7%) | 11,968,154 | 7,215,449 (-39.7%) |
| spline-heavy-1e5 requests at most | 29 | 16 (-44.8%) | 29 | 16 (-44.8%) |
| architectural-1e6 viewport p95 (ms) | 68.27 | 68.42 (+0.2%) | 89.47 | 85.32 (-4.6%) |
| architectural-1e6 total viewport bytes, 200 viewports, stored | 1,025,074,464 | 962,507,059 (-6.1%) | 1,025,074,464 | 962,507,059 (-6.1%) |
| architectural-1e6 total viewport bytes, uncompressed layout | 2,711,874,528 | 2,609,576,624 (-3.8%) | 2,711,874,528 | 2,609,576,624 (-3.8%) |
| architectural-1e6 time to complete Tier 0, 25 Mbit/s (s) | 0.8187 | 0.7923 (-3.2%) | 0.8208 | 0.7946 (-3.2%) |
| huge-coordinate-1e5 time to complete Tier 0, 25 Mbit/s (s) | 0.9991 | 0.9306 (-6.9%) | 1.0012 | 0.9328 (-6.8%) |
| ns per segment, mean of 428 rows | 359.0 | 359.7 (+0.2%) | 663.1 | 523.5 (noise, geometry is untouched) |

Three things moved the wrong way by more than 5%, and none is one of the four headline metrics:

* the file of `spline-heavy-1e5` and `-1e6` grows 3.2% (12,341,936 against 11,964,360 bytes; 122,753,728 against 118,910,464),
  because a chunk is six blocks and four times as many chunks is four times the block headers and four times the
  compression frames. 64 KiB chunks would cost 11% (132,540,976 bytes) for no gain in time (table below);
* requests: the 200 viewports of `architectural-1e6` make 1,239 against 1,162 (+6.6%), because its 10^4 splines are 13 chunks
  now and not 4. The bytes they move are 3.8% fewer. `spline-heavy-1e5` at the shipped index measure goes from 3.68 to 7.14
  requests for a mid-zoom viewport, and from 22,239,314 to 3,921,415 bytes (-82%);
* the cold open of `spline-heavy-1e5` takes 0.023 ms against 0.010 ms, which is 13 microseconds for the block directory of
  four times as many blocks.

The p95 of `spline-heavy-1e6` is 82.0 ms on x64, against 297 ms for STEP-HDF (`viewport-p95-vs-hdf` passes; it was 3,189 ms
in the P8.2 run, 172 ms at the tip of this branch).

## The sweeps behind the rows

### Advanced chunk size

Each viewport priced on a link as `requests x RTT + bytes x 8 / rate`, one request at a time, over the 200 recorded viewports,
on the file the writer defaults make (LZ4), reads at most 64 KiB apart merged. Mean seconds a viewport:

| chunk | `spline-heavy-1e5` 25 Mbit/s, 60 ms | 5 Mbit/s, 200 ms | file bytes | `spline-heavy-1e6` 25 Mbit/s, 60 ms | 5 Mbit/s, 200 ms | file bytes |
|---|---|---|---|---|---|---|
| 64 KiB | 1.488 | 6.513 | 13,262,696 | 7.262 | 33.606 | 132,540,976 |
| 256 KiB | 1.535 | 6.881 | 12,341,936 | 7.156 | 33.186 | 122,753,728 |
| 512 KiB | not run | not run | not run | 7.656 | 35.973 | 120,292,000 |
| 1 MiB | 1.646 | 7.934 | 11,964,360 | 8.611 | 41.441 | 118,910,464 |
| 4 MiB | 2.013 | 9.799 | 11,892,408 | not run | not run | not run |

256 KiB is the cheapest at 10^6 and within 3% of the cheapest at 10^5, with a file 3.2% over the 1 MiB one. 64 KiB buys nothing at 10^6 and costs 11%.

### Range-coalescing window (`ReaderConfig::gap_threshold`)

The same pricing, the same files, the reads of a cold reader with no merging, merged at each gap afterwards (the reader and
`merge` use the same rule). Mean seconds a viewport at 25 Mbit/s and 60 ms, requests one at a time:

| gap | architectural-1e5 | architectural-1e6 | spline-heavy-1e5 | spline-heavy-1e6 (1 MiB chunks) |
|---|---|---|---|---|
| 0 | 1.130 | 3.646 | 4.761 | 24.143 |
| 16 KiB | 0.510 | 1.946 | 1.646 | 8.611 |
| 64 KiB | 0.508 | 1.945 | 1.646 | 8.611 |
| 256 KiB | 0.509 | 1.949 | 1.646 | 8.611 |
| 1 MiB | 0.639 | 2.102 | 1.684 | 9.986 |
| 4 MiB | 0.639 | 3.393 | 2.233 | 10.910 |

The cost is flat from 16 KiB to 256 KiB: a leaf is 0.4 MB now and the chunks are 0.25 to 1 MiB, so the gaps between wanted blocks
are either a few bytes (they merge at any window) or megabytes (they never do). 64 KiB is in the middle of that plateau, and
a larger window only adds cost (1 MiB: +8% on `architectural-1e6`, +16% on `spline-heavy-1e6`).

## Not tuned

* `pvec_container::ReaderConfig::gap_threshold` stays at 65536: the sweep above is flat around it, so there is nothing to buy.
* `pvec_stream::FANOUT` stays at 32: the directory of the million-entity files is 2 levels and a few kilobytes at 32,768 entities
  a leaf (`index_bytes` in `layout.csv`), and no benchmark moves with it.

## Where this leaves the thresholds I was given (x64, the x64 file of `spline-heavy-1e6` and the rows of `layout.csv`)

* `viewport-p95-vs-hdf` on `spline-heavy-1e6`: resolved. 82.0 ms against 297 ms (x64); arm64 103.8 ms, STEP-HDF was not run there.
* `index-vs-best-alternative`: improved, not met. Mid-zoom bytes on `spline-heavy-1e6` are 12,219,822 against 5,887,375 for the best
  alternative (2.1x; it was 221,854,632, 38x), `spline-heavy-1e5` 3,921,415 against 933,592 (4.2x; it was 24x). Requests are 17.82
  against 8.34 and 7.14 against 4.06, which is more than the best model's now. `architectural-1e6` is 7,804,076 bytes against
  1,223,623 (6.4x) and 6.94 requests against 8.78. The cause that is left is the leaf: a viewport reads whole leaves of about
  0.4 MB, and the models read 16 to 512 KiB records.
* `block-size-within-10pct`: open, and further away than before. The shipped total of `architectural-1e6` is 2,609,576,624 bytes
  against 1,078,894,209 (142% over; it was 105%) and `architectural-1e5` 633,335,736 against 163,488,853 (287% over; it was 64%).
  The leaf of 32,768 entities (the P8.5 directory and Tier 0 fix) is what costs it, since the verdict counts uncompressed
  bytes. The stored bytes of the file the writer makes are 962,507,059, under the best swept arm's uncompressed 1,078,894,209.
  Whether the verdict should count stored bytes is a decision for Rom.
* `soa-3x-record` on x64: 3.20x in six quiet runs back to back (5.14 to 5.17e8 against 1.61e8 entities a second), and 2.9x to 3.35x
  across five full `tune-check` runs while the host was busy. Nothing in this branch touches the hot block decode, so the
  ratio is the leaf's and the load's. arm64 is 4.3x to 5.5x.
