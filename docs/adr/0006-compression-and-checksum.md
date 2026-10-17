# ADR 0006: Compression and checksum strategy

Status: Accepted, 2026-10-16. **The concrete codec is not final here: it is re-benchmarked in P4.6 and
P8.3.** What is decided is the structure around it. Evidence: `docs/spikes/01-8-benchmark-summary.md`
(section on compression and checksums) and `docs/spikes/01-8-data/compress-checksum.txt`.

## Context

The container has independent blocks (ADR 0003), so compression and integrity work per block. PVec must
bound decompression expansion for untrusted files and detect corruption cheaply in the browser.

## Options

Layout B's blocks of the `million` fixture (1,954 blocks, 65,696,072 bytes, mean 33,621 bytes), native x64
on MARS, CPU time, one thread; wasm32 under node 22 on the same machine.

| codec, independent blocks | stored bytes (ratio) | compress MB/s | decompress MB/s native / wasm |
|---|---|---|---|
| none | 65,696,072 (1.000) | | zero copy |
| LZ4 block | 46,925,070 (0.714) | 689 | 4,381 / 3,796 |
| deflate level 1 | 41,008,296 (0.624) | 182 | 379 / 287 |
| deflate level 6 | 36,716,000 (0.559) | 53 | 481 / not run |
| zstd level 3 (C library) | 37,958,243 (0.578) | 334 | 1,289 / not built |

* Independence between blocks costs 0.4% (deflate 1: 41,008,296 against 40,842,192 for one stream over
  all blocks) to 3.1% (deflate 6: 36,716,000 against 35,607,726).
* An 8-byte shuffle pre-filter makes this corpus larger, not smaller: deflate 1 goes from 41.0 MB to
  48.7 MB (+18.8%). The fixtures round coordinates to three decimals and are plan-like, so the finding
  does not generalise to real drawings; the HDF5 spike saw the same with its shuffle filter.
* HDF5's own shuffle + deflate 6 gave 70,011,564 bytes for the HDF5 structure-of-arrays file (0.191 of
  Part 21), against 100,478,313 uncompressed.

Checksums over the same blocks, MB/s, native / wasm: CRC-32 16,165 / 3,982; CRC-32C 13,067 / 2,881;
XXH3-64 13,649 / 12,856; FNV-1a 1,304 / 1,333; SHA-256 2,467 / 507. The two full passes in the data file
differ by 10 to 20% on single cells (an earlier pass had XXH3 at 10,523 in wasm); the ranking did not change.

## Decision

1. **Per-block compression with a codec id and the uncompressed length in the block's directory entry.**
   Decoders refuse output beyond the declared length (tested: a 64 MB bomb claiming 1 KB is refused for
   deflate and LZ4 without being expanded). The codec may vary per block and per file, so P4.6 can choose
   later; the candidates measured here are LZ4, deflate and zstd.
2. **No byte-shuffle pre-filter in v1.**
3. **A per-block XXH3-64 checksum over the stored bytes**, verified before decompressing, so a corrupt
   block never reaches the decompressor. It is the fastest in wasm (3.2 times CRC-32 and 4.5 times CRC-32C), 8 bytes per block
   (15.6 KB for 1,954 blocks, 0.024% of the file).
4. **An artifact-level SHA-256** over the header, the directory and the per-block checksums in the manifest,
   for integrity against tampering, since XXH3 is not a security hash. Over a whole 65.8 MB file SHA-256
   is 27 ms native and 130 ms wasm, so it is verified at open, not per query.

## Consequences

* The directory entry grows from 56 to 64 bytes (0.125 bytes per entity at 512 entities per block).
* Checksum cost is not free: stored bytes of LZ4 blocks are 0.714 of raw, so verifying at 10.5 GB/s adds
  about a fifth of the LZ4 decode time in wasm (0.056 s against 0.263 s per raw GB); against deflate it is
  about 1.4%. For uncompressed zero-copy blocks the checksum is the dominant cost.
* Compression is applied per block, so a random-access read decompresses one block, not a stream.

## Reversal threshold

Make verification opt-in per file (a manifest flag) if P4.6 finds checksum verification costing more than
25% of block decode time in wasm for the chosen codec; today it is about 21% for LZ4 and about 1.4% for
deflate. Reverse the shuffle decision if P8.1's real-drawing corpus shows it saving more than 10% on
deflate. Replace XXH3 if its specification changes or P7.8's fuzzing finds a collision problem.
