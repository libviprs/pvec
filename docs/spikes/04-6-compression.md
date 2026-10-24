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

**Platform.** Native x64 on MARS (`devshell@192.168.0.100`, rustc 1.99.1, `--release`, one thread, in a
`linux/amd64` container). **arm64 was not run:** the local container engine refuses to start containers
(read-only file system in its VM), so there is no arm64 result for this task. **WASM-side decode cost was not
measured either:** nothing on the build host runs a wasm module (the P1.8 numbers ran under node, which is not
on it), so the column that issue #7 asks for is missing and follow-up #110 is where it gets filled. What I can
say about wasm is only what ADR 0006 measured on its fixture: LZ4 decodes at 3,796 MB/s and XXH3 hashes at
12,856 MB/s there.

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

## What the table says

* Both codecs find a lot to compress in plan-like data, and deflate finds about twice as much as LZ4 (0.09 to
  0.34 against 0.21 to 0.58) at a fifth to a third of the decode speed (0.5 to 1.8 GB/s against 3.1 to 5.2).
  On the hardest fixture, whose coordinates are random, LZ4 still takes the hot blocks to 0.577.
* Decoding is fast enough not to matter next to a network: even deflate decodes a 33 KB block in tens of
  microseconds.
* The block checksum is cheap. XXH3 over the stored bytes costs 0.1 to 5.1% of the time to decode them, so it
  is nowhere near ADR 0006's reversal threshold of 25%, native. The wasm ratio is the open question, and the
  ADR's own wasm numbers put it near 21% for LZ4 on that fixture, which is also under the threshold.
* The SHA-256 is 411 MB/s, which is 160 ms for a 65 MB file. That is fine for something that is verified on
  request and not at every open, and it is about six times slower than the 2.4 GB/s ADR 0006 measured with a
  hardware-assisted implementation. I wrote a small safe one so that this crate depends on nothing with
  `unsafe` for it, and a faster one is a follow-up if a consumer finds it slow (#110 says so).

## The decision

1. **Admit LZ4 (codec 1) and raw deflate at level 6 (codec 2), both pure Rust.** The level is fixed and not a
   parameter: a changed level is a changed artifact. Which one a file uses is per block kind and per file,
   recorded in each directory entry, and a block is stored compressed only when that is smaller, so an
   uncompressed block (codec 0) stays legal and a block that does not shrink is not made larger by trying.
2. **The writer's default is no compression.** I am not choosing a default from synthetic fixtures. The
   numbers above say compression pays on plan-like data, and they say nothing about a real drawing, which
   P8.1's corpus and P8.3's benchmark are for. `CodecPolicy::RECOMMENDED` is LZ4 for every kind, which is
   what I would pass for anything served over HTTP: it costs 5% of a network round trip in decode time and
   takes 40 to 80% off the bytes. A producer that wants the smallest file passes deflate. A mapped file that
   is read in place wants none.
3. **Carry the block checksum.** The measurement above is the number ADR 0006 asked for: 0.1 to 5.1% of
   decode time natively. It is an 8 byte entry per block (0.024% of a file with 512 entities a block) and it
   is verified before a block is decompressed or parsed, which is what makes "a corrupt byte is caught before
   it reaches the decoder" true.
4. **The artifact digest is the SHA-256 of the whole file with its own 32 bytes read as zero.** ADR 0006 said
   "over the header, the directory and the per-block checksums". That covers the directory and the table
   but not the manifest or a single byte of a block except through XXH3, which is not a security hash, so a
   digest of the whole canonical byte stream is the stronger statement and the one issue #7 asks for. It
   costs a read of the whole file, so it is verified on request (`verify_artifact_digest`), in chunks of at
   most 1 MiB with a bounded allocation, and not at every open: a streamed artifact is the case that cannot
   afford it.
5. **A compressed artifact is canonical for the pinned codec crates.** The writer is deterministic for a given
   version of `lz4_flex` and `miniz_oxide`, and `Cargo.lock` pins them. A change of codec crate version is a
   change of artifact and goes through a golden-file bless like any other format change.

## What this does not settle

* The wasm decode cost, and any arm64 number (see above). #110.
* A default compression policy, which needs real drawings (P8.1, P8.3, P8.6).
* A faster SHA-256 than the safe one, if a consumer needs it.
