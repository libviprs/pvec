# Fuzz regressions

One directory per fuzz target (`fuzz_header`, `fuzz_directory_traversal`, `fuzz_immediate_blocks`, `fuzz_advanced_blocks`, `fuzz_decompress_boundary`, `fuzz_component_streams`, `fuzz_raster_index`, `fuzz_raster_headers`, `fuzz_raster_plan`, `fuzz_raster_archive`, `fuzz_spill_writer`), one file per crash, named for what it was (`oob-knot-count-<short hash>`).

A crash is fixed by adding or correcting a bounds check or a limit check in the reader, never by widening a limit, and the input that crashed is copied here unchanged. `crates/pvec-fuzz/tests/regressions.rs` replays every file in the directory of its target and fails if any of them panics, so the fix cannot quietly go away.

To add one: take the input libFuzzer wrote (`fuzz/artifacts/<target>/crash-...`), put it in `tests/regressions/<target>/`, and note in the pull request which check now refuses it.

`fuzz_raster_plan/slow-quadratic-range-check-*` is the one finding of the plan target so far, and it was in the target and not in the plan: the smoke driver ran 215 seconds on it because the target's own check of the ranges was quadratic in the tiles of a window of a million. The check is a binary search now, and the file replays in milliseconds.
