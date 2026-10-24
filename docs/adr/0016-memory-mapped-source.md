# ADR 0016: The memory-mapped source

Status: Accepted, 2026-10-24. Decides #108 (the follow-up to P4.5, `crates/pvec-mmap`). Builds on the `unsafe`
rule in `docs/TOOLCHAIN.md` and the boundary check in `crates/pvec-checks/tests/crate_boundaries.rs`.

## Context

P4.5 asked for an `MmapSource` and for "no `unsafe` outside the mmap implementation". Mapping a file cannot be
written in safe Rust, and the workspace allowed `unsafe` in one file only, the three casts in
`pvec-container/src/blocks/aligned.rs`, with the count pinned by a test. `pvec-container` is an API crate that
may not touch `std::fs` either. So #107 shipped every other source and left the map to a caller, who passes
their own mapped slice to `MemorySource`. That works, but it makes every caller bring a dependency and an
`unsafe` of their own, each with its own argument and none of them checked here.

What was open: where the one block lives, whether it is written against libc or taken from a map crate, and
what the source does about a file that changes while it is mapped.

## Options

| question | options | evidence |
|---|---|---|
| where the block lives | A. a fourth site in `pvec-container`. B. a separate crate, `pvec-mmap`, next to `pvec-sources`. C. nothing, document `MemorySource` over a caller's map | A puts `unsafe` and `std::fs` into an API crate, which the boundary check forbids on both counts. C is the state before this ADR: the `unsafe` is still written, only by someone else, without the count check. B adds one block that the same check can pin |
| how the map is made | A. `libc::mmap` and `munmap` by hand. B. `memmap2` | A is several `unsafe` blocks (the call, the slice, the drop) and a platform split with Windows. B is one block, the call to `Mmap::map`, because the slice and the unmap are safe on its side. The map crate is the common vetted one, it has `libc` as its only dependency on Unix, and the manifest check pins it as the only dependency besides `pvec-container` |
| a file that changes while mapped | A. nothing. B. compare the file's length with the mapped length before each read and refuse a shrunk file. C. install a SIGBUS handler | A faults on the first touch of a missing page. C is more `unsafe`, process-global, and cannot resume a copy safely. B turns the common case (the file was cut before the read) into `SourceError::Short`, and leaves a window between the check and the copy that nothing in safe Rust closes |
| what a read returns | A. a borrow of the map. B. a copy | A would hand untrusted, changeable memory to the decoders as a `&[u8]` they assume is stable. B is what every source does (`MemorySource` copies too), so a changed byte is a wrong value that the reader's checks see, and no decoder holds a reference into the map |

I did not measure a map against `ReadAtSource` over a `PositionalFile` here (P8.3 owns that number). The
decision to have the source does not depend on it: the cost of B in the first row is one block with a
`SAFETY:` argument, and a caller who prefers no map has a safe source of the same checks.

## Decision

B, B, B, B. `crates/pvec-mmap` holds `MmapSource`. Its only `unsafe` is the call to `Mmap::map` in
`src/map.rs`, with its argument in a `SAFETY:` comment, and `lib.rs` denies `unsafe_code`. The workspace now
allows `unsafe` in exactly two files, `aligned.rs` (three blocks) and `pvec-mmap/src/map.rs` (one), and
`crates/pvec-checks/tests/issue_108_unsafe_policy.rs` scans the source of every crate for it, requires the
comment, and pins the counts and the mmap crate's two dependencies. The `unsafe_code = "deny"` lint of the
workspace is not loosened: the one module opts in with `#![allow(unsafe_code)]`, as `aligned.rs` does.

The source checks each range against the length it had when it was mapped before it touches the map, refuses
a file that has shrunk (`SourceError::Short`), maps no empty file, takes only regular files, and returns a copy.
The crate documentation says to map a file the process owns or a read-only copy.

## Consequences

* The residual risk is real and is not hidden: another process that truncates the file between the length
  check and the copy raises SIGBUS and ends the process, and one that writes to it changes bytes under a map
  that the language calls a data race. Neither can be closed from safe Rust. The tests show the case the check
  does close (a file cut after the map was made is an error and holds no memory) and the sources that do not
  have the problem stay available for any file that is not private.
* The hostile corpus and its sweeps run through a map with the verdicts they get from memory (every case is
  written to a file and read back through `MmapSource`), so what a map changes is where the bytes come from
  and not what is checked.
* Browsers and wasm have no map: `pvec-mmap` is not a dependency of any API crate and is not built for
  `wasm32` by the gate.
* A new dependency means a new line in `Cargo.lock` and a new thing to track for advisories.

## Reversal threshold

If the measurement in P8.3 shows a map within 10 percent of `ReadAtSource` on the viewport workload on native
x64 and arm64, the map is not worth its `unsafe` block, and this crate is removed with this ADR superseded. If
`memmap2` stops being maintained, or a release changes what `Mmap::map` requires of its caller, the block is
re-argued before the lock is updated. A second `unsafe` block in this crate, or one in any other crate, needs
a new ADR and a panel decision, not a number changed in the check.
