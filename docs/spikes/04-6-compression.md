# P4.6: per-block compression and artifact integrity

This is the measurement that issue #7 asks for and the decision I wrote down beside it. ADR 0006 fixed the
structure (independent blocks, a codec id and the decoded length in the directory entry, a per-block XXH3-64,
an artifact digest) and left the codec open until this task. Amendment 2026-10-24 of that ADR records what
changed. The code is `crates/pvec-container/src/compress.rs` and `integrity.rs`, and the format is
`docs/CONTAINER.md` section 4.7.

## What was measured, and on what

`crates/pvec-testkit/tests/p4_6_codec_measurement.rs`, an `#[ignore]` test so it is a measurement and not a
gate, run in release mode:

```
cargo test --release -p pvec-testkit --test p4_6_codec_measurement -- --ignored --nocapture
```

Four named fixtures, each cut into blocks of about 512 entities (the block size ADR 0006 measured):

* `plan-like-200k`: 200,000 entities, five in eight lines, the rest polylines of 3 to 10 vertices and
  circular arcs, coordinates on a grid of eighths. Its coordinates are drawn at random from a 10,000 by
  7,500 box, so it is a hard case for a compressor: nothing in it repeats except the structure.
* `spline-heavy-20k`: 20,000 lines and 20,000 B-splines (a third of them rational, degree 2 to 4, knot
  vectors that repeat), so the record block and the five arenas are real blocks.
* `with-streams-50k`: 50,000 lines with a layer stream that names every entity and a text stream, so the
  component set is a real block.
* `corpus-r0-open`: the 16 known-truth fixtures of the vendored corpus, all their blocks together. They are
  a few hundred bytes a block, so their rows say what a codec costs per call on tiny blocks and nothing about
  ratio.

**Platform.** The first table is native x64 on MARS (`devshell@192.168.0.100`, rustc 1.99.1, `--release`,
one thread, in a `linux/amd64` container). #110 added the other two platforms, and `tools/codec-measure.sh`
runs all three (`x64`, `wasm`, `arm64`), in containers only:

* **arm64**: native, on mercury (Apple silicon, Colima, so a Linux VM under the hood), in a
  `linux/arm64` `rust:1.99.1-bookworm` container, sources in a throw-away named volume.
* **wasm**: the same test compiled for `wasm32-wasip1` in the MARS gate image, then run under node 22.23 (V8's
  wasm engine, default tiering) in a second container through its WASI, with `tools/wasi-run.mjs`.
  I used `wasip1` and not `wasm32-unknown-unknown` because `std::time::Instant` has no clock on the latter,
  and a timing needs one. The codecs and hashes are pure Rust with no imports, so the machine code the engine
  gets is the same; only the clock and the corpus read (the vendored tar, a file) come from WASI. The target
  features are rustc's default for wasm32, so no `simd128`: that is the baseline a consumer gets unless it
  opts in.

Each timing is the least of five runs, each run long enough to last 150 ms. The load on the host was not
controlled, so read the throughput columns as ratios between rows and not as absolute speeds.

## The table

Codecs are the two ADR 0006 admitted that are pure Rust: an LZ4 block (`lz4_flex`, safe mode) and raw
deflate at level 6 (`miniz_oxide`). zstd is a C library and the downstream dependency policy has no room for
one. The ratio is stored over raw; decompress MB/s is raw bytes out per second; the last column is the XXH3 of
the stored bytes as a share of the time to decompress them, which is what ADR 0006's reversal threshold
(25% in wasm) is about.

| fixture | block kind | codec | blocks | raw bytes | stored bytes | ratio | compress MB/s | decompress MB/s | XXH3 of the stored bytes, % of decode time |
|---|---|---|---|---|---|---|---|---|---|
| plan-like-200k | hot | lz4 | 1173 | 12057624 | 6959379 | 0.577 | 940 | 3107 | 4.9 |
| plan-like-200k | hot | deflate | 1173 | 12057624 | 4119723 | 0.342 | 33 | 527 | 0.5 |
| spline-heavy-20k | hot | lz4 | 40 | 801280 | 164567 | 0.205 | 4599 | 3229 | 1.7 |
| spline-heavy-20k | hot | deflate | 40 | 801280 | 70679 | 0.088 | 92 | 1778 | 0.4 |
| spline-heavy-20k | records | lz4 | 1 | 640032 | 248019 | 0.388 | 1478 | 5196 | 5.1 |
| spline-heavy-20k | records | deflate | 1 | 640032 | 102210 | 0.160 | 46 | 1698 | 0.7 |
| spline-heavy-20k | arena | lz4 | 5 | 1442720 | 455725 | 0.316 | 1111 | 3466 | 2.8 |
| spline-heavy-20k | arena | deflate | 5 | 1442720 | 213977 | 0.148 | 48 | 1321 | 0.5 |
| with-streams-50k | hot | lz4 | 98 | 2003136 | 412254 | 0.206 | 4521 | 3263 | 1.8 |
| with-streams-50k | hot | deflate | 98 | 2003136 | 182366 | 0.091 | 90 | 1720 | 0.4 |
| with-streams-50k | components | lz4 | 1 | 541528 | 233039 | 0.430 | 1353 | 4409 | 4.9 |
| with-streams-50k | components | deflate | 1 | 541528 | 115126 | 0.213 | 32 | 1175 | 0.6 |
| corpus-r0-open (16 fixtures) | hot | lz4 | 8 | 808 | 445 | 0.551 | 514 | 1569 | 3.7 |
| corpus-r0-open (16 fixtures) | hot | deflate | 8 | 808 | 440 | 0.545 | 16 | 57 | 0.1 |
| corpus-r0-open (16 fixtures) | records | lz4 | 8 | 512 | 293 | 0.572 | 366 | 1455 | 4.5 |
| corpus-r0-open (16 fixtures) | records | deflate | 8 | 512 | 284 | 0.555 | 12 | 39 | 0.1 |
| corpus-r0-open (16 fixtures) | arena | lz4 | 40 | 3016 | 1679 | 0.557 | 410 | 1297 | 3.3 |
| corpus-r0-open (16 fixtures) | arena | deflate | 40 | 3016 | 1378 | 0.457 | 14 | 42 | 0.1 |

The hash rates over the largest fixture (12 MB of hot blocks), native x64:

| hash | MB/s |
|---|---|
| XXH3-64 | 39,479 |
| SHA-256 (the safe implementation in `integrity.rs`) | 411 |

## arm64 and wasm (#110)

Same test, same fixtures, three platforms. The x64 column is a rerun for this table (the first table above is the
original run, so the two differ by the noise of a shared host: `with-streams-50k` hot lz4 decoded at 3,263 MB/s
then and 1,994 now). The wasm column is the first of two runs; the second agreed within about 10% on the large
rows (plan-like hot lz4 decode 2,557 MB/s and 8.3%, with-streams components lz4 2,322 MB/s and 8.5%). The stored
bytes are the wasm32 ones: deflate matches the 64-bit writer byte for byte, and lz4 differs on a few rows
(`records` 242,115 against 248,019, `arena` 466,864 against 455,725) because `lz4_flex` uses a different match
finder on 32-bit targets, which is the canonical-for-pointer-width caveat in decision 5 showing up in a number.

| fixture | block kind | codec | stored bytes, wasm32 | decompress MB/s: x64 / arm64 / wasm | compress MB/s: x64 / arm64 / wasm | XXH3 % of decode: x64 / arm64 / wasm |
|---|---|---|---|---|---|---|
| plan-like-200k | hot | lz4 | 6959379 | 3044 / 2956 / 2479 | 926 / 660 / 743 | 4.9 / 5.1 / 8.3 |
| plan-like-200k | hot | deflate | 4119723 | 508 / 399 / 332 | 32 / 25 / 25 | 0.6 / 0.4 / 1.3 |
| spline-heavy-20k | hot | lz4 | 164567 | 3266 / 1779 / 2582 | 4433 / 2941 / 2251 | 1.8 / 1.0 / 3.0 |
| spline-heavy-20k | hot | deflate | 70679 | 1712 / 1204 / 1311 | 90 / 69 / 69 | 0.4 / 0.3 / 0.7 |
| spline-heavy-20k | records | lz4 | 242115 | 5157 / 3569 / 4178 | 1415 / 934 / 1074 | 5.2 / 3.8 / 8.3 |
| spline-heavy-20k | records | deflate | 102210 | 1665 / 1100 / 1100 | 45 / 35 / 35 | 1.1 / 0.5 / 0.9 |
| spline-heavy-20k | arena | lz4 | 466864 | 3396 / 2879 / 2729 | 1090 / 848 / 914 | 2.8 / 2.5 / 4.7 |
| spline-heavy-20k | arena | deflate | 213977 | 1311 / 891 / 778 | 47 / 36 / 36 | 0.5 / 0.4 / 0.6 |
| with-streams-50k | hot | lz4 | 412254 | 1994 / 1795 / 2467 | 2288 / 2881 / 3142 | 1.9 / 1.0 / 2.9 |
| with-streams-50k | hot | deflate | 182366 | 1599 / 1180 / 1236 | 71 / 68 / 68 | 0.4 / 0.3 / 0.7 |
| with-streams-50k | components | lz4 | 233062 | 4324 / 2994 / 3663 | 1284 / 857 / 1015 | 4.9 / 3.6 / 13.7 |
| with-streams-50k | components | deflate | 115126 | 1177 / 802 / 431 | 32 / 24 / 18 | 0.6 / 0.5 / 0.8 |
| corpus-r0-open (16 fixtures) | hot | lz4 | 445 | 1587 / 896 / 864 | 513 / 336 / 274 | 3.7 / 4.3 / 18.9 |
| corpus-r0-open (16 fixtures) | hot | deflate | 440 | 51 / 35 / 31 | 16 / 16 / 7 | 0.1 / 0.2 / 0.6 |
| corpus-r0-open (16 fixtures) | records | lz4 | 293 | 1450 / 659 / 790 | 371 / 281 / 210 | 5.1 / 4.5 / 24.8 |
| corpus-r0-open (16 fixtures) | records | deflate | 284 | 35 / 24 / 31 | 11 / 12 / 5 | 0.2 / 0.2 / 0.5 |
| corpus-r0-open (16 fixtures) | arena | lz4 | 1679 | 918 / 682 / 1365 | 329 / 278 / 546 | 4.0 / 3.6 / 17.6 |
| corpus-r0-open (16 fixtures) | arena | deflate | 1378 | 37 / 30 / 34 | 12 / 12 / 12 | 0.1 / 0.1 / 0.4 |

Hash rates over the 12 MB of hot blocks:

| hash | x64 MB/s | arm64 MB/s | wasm MB/s |
|---|---|---|---|
| XXH3-64 | 34,103 | 34,912 | 14,789 to 17,355 |
| SHA-256 (the safe implementation) | 390 | 199 | 218 to 383 |

What it says:

* **The reversal threshold holds.** ADR 0006 says a block checksum that costs more than 25% of block decode time
  in wasm becomes opt-in per file. On the three generated fixtures (blocks of 10 to 640 KB) the worst
  XXH3 share in wasm is 13.7% (one run) and 12.7% (the other), both on LZ4, so it is under, by about half. The
  ADR's own fixture put it near 21% for LZ4; this one decodes a little slower and hashes a little faster than
  that estimate assumed. Deflate is under 1.5% everywhere.
* **Tiny blocks are the edge.** The corpus fixtures cut blocks of 300 to 800 bytes, where an LZ4 decode is a
  few hundred nanoseconds and the per-call cost dominates. There XXH3 is 17.6 to 24.8% in the first wasm run and
  25.1% in one row of the second, so right on the line. I am not treating that as a reversal: nobody ships a
  file made of 500 byte blocks (ADR 0006 measured a mean of 33,621 bytes), the writer's block size is the
  ADR's, and the three generated fixtures, whose blocks are 10 to 640 KB, stay at 13.7% or under. It does say that a
  producer who cuts very small blocks and uses LZ4 should expect the checksum to be a visible part of decoding
  them in wasm.
* **wasm decode is about 0.8 of native for LZ4 and 0.4 to 0.8 of native for deflate** on the large rows
  (lz4 2.5 to 4.2 GB/s against 3.0 to 5.2, deflate 0.33 to 1.3 GB/s against 0.5 to 1.7). ADR 0006's fixture
  measured 3.8 GB/s for LZ4 in wasm, so this one is lower, but a fetched block is still bound by the network
  long before it is bound by the decoder.
* **arm64 (mercury, inside a Colima VM) is within about half of x64.** LZ4 decodes at 1.8 to 3.6 GB/s, deflate at
  0.4 to 1.2 GB/s, XXH3 at the same 35 GB/s. Compression is slower (lz4 660 against 926 MB/s on the plan
  fixture); a writer on arm64 pays for it, a reader does not.
* **SHA-256 stays as it is.** The safe implementation does 218 to 383 MB/s in wasm and 199 on arm64 (no
  hardware extension is used, which is the cost of having no `unsafe` in the crate). That is 170 to 300 ms for a
  65 MB file and it runs on request, not at open, so the issue's condition for replacing it ("if SHA-256
  verification turns out slow in wasm") is not met. If a consumer disagrees, the replacement goes behind the
  same `sha256` function and this table is where to look first.

The three runs are on shared hosts (MARS was at a load average above 20 for part of this, mercury runs other
containers), so read these as ratios between rows and platforms, not as speeds to quote.

## What the table says

* Both codecs find a lot to compress in plan-like data, and deflate finds about twice as much as LZ4 (0.09 to
  0.34 against 0.21 to 0.58) at about a fifth to a half of the decode speed (0.5 to 1.8 GB/s against 3.1 to 5.2).
  On the hardest fixture, whose coordinates are random, LZ4 still takes the hot blocks to 0.577.
* Decoding is fast enough not to matter next to a network: even deflate decodes a 33 KB block in tens of
  microseconds.
* The block checksum is cheap. XXH3 over the stored bytes costs 0.1 to 5.1% of the time to decode them, so it
  is nowhere near ADR 0006's reversal threshold of 25%, native. The wasm ratio was the open question: #110
  measured it (see "arm64 and wasm" above) and the worst share on the three generated fixtures (blocks of 10 to 640 KB) is 13.7%, under the
  threshold.
* The SHA-256 is 411 MB/s, which is 160 ms for a 65 MB file. That is fine for something that is verified on
  request and not at every open, and it is about six times slower than the 2.4 GB/s ADR 0006 measured with a
  hardware-assisted implementation. I wrote a small safe one so that this crate depends on nothing with
  `unsafe` for it, and a faster one is a follow-up if a consumer finds it slow (#110 measured it in wasm and did not find it slow).

## The decision

1. **Admit LZ4 (codec 1) and raw deflate at level 6 (codec 2), both pure Rust.** The level is fixed and not a
   parameter: a changed level is a changed artifact. Which one a file uses is per block kind and per file,
   recorded in each directory entry, and a block is stored compressed only when that is smaller, so an
   uncompressed block (codec 0) stays legal and a block that does not shrink is not made larger by trying.
2. **The writer's default is no compression.** I am not choosing a default from synthetic fixtures. The
   numbers above say compression pays on plan-like data, and they say nothing about a real drawing, which
   P8.1's corpus and P8.3's benchmark are for. `CodecPolicy::RECOMMENDED` is LZ4 for every kind, which is
   what I would pass for anything served over HTTP: it decodes at 3 to 5 GB/s and takes 40 to 80% off the bytes on these fixtures. A producer that wants the smallest file passes deflate. A mapped file that
   is read in place wants none.
3. **Carry the block checksum.** The measurement above is the number ADR 0006 asked for: 0.1 to 5.1% of
   decode time natively. It is an 8 byte entry per block (about 0.02 to 0.08% of a file at 512 entities a block, depending on how big the entities are) and it
   is verified before a block is decompressed or parsed, which is what makes "a corrupt byte is caught before
   it reaches the decoder" true.
4. **The artifact digest is the SHA-256 of the whole file with its own 32 bytes read as zero.** ADR 0006 said
   "over the header, the directory and the per-block checksums". That covers the directory and the table
   but not the manifest or a single byte of a block except through XXH3, which is not a security hash, so a
   digest of the whole canonical byte stream is the stronger statement and the one issue #7 asks for. It
   costs a read of the whole file, so it is verified on request (`verify_artifact_digest`), in chunks of at
   most 1 MiB with a bounded allocation, and not at every open: a streamed artifact is the case that cannot
   afford it.
5. **A compressed artifact is canonical for the pinned codec crates and the pointer width.** The writer is
   deterministic for a given version of `lz4_flex` and `miniz_oxide`, and `lz4_flex` picks a different match
   finder on 32-bit targets, so a wasm32 writer's LZ4 bytes are not a 64-bit writer's (deflate has no such
   difference), and `Cargo.lock` pins them. A change of codec crate version is a
   change of artifact and goes through a golden-file bless like any other format change.

## What this does not settle

* ~~The wasm decode cost, and any arm64 number.~~ Measured in #110, see "arm64 and wasm" above. A wasm run with
  `simd128` enabled, and a browser engine instead of node, are not measured.
* A default compression policy, which needs real drawings (P8.1, P8.3, P8.6).
* A faster SHA-256 than the safe one, if a consumer needs it (not needed on the numbers above).
