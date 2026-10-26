# Fuzz regressions

One directory per fuzz target (`fuzz_header`, `fuzz_directory_traversal`, `fuzz_immediate_blocks`, `fuzz_advanced_blocks`, `fuzz_decompress_boundary`, `fuzz_component_streams`), one file per crash, named for what it was (`oob-knot-count-<short hash>`).

A crash is fixed by adding or correcting a bounds check or a limit check in the reader, never by widening a limit, and the input that crashed is copied here unchanged. `crates/pvec-fuzz/tests/regressions.rs` replays every file in the directory of its target and fails if any of them panics, so the fix cannot quietly go away.

To add one: take the input libFuzzer wrote (`fuzz/artifacts/<target>/crash-...`), put it in `tests/regressions/<target>/`, and note in the pull request which check now refuses it.
