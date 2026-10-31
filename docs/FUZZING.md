# Fuzzing and hardening the reader

This is the hardening report for issue P7.8 (pvec#59), and how to rerun it. The reader, the decompress boundary, the geometry payloads and the component streams are fuzzed continuously, and a mechanical old/new capability matrix runs as an ordinary test. Every number below was measured in the session that wrote this file; the claims it does not support are in the last section.

## What is fuzzed

The eight targets are plain functions in `crates/pvec-fuzz` (`fn(&[u8]) -> String`, the string being the rule that refused the input or `ok`), so the same code runs under libFuzzer, under the stable-Rust smoke driver and in `cargo test`. The libFuzzer entry points are in `fuzz/fuzz_targets/`, a workspace of its own because `libfuzzer-sys` builds C++ and the root `Cargo.lock` is kept free of native source.

| target | input | what it holds the reader to |
|---|---|---|
| `fuzz_header` | the first 4 KiB of an artifact | a header that is accepted points inside the file; the manifest parses or is refused by name |
| `fuzz_directory_traversal` | a whole artifact | open, plan, read every block, validate and verify the digest; an accepted directory has every block inside the file, every planned request inside the file, every decoded block the length the directory said |
| `fuzz_immediate_blocks` | one hot block | an accepted block has ascending entity ids, finite coordinates and equal-length polyline columns |
| `fuzz_advanced_blocks` | six `u32` lengths, then the records, knots, weights, control points, segments and trims blocks | an accepted spline has a knot vector of the right length that does not decrease, finite numbers and strictly positive weights; a composite names only records that exist; nesting is checked before any segment is resolved |
| `fuzz_decompress_boundary` | `[codec][declared length u32][stored bytes]`, laid out as a one-block artifact and read back | a block that decodes decodes to exactly the declared length and no more than the expansion ratio (64) times the stored length; a bomb is `LIMIT-01` |
| `fuzz_component_streams` | a component stream set | a frame is stepped over by its declared length; only registered streams are read |
| `fuzz_raster_index` | one raster index block (kind 9, `docs/RASTER.md`) | a block that parses encodes back to the same bytes; every level starts where the last ended and holds `cols * rows` tiles in the grid; every tile the lookup is asked for is found; an inline tile has its bytes (its checksum is the reader's to check when the tile is read, not the parser's); a tile in a chunk names a chunk the level has and every chunk has a tile; a tile that is not stored carries zeros; the chunk blocks the tiles describe are exactly as long as their tiles; a count in the header is checked against the block's length before anything is sized by it |
| `fuzz_raster_plan` | a raster grid, a placement (six `f64`) and a viewport, framed in 116 bytes (pvec#220) | the plan never panics; every tile is inside its level, the decoded bytes are under the limit (`LIMIT-16`), a range starts at its first tile and ends at its last, no range is over the request ceiling unless it is one tile, and a not-stored tile is in no range |

Each target asserts these, so a parser that accepts something it should have refused is a crash, as well as one that panics.

`fuzz_raster_index` is the seventh and the first of the raster targets (ADR 0018, issue #218). Its seeds are not artifacts: they are the index blocks of the test-only tiler in `pvec-testkit` (a single tile, a pyramid with inline levels, a sparse one, a split one, a 65,000 entry tile table), eight mutants of each from `pvec_fuzz::mutate::mutate_index` (which keeps the length, so the counts in the header keep adding up and a mutant reaches the parts behind the length check), and header-only blocks that claim more tiles than the ceiling. Its libFuzzer entry point has the same mutator as `fuzz_mutator!`.

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

Every run ended with no crash, no out-of-memory and no timeout. The peak RSS of 413 to 576 MB is the process, libFuzzer and the sanitizer included, and the limit libFuzzer was given was 2,560 MB (the 2,048 MB `max_alloc_bytes` of the P4.8 budget plus 512 MB for the runtime): an execution over it is an `oom` artifact and would be a finding. Every target went past a million executions in its smoke budget on both architectures, the least being 2.48 million for the decompress boundary on x86_64.

The same targets also run on stable Rust without libFuzzer, so the pull request gate does not need a nightly toolchain: `PVEC_FUZZ_EXECUTIONS=3000000 cargo test --release -p pvec-fuzz --test smoke -- --nocapture` ran three seeds of 3,000,000 executions per target (54 million in about 40 seconds on the MacBook), with the allocator counted per execution against the same budget. That driver is not coverage guided and is the weaker evidence; it is the one a test can run every time.

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

`crates/pvec-fuzz/tests/capability_matrix.rs` is an ordinary test. It reads the pairs P7.6 declared in `tests/compat/matrix.txt` (eleven, with the outcome written in the fixture), builds the three readers (`S_old`, `S_new`, this build), and checks that each declared pair keeps its outcome, that every one of the 3 x 13 reader and file crossings is consistent (`negotiate` refuses exactly when `open` does, with the same rule; a degraded open skips only declared capabilities and validates; negotiation is deterministic) and that the geometry read is the same whichever reader opens the file. Another test edits a declared outcome and shows the harness disagrees, which is what turns the matrix red when an outcome changes.

## Running it

```
tools/fuzz.sh --host mercury --seconds 300                      # all eight targets, arm64, the smoke budget
tools/fuzz.sh --host mars --seconds 300 fuzz_header             # one target, native x86_64
tools/fuzz.sh --host jupiter --no-checksum-gate fuzz_directory_traversal
tools/fuzz.sh --host mars --seconds 14400                       # the nightly: 4 hours per target, one after the other
RUN_ID=a CORPUS_VOL=corpus-a FUZZ_CPUS=1 tools/fuzz.sh --host nas --seconds 14400 fuzz_header   # parallel: one invocation per target, image built first
```

Crash reproducers come back to `.fuzz-out/<run>/<host>/<target>/`. Hosted CI is off, so nothing schedules the nightly; the long run above was started by hand.

## What is not claimed

* `fuzz_raster_index` (issue #218) has not run for the nightly window: the long run above is of the first six targets and predates it. The nightly of the two raster targets on MARS is the last step of the raster work (issue #219), and it is recorded here when it has run, with its counts and platform.
* `fuzz_raster_plan` (pvec#220) has not run for the 4 hour budget of the others, and has not run on arm64 at all. It has the smoke budget in `cargo test` and the 300 second libFuzzer run of its acceptance criterion, recorded below once it is done.
* Four of the six targets ran 1.94 hours, not 4 (see the long run above); only `fuzz_directory_traversal` and `fuzz_decompress_boundary` ran the full 4 hours, on x86_64 only. Neither architecture ran every target for 4 hours. All six passed 10^6 executions with no crash, so the open part is the extra two hours of coverage guided search on those four, and the claim P8.7 needs to freeze the security limits rests on that.
* The x86_64 results are 120 seconds, not the stated 300. The arm64 ones are the stated budget.
* The P8.1 benchmark corpus and the EPIC O gold fixtures are not in the seed corpus (see above), and the P2.8 and P4.8 conformance gates they would be judged by are not on main.
* Fuzzing the exporters, converters and a WASM host embedding is out of scope (phase 6, EPICs N, O and M).
