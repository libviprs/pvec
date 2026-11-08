# Fuzzing and hardening the reader

This is the hardening report for issue P7.8 (pvec#59), and how to rerun it. The reader, the decompress boundary, the geometry payloads and the component streams are fuzzed continuously, and a mechanical old/new capability matrix runs as an ordinary test. Every number below was measured in the session that wrote this file; the claims it does not support are in the last section.

## What is fuzzed

The eleven targets are plain functions in `crates/pvec-fuzz` (`fn(&[u8]) -> String`, the string being the rule that refused the input or `ok`), so the same code runs under libFuzzer, under the stable-Rust smoke driver and in `cargo test`. The libFuzzer entry points are in `fuzz/fuzz_targets/`, a workspace of its own because `libfuzzer-sys` builds C++ and the root `Cargo.lock` is kept free of native source.

| target | input | what it holds the reader to |
|---|---|---|
| `fuzz_header` | the first 4 KiB of an artifact | a header that is accepted points inside the file; the manifest parses or is refused by name |
| `fuzz_directory_traversal` | a whole artifact | open, plan, read every block, validate and verify the digest; an accepted directory has every block inside the file, every planned request inside the file, every decoded block the length the directory said |
| `fuzz_immediate_blocks` | one hot block | an accepted block has ascending entity ids, finite coordinates and equal-length polyline columns |
| `fuzz_advanced_blocks` | six `u32` lengths, then the records, knots, weights, control points, segments and trims blocks | an accepted spline has a knot vector of the right length that does not decrease, finite numbers and strictly positive weights; a composite names only records that exist; nesting is checked before any segment is resolved |
| `fuzz_decompress_boundary` | `[codec][declared length u32][stored bytes]`, laid out as a one-block artifact and read back | a block that decodes decodes to exactly the declared length and no more than the expansion ratio (64) times the stored length; a bomb is `LIMIT-01` |
| `fuzz_component_streams` | a component stream set | a frame is stepped over by its declared length; only registered streams are read |
| `fuzz_raster_headers` | `[codec][edge][format][bytes]`: the codec id, the edge in units of 128, the format (channels less one, and a bit for one bit deep), then a tile | an accepted tile is what the contract says: a PNG with the signature, an `IHDR` of the placement's edge, bit depth and a colour type that is not palette, no interlace and `IEND` as its last chunk and its last byte; a JPEG with `SOI`, only under gray or RGB at 8 bits, ending in `EOI`; a WebP whose RIFF size is its length less 8; none longer than `LIMIT-15`; and no sniffer examines more than 64 KiB of any tile |
| `fuzz_raster_index` | one raster index block (kind 9, `docs/RASTER.md`) | a block that parses encodes back to the same bytes; every level starts where the last ended and holds `cols * rows` tiles in the grid; every tile the lookup is asked for is found; an inline tile has its bytes (its checksum is the reader's to check when the tile is read, not the parser's); a tile in a chunk names a chunk the level has and every chunk has a tile; a tile that is not stored carries zeros; the chunk blocks the tiles describe are exactly as long as their tiles; a count in the header is checked against the block's length before anything is sized by it |
| `fuzz_raster_plan` | a raster grid, a placement (six `f64`) and a viewport, framed in 116 bytes (pvec#220) | the plan never panics; every tile is inside its level, the decoded bytes are under the limit (`LIMIT-16`), a range starts at its first tile and ends at its last, no range is over the request ceiling unless it is one tile, and a not-stored tile is in no range |
| `fuzz_raster_archive` | a PMTiles v3 archive, with or without the `vnd.pvec` placement (pvec#222) | an archive that reads is written back to the same tiles, at distinct addresses inside their zoom; an import that succeeds gives a placement the stream encoder accepts and tiles inside its pyramid; the limits are checked before anything is sized by a count in the header or a directory |
| `fuzz_spill_writer` | `[budget][store fails at call][sink room]`, then pushes of seven bytes each (pvec#235) | the writer that keeps its pushes in files writes the in-memory writer's artifact byte for byte for the same pushes, whatever the budget; a rule refused at a push or at the finish is the same rule; a store that fails at any call or a sink that fills is a typed error, and a build that was refused writes nothing to the sink, and one whose files cannot be read back while the bytes flow leaves a prefix of the artifact in it |

Each target asserts these, so a parser that accepts something it should have refused is a crash, as well as one that panics.

`fuzz_raster_index` is the seventh and the first of the raster targets (ADR 0018, issue #218), and `fuzz_raster_headers` the eighth (issue #219; seeds: one tile of each codec and variant libviprs writes at 256, 512 and 1024, the thirteen hostile tiles of `pvec_testkit::raster_hostile` and four header mutants of each). Its seeds are not artifacts: they are the index blocks of the test-only tiler in `pvec-testkit` (a single tile, a pyramid with inline levels, a sparse one, a split one, a 65,000 entry tile table), eight mutants of each from `pvec_fuzz::mutate::mutate_index` (which keeps the length, so the counts in the header keep adding up and a mutant reaches the parts behind the length check), and header-only blocks that claim more tiles than the ceiling. Its libFuzzer entry point has the same mutator as `fuzz_mutator!`. `fuzz_raster_plan` is the ninth (issue #220): its seeds are not artifacts either but framed placements and viewports (an upright image, a rotated and sheared one, a tiny scale, overzoom, a viewport off the image, a huge viewport, a singular map), each under two request limits.

`fuzz_raster_archive` is the tenth (issue #222, the raster half of the PMTiles interchange): its seeds are sparse three level pyramids of each tile type with and without the placement, each cut short at six lengths and with a byte of the header and of the first directory flipped. It ran 300 seconds under libFuzzer on mars (native x86_64, the row above): 2.55 million executions, no crash, no out-of-memory, no timeout. It has not run for the 4 hour budget and has not run on arm64.

`fuzz_spill_writer` is the eleventh (issue #235) and the first that is not a parser: it drives `SpillWriter` over a store in memory that fails at the call the input names, against `ContainerWriter` for the same pushes. Its seeds are sixty scripts and the empty one: budgets from 0 to 1 MiB, a store that never fails or fails at a call between 0 and 79, a sink with room for none of the artifact up to all of it, and pushes of every kind, with a repeated id and with a spline given a cell so that the refusals are reached. The deterministic runs of the unit tests in `crates/pvec/tests/issue_235_streaming_writer.rs` and `crates/pvec-container/tests/issue_235_spill_writer.rs` cover the same ground on the real directory of files. It has run under the stable-Rust smoke driver (below), and it has not run under libFuzzer, for the 4 hour budget, or on arm64.

## Seed corpus

`cargo run --release -p pvec-fuzz --bin seed-corpus -- <dir>` writes it, per target, as libFuzzer wants it:

| source | artifacts |
|---|---|
| hostile corpus of P2.7 (the named cases; the 37,000 truncation sweeps are left to the hostile test) | 149 |
| `KNOWN_TRUTH_EXACT` fixtures of the vendored bundle, written sealed by the container writer | 22 |
| container writer output of the synthetic shapes (drawing, splines, composites, streams, gapped, lz4, deflate) and four mutated manifests of each, sealed again | 35 |

From these, per target: 206 inputs for `fuzz_header` and `fuzz_directory_traversal`, 40 hot blocks, 57 framed advanced sets, 409 framed blocks for the decompress boundary (the stored block, and the same block packed by each codec) and 8 component stream sets.

Not included, and why: the P8.1 benchmark corpus after minimisation (P8.1 is not on main, so there is nothing to minimise) and the `KNOWN_TRUTH_EXACT` fixtures of `libviprs/pvec-gold-fixtures#1` (EPIC O has produced none; the vendored records stand in for them). `pvec_fuzz::seeds::DEFERRED` says the same and a test pins it.

The corpus persists between runs in the docker volume `pvec-fuzz-corpus` on the host (`tools/fuzz.sh --keep-corpus`), and `cp -n` adds the seeds without overwriting what a run grew.

### Raster seeds (issues 216 and 217, ADR 0018)

`fuzz_directory_traversal` gets, for each of the writer bases, three files that add a kind 9 and a kind 10 entry naming
capability 21, naming none, and naming an unregistered capability (77), so the traversal meets the capability-owned
kinds in every state the negotiation rules decide: handed over, skipped, and refused (`NEGOTIATE-06`). `fuzz_component_streams`
now implements the raster class as well as the components, decodes the raster stream of every set it opens, asserts that
what decodes writes back to something that decodes to the same placements, and is seeded with one stream set holding a
valid placement (the test vector of `docs/RASTER.md`). A smoke run is 300 seconds on MARS (native x86_64) per target:
`fuzz_directory_traversal` ran 6,849,016 executions at about 22,700 a second and found nothing (issue 216), and
`fuzz_component_streams` ran 10,444,284 executions at about 34,700 a second and found nothing (issue 217). The long run of issue 190 has not run with these seeds; only the smoke
runs have.

## The mutator and the checksum gate

A byte flip in a sealed artifact is stopped by a block checksum or the artifact digest, which is a check the fuzzer then spends its executions on. Two things deal with that:

* `pvec_fuzz::mutate` is a structure-aware mutator (it is also libFuzzer's `fuzz_mutator!`): it changes a byte of the header, the manifest, the directory or one block, sometimes to an edge value (0, 1, a sign bit, the largest) and sometimes cuts the tail, then seals the file again over the layout it has, so the checksum table and the digest are right. Measured on the known-truth seeds, 484 of 880 plain byte flips (55%) are stopped at an integrity check and fewer than one in five mutants are (a test pins both).
* The `no-checksum-gate` feature (`cargo +nightly fuzz run --features no-checksum-gate <target>`, which is the fuzz-only `--no-checksum-gate` of the issue) makes every target reseal its input first. The reader is never changed, so a limit or a bounds check cannot be weakened by it; only the input is.

On `fuzz_directory_traversal`, arm64, the mutator alone reached 4,271 edges in 300 s and the flag 4,247 in 120 s, so the structure-aware mutator already does most of what the flag does for this target. The flag matters for inputs the mutator does not produce, such as libFuzzer's own splices of two artifacts.

## Results

`tools/fuzz.sh` runs libFuzzer in throw-away containers (`--platform` always passed; the image, the sources and the corpus are in named volumes that the script removes). The smoke budget of a pull request is 300 seconds per target. I ran 300 s per target on arm64, and 120 s per target on x86_64 because mars is shared with two other agents' gates. Platforms: mercury is an Apple silicon Mac running Colima (native arm64), jupiter is the MacBook (Docker Desktop, native arm64), mars is native x86_64. No emulated result is reported.

| target | host | platform | checksum gate in play | time | executions | exec/s | edges | features | corpus | peak RSS |
|---|---|---|---|---|---|---|---|---|---|---|
| `fuzz_header` | jupiter | native arm64 | yes | 301 s | 31,775,131 | 105,565 | 494 | 746 | 330 -> 368 | 532 MB |
| `fuzz_header` | mars | native x86_64 | yes | 121 s | 13,378,888 | 110,569 | 459 | 689 | 206 -> 368 | 576 MB |
| `fuzz_directory_traversal` | jupiter | native arm64 | yes | 301 s | 12,066,780 | 40,088 | 4,271 | 9,928 | 206 -> 1,067 | 540 MB |
| `fuzz_directory_traversal` | mars | native x86_64 | yes | 121 s | 3,198,725 | 26,435 | 3,907 | 8,882 | 206 -> 910 | 463 MB |
| `fuzz_directory_traversal` (flag) | jupiter | native arm64 | no | 121 s | 4,975,399 | 41,119 | 4,247 | 10,023 | 206 -> 1,071 | 472 MB |
| `fuzz_immediate_blocks` | jupiter | native arm64 | yes | 301 s | 41,388,812 | 137,504 | 236 | 325 | 40 -> 78 | 488 MB |
| `fuzz_immediate_blocks` | mars | native x86_64 | yes | 121 s | 24,837,711 | 205,270 | 229 | 321 | 40 -> 77 | 486 MB |
| `fuzz_advanced_blocks` | mercury | native arm64 | yes | 301 s | 15,637,984 | 51,953 | 852 | 1,971 | 57 -> 263 | 535 MB |
| `fuzz_advanced_blocks` | mars | native x86_64 | yes | 121 s | 9,652,299 | 79,771 | 791 | 1,816 | 57 -> 248 | 536 MB |
| `fuzz_decompress_boundary` | mercury | native arm64 | yes | 301 s | 5,744,672 | 19,085 | 2,162 | 4,018 | 409 -> 801 | 438 MB |
| `fuzz_decompress_boundary` | mars | native x86_64 | yes | 121 s | 2,477,290 | 20,473 | 2,191 | 4,003 | 409 -> 769 | 436 MB |
| `fuzz_decompress_boundary` (flag) | jupiter | native arm64 | no | 121 s | 4,599,176 | 38,009 | 2,160 | 4,051 | 409 -> 794 | 442 MB |
| `fuzz_component_streams` | mercury | native arm64 | yes | 301 s | 11,015,253 | 36,595 | 420 | 603 | 8 -> 74 | 413 MB |
| `fuzz_component_streams` | mars | native x86_64 | yes | 121 s | 5,397,979 | 44,611 | 449 | 616 | 8 -> 58 | 417 MB |
| `fuzz_raster_plan` | mars | native x86_64 | yes | 301 s | 12,390 | 41 | 379 | 1,291 | 16 -> 141 | 374 MB |
| `fuzz_raster_archive` | mars | native x86_64 | yes | 301 s | 2,548,480 | 8,466 | not logged | not logged | 1,091 -> 2,035 | 817 MB |

Every run ended with no crash, no out-of-memory and no timeout. The peak RSS of 413 to 576 MB is the process, libFuzzer and the sanitizer included, and the limit libFuzzer was given was 2,560 MB (the 2,048 MB `max_alloc_bytes` of the P4.8 budget plus 512 MB for the runtime): an execution over it is an `oom` artifact and would be a finding. Every target went past a million executions in its smoke budget on both architectures, the least being 2.48 million for the decompress boundary on x86_64.

The same targets also run on stable Rust without libFuzzer, so the pull request gate does not need a nightly toolchain: `PVEC_FUZZ_EXECUTIONS=3000000 cargo test --release -p pvec-fuzz --test smoke -- --nocapture` ran three seeds of 3,000,000 executions per target (54 million in about 40 seconds on the MacBook), with the allocator counted per execution against the same budget. That driver is not coverage guided and is the weaker evidence; it is the one a test can run every time.

## The raster targets (issues #218 and #219)

The smoke budget of a pull request for the raster work is 300 seconds a target, run on MARS (native x86_64, one container, two cpus) with `tools/fuzz.sh --host mars --seconds 300 fuzz_raster_index fuzz_raster_headers fuzz_directory_traversal`. `fuzz_directory_traversal` has the kind 9 and kind 10 seed entries of issue #216 in its corpus, which is where the traversal changed. No run produced a crash, an out-of-memory or a timeout.

| target | host | platform | time | executions | exec/s | edges | features | corpus | peak RSS |
|---|---|---|---|---|---|---|---|---|---|
| `fuzz_raster_index` | mars | native x86_64 | 301 s | 38,193,245 | 126,887 | 479 | 1,159 | 69 -> 102 | 457 MB |
| `fuzz_directory_traversal` | mars | native x86_64 | 301 s | 7,927,434 | 26,336 | 4,251 | 9,976 | 257 -> 1,067 | 466 MB |
| `fuzz_raster_headers` | mars | native x86_64 | 301 s | 80,509,768 | 267,474 | not kept | not kept | 147 -> 520 | 624 MB |

The edge and feature counts of the headers run were not kept (the log keeps its last forty lines). The index target plateaus early (479 edges after 1.3 million executions): its parser is a short chain of checks, and the structure-aware mutator keeps the length so the counts keep adding up. The corpus of the run lives in a docker volume that `tools/fuzz.sh` removes at the end unless `--keep-corpus` is given; the seeds are rebuilt from the tiler by `seed-corpus`, so nothing is lost by that.

The four hour nightly of the two raster targets (issue #219), `tools/fuzz.sh --host mars --seconds 14400 --keep-corpus`, one container each, started 2026-10-31 about 04:00 PDT: no crash, out-of-memory or timeout, and no file in any artifact directory.

| target | host | platform | time | executions | exec/s | corpus | peak RSS |
|---|---|---|---|---|---|---|---|
| `fuzz_raster_index` | mars | native x86_64 | 4.0 h (14,400 s) | 2,245,200,601 | 155,905 | 69 -> 105 | 652 MB |
| `fuzz_raster_headers` | mars | native x86_64 | 4.0 h (14,400 s) | 4,772,243,633 | 331,382 | 147 -> 615 | 713 MB |

Both are far past 10^6 executions. The corpora are in the docker volumes `pvec-fuzz-corpus-r219n-index` and `pvec-fuzz-corpus-r219n-headers` on MARS.

## The long run (P7.8, issue #190)

`--seconds` is per target and the targets of one invocation run one after the other, so six targets at 14,400 s on one host is 24 hours. To fit the window I ran separate `tools/fuzz.sh` invocations in parallel, one container each (`--cpus 1`, libFuzzer is single threaded), each with its own `RUN_ID` and its own corpus volume (`CORPUS_VOL`), after building the image once on every host so they did not race on the image tag. All of it started at 00:07 PDT on 2026-10-28 and ended by 04:12.

| target | host | platform | time | executions | exec/s | corpus | peak RSS |
|---|---|---|---|---|---|---|---|
| `fuzz_directory_traversal` | nas | native x86_64 | 4.0 h (14,400 s) | 124,787,740 | 8,665 | 206 -> 1,198 | 650 MB |
| `fuzz_decompress_boundary` | nas | native x86_64 | 4.0 h (14,400 s) | 103,382,149 | 7,178 | 409 -> 866 | 426 MB |
| `fuzz_header` | mars | native x86_64 | 1.94 h (7,000 s) | 805,371,179 | 115,036 | 206 -> 685 | 533 MB |
| `fuzz_immediate_blocks` | mars | native x86_64 | 1.94 h (7,000 s) | 1,398,219,331 | 199,717 | 40 -> 79 | 495 MB |
| `fuzz_advanced_blocks` | mercury | native arm64 | 1.94 h (7,000 s) | 415,395,283 | 59,333 | 57 -> 286 | 536 MB |
| `fuzz_component_streams` | mercury | native arm64 | 1.94 h (7,000 s) | 323,627,667 | 46,225 | 8 -> 74 | 409 MB |

Every run ended with no crash, no out-of-memory and no timeout (no file in any artifact directory), and every target is far past 10^6 executions.

I did not give every target 4 hours. Only the two targets on nas ran the full 4 hours. Mars and mercury are shared with benchmark runs, so I kept each to one container, which meant two targets one after the other at 7,000 s (1.94 h) each. Jupiter was not used: Docker Desktop was not running on the MacBook and I left it alone. The two nas targets are also the slowest per second because the container had one cpu, so their count is lower, still more than 100 million.

## Findings

None. Neither the 187 million libFuzzer executions of the smoke runs, the 3.17 billion of the long run, nor the 54 million executions of the smoke driver found a crash, an assertion failure, an out-of-memory or a hang, so there is no reproducer to check in and `tests/regressions/` holds only its README. The replay test is in place (`crates/pvec-fuzz/tests/regressions.rs`): the first crash lands there as a file named for its target, with the bounds or limit check that fixed it named in the pull request.

## Capability matrix

`crates/pvec-fuzz/tests/capability_matrix.rs` is an ordinary test. It reads the pairs P7.6 declared in `tests/compat/matrix.txt` (fifteen, with the outcome written in the fixture), builds the four readers (`S_old`, `S_new`, `S_raster`, this build), and checks that each declared pair keeps its outcome, that every one of the 4 x 17 reader and file crossings is consistent (`negotiate` refuses exactly when `open` does, with the same rule; a degraded open skips only declared capabilities and validates; negotiation is deterministic) and that the geometry read is the same whichever reader opens the file. Another test edits a declared outcome and shows the harness disagrees, which is what turns the matrix red when an outcome changes.

## Running it

```
tools/fuzz.sh --host mercury --seconds 300                      # all ten targets, arm64, the smoke budget
tools/fuzz.sh --host mars --seconds 300 fuzz_header             # one target, native x86_64
tools/fuzz.sh --host jupiter --no-checksum-gate fuzz_directory_traversal
tools/fuzz.sh --host mars --seconds 14400                       # the nightly: 4 hours per target, one after the other
RUN_ID=a CORPUS_VOL=corpus-a FUZZ_CPUS=1 tools/fuzz.sh --host nas --seconds 14400 fuzz_header   # parallel: one invocation per target, image built first
```

Crash reproducers come back to `.fuzz-out/<run>/<host>/<target>/`. Hosted CI is off, so nothing schedules the nightly; the long run above was started by hand.

## What is not claimed

* The nightly of the two raster targets ran 4.0 h each on MARS (see "The raster targets"); the other six targets were not rerun. It was one container each, one cpu, so the counts are one core's.
* `fuzz_raster_plan` (pvec#220) has run 300 seconds on mars (native x86_64) and no other time: 12,390 executions with no crash, the row above. It has not run for the 4 hour budget of the others and has not run on arm64 at all. The throughput is 41 executions a second against thousands for the others, because one input can make the plan look at a window of a million cells at several levels, which is the bound that the plan keeps (`MAX_WINDOW_CELLS`) and not a stall; mars was also loaded by other jobs during the run (its load average was above 40).
* Four of the six targets ran 1.94 hours, not 4 (see the long run above); only `fuzz_directory_traversal` and `fuzz_decompress_boundary` ran the full 4 hours, on x86_64 only. Neither architecture ran every target for 4 hours. All six passed 10^6 executions with no crash, so the open part is the extra two hours of coverage guided search on those four, and the claim P8.7 needs to freeze the security limits rests on that.
* The x86_64 results are 120 seconds, not the stated 300. The arm64 ones are the stated budget.
* The P8.1 benchmark corpus and the EPIC O gold fixtures are not in the seed corpus (see above), and the P2.8 and P4.8 conformance gates they would be judged by are not on main.
* Fuzzing the exporters, converters and a WASM host embedding is out of scope (phase 6, EPICs N, O and M).
