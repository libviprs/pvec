# The `pvec` command line tool

`pvec` (crate `pvec-conformance`) runs the checks of phases 2 to 6 from a shell, so a file can be validated, read and compared without writing Rust. Every subcommand takes `--format json` for a machine-readable report and prints a short text otherwise. The text is secondary and can change; the JSON is the contract.

```
pvec <subcommand> [--format json|human] <arguments>
```

It needs nothing outside its own binary: no converter image, no network, no process it starts. STEP files are inputs and are never generated.

## Exit codes

| code | meaning |
|---|---|
| 0 | the file conforms, or the command only reports |
| 1 | non-conformance found: `compare-step` found a disagreement, or `tessellate` could not meet its error budget |
| 2 | the file was refused or is invalid; the report carries a named `error.code` |
| 3 | the file requires a capability this build does not support (`NEGOTIATE-01`, `-02`, `-03`) |
| 64 | the command line was wrong; a message goes to stderr and nothing to stdout |
| 65 | a file could not be read; a message goes to stderr |
| 70 | an internal error; the tool never panics out to a caller |

## The JSON report

Every report is one object with the same envelope, and `report_schema_version` is `1`. The version goes up when a field is removed or changes meaning; a field may be added without it. The schema is checked in as `docs/cli-report.schema.json`, and downstream CI can assert against it.

| field | meaning |
|---|---|
| `report_schema_version` | the version of the report format |
| `command` | the subcommand |
| `input` | `path` as given, and `size_bytes` |
| `exit_code` | the exit code, repeated so a report read later still says |
| `status` | `ok`, `nonconforming`, `unsupported_capability` or `refused` |
| `error` | `null`, or `code` (a named rule such as `DIR-06`, `LIMIT-01`, `INTEGRITY-03`, `NEGOTIATE-01`) and `message` |
| `result` | what the command found; on a refusal only what was known before it stopped |

The conformance verdicts are three separate classes and are never one boolean. `structural_exactness` says whether the parameters of the file and of the STEP export agree, `geometric_exactness` whether the locus agrees at the sampled points, tangents and lengths, and `display_fidelity` whether an approximation stayed inside its error bound. Each is `pass`, its failing word (`differs` for structural, `fail` for the others) or `not_assessed`. A reparameterised export is `geometric_exactness: pass` with `structural_exactness: differs`.

The same input gives the same bytes, with one exception: the fields listed under each command as volatile. Today that is only `timings_ns` of `bench-open`.

## Subcommands

### `inspect`

`pvec inspect <file.pvec>`. Reads the header, the manifest and the directory without opening the file, so a file that needs a capability this build lacks can still be inspected. Exit 0, or 2 with the rule that refused it.

Result fields: `header` (container version, header length, checksum algorithms, where the manifest is), `manifest` (unit, bounds, minimum reader, producer, whether the block checksum table and the artifact digest are written) and `directory` (`blocks`, `entities` and `by_kind`, the blocks, entities and stored bytes of each block kind) and `calibration` (ADR 0019: `status` `absent`, `ok`, `refused` with its `rule`, or `unread` with the rule that refused the file; for `ok` the `scale` with its `provenance` and the `georeference` with its CRS id, kind, method, affine and point count). The field names inside are in the schema. The human output prints the manifest's unit and the calibration's scale as two lines (`unit: 0.001 m per source unit (manifest)`, `calibration scale: 0.1 m per source unit (plan_scale)`), or `calibration: none`; neither replaces the other.

### `validate`

`pvec validate <file.pvec>`. Opens the file under the default security limits, validates every block, verifies the block checksums and the artifact digest. Exit 0 when it is valid, 2 when it is not, 3 when it requires a capability this build lacks.

Result fields: `format_valid`, `blocks`, `entities`, `skipped_blocks` (blocks of an optional capability this build skipped without reading), `rasters` (one entry for each raster the file names: `raster`, `status` `ok` or `skipped`, and for a skipped one the `rule` that refused it, `RASTER-nn`, `LIMIT-nn` or `INTEGRITY-05`, and a `message`; a raster is refused on its own, so a file whose only fault is a bad tile still exits 0 with status `ok` and the geometry gates passed), `calibration` (as `inspect` gives it, plus `residual`: the largest residual recomputed from the control points when the record is fitted from at least 3, the CAL-11 check of `docs/CALIBRATION.md`; a calibration that fails a CAL rule is `refused` with the rule and the file still exits 0), `checks` (`structure`, `limits`, `block_checksums`, `artifact_digest`; the last two are `verified` or `not_written`) and `limits` (the ceilings the file was held to).

### `calibrate`

`pvec calibrate <file.pvec>` with one of `--remove`, `--block <calibration.bin>` (an encoded kind 11 payload, `docs/CALIBRATION.md`, which replaces the whole calibration), `--scale <m>` with `--provenance manual`, `declared_unit` or `georeference` (manual by default), `--plan-scale <n>:<d> --paper-unit <m>`, or `--measured <x0,y0,x1,y1> --distance <m>`, and `--note <text>` for the scale's note. A scale replaces the scale record and keeps the georeference and external id the file carries. The file is rewritten through `recalibrate` (ADR 0019): every other block is copied as it is stored, the checksum table and the digest are recomputed, the input is checked against its own first, and the new bytes go to a temporary file beside it that is renamed over it. Exactly one of `--remove`, `--block` and one scale is given, and `--note` only with a scale. A scale is refused, not dropped, when the file's own calibration fails a CAL rule, since its georeference could not be kept. The temporary file is made new (never truncating another writer's), synced and given the old file's permissions before the rename. Exit 0, 2 with the rule when the calibration or the file is refused (nothing is written then), 64 for flags that conflict, are missing or are not numbers.

Result fields: `bytes` (the new length) and `calibration` (as `inspect` gives it, for the file as rewritten).

### `dump-manifest`

`pvec dump-manifest <file.pvec>`. The manifest in full. Exit 0, or 2.

Result fields: `container_version`, `min_reader`, `unit`, `bounds`, `capabilities` (id, registry name, requirement, version), `directories`, `block_checksums`, `artifact_digest` (hex, or `null`) and `metadata`.

### `capabilities`

`pvec capabilities <file.pvec>`. What the file requires against what this build supports, by the rules of `docs/COMPATIBILITY.md`. Exit 0 when the file can be opened, 3 when a required capability is unsupported, 2 for any other refusal.

Result fields: `reader_version`, `file_min_reader`, `supported`, `declared`, `unsupported_required` (every required capability this build lacks, not only the first), `skipped_optional` and `version_gaps` (capabilities read at a newer minor version than this build implements).

### `compare-step`

`pvec compare-step <file.pvec> <file.step> --evaluator-report <report.txt> [--requests]`. Compares the file with an independent evaluator's reading of a STEP export of it. The evaluator is OpenCASCADE in the container of `tools/step-eval`, which is not run by this tool: its output is an input here, like the STEP file. The entities are taken in ascending id order, and the STEP file is expected to hold exactly those, in that order.

It runs in two passes, as the evaluator does. Pass one: run the evaluator on the STEP file, give its output as the report with `--requests`, and read `result.requests`, the JSON the evaluator takes as its second argument. Pass two: give the report with the samples it answered. Exit 0 when nothing disagrees, 1 when anything does, 2 when the report does not belong to the file (`STEP-REPORT`), the STEP file is not Part 21 (`STEP-FILE`) or an entity cannot be compared (`STEP-UNSUPPORTED`; a composite that trims its segments is one).

Result fields: `step_file` (`path`, `size_bytes`), `evaluator`, `entities`, `verdict` (`structural_exactness`, `geometric_exactness`, `display_fidelity`, each with `status`, `checked` and `disagreements` listing the entity, both sides' parameters, the deviating sample and the tolerance) and `not_comparable` (entities whose edge has no directly comparable parameters, with the reason). With `--requests` the result is `step_file`, `evaluator` and `requests` instead.

A Bezier is stored as a clamped B-spline, so it is compared as one. Tolerances come from the numeric policy of `docs/NUMERIC_POLICY.md`; this tool declares none of its own.

### `tessellate`

`pvec tessellate <file.pvec> --error-px <n> [--units-per-pixel <n>]`. Tessellates every entity so that no chord is further than `n` pixels from the curve, where a pixel is `--units-per-pixel` source units (default 1). This is display fidelity and nothing else: the report asserts nothing about the mathematics of the curves. Exit 0 when every curve met the budget, 1 when one did not.

Result fields: `error_budget_px`, `units_per_pixel`, `tolerance_world`, `display_fidelity` (`status`, `max_deviation_px`), `entities` (`id`, `status`, `points`, `deviation_world`, `deviation_px`) and `skipped` (entities that could not be decoded, with the reason).

### `bench-open`

`pvec bench-open <file.pvec> [--iterations <n>]`. Opens the file `n` times (default 10) and reports the time. Exit 0, or 2 if it does not open.

Result fields: `iterations`, `bytes`, `volatile` and `timings_ns` (`min`, `median`, `max`). `timings_ns` is the volatile field: it differs run to run, and everything else is stable.

## What is not here yet

* Running the evaluator itself. `compare-step` takes its report; starting the container is `pvec-hermetic`'s job in the test suites, and a CLI that started processes would no longer run without a converter image.
* The conformance gates P2.5 and P2.6, the WASM ABI of P7.3 and the gold fixtures of EPIC O are not on main. The hostile corpus of P2.7 is: `crates/pvec-conformance/tests/hostile.rs` runs all of it through `validate`.
