# Migrating within and beyond v1

This is what a v1.x reader or writer can rely on. It restates the rules of [`COMPATIBILITY.md`](COMPATIBILITY.md) and the
container layout of [`CONTAINER.md`](CONTAINER.md) for someone who has to decide whether an upgrade is safe, and adds no rule of its own.

## What a v1.x reader must keep working

* Every file whose header says container 1.0 and whose manifest asks for no capability the reader lacks. The header layout,
  the byte-order marker, the block directory entry and the integrity tables read as they do today.
* Every capability in the v1 registry snapshot (`docs/capabilities.toml`) at the version and minimum reader it states.
* Skipping an unknown optional block without parsing its payload, and refusing an unknown must-understand block by name.
* The numeric policy and the security limits as defaults: a reader may be configured stricter, never silently looser.
* The packed WASM layouts at `wasm_abi_version` 1.

## What may change within v1.x

* New optional blocks and new capabilities added through the extension rules, which an older reader skips or refuses by name. 1.1.0 is the first such step: capability 21 `raster` (ADR 0018) with block kinds 9 and 10, optional, so a 1.0.0 reader opens the file degraded and the container stays 1.0. 1.2.0 is the second: capability 22 `calibration` (ADR 0019) with block kind 11, a scale and a georeference for the whole artifact, optional, so a 1.0.0 or 1.1.0 reader opens the file degraded with `Effect::Uncalibrated`. A file without a calibration is byte for byte what 1.1.0 wrote. 1.3.0 is the third: the paint model (ADR 0021) starts with component streams 23 `paint_order`, 24 `stroke_paint`, 25 `fill_paint` and 26 `clip`, optional, so any older reader opens the file degraded and draws it as it always did.
* A new minor container version that only adds optional fields after the existing header.
* Writer defaults that do not change what a reader must accept (codec choice, block packing, index tuning).
* Tightened limits in the browser preset, and faster or smaller readers behind the same behaviour.
* The WASM module opening with `Limits::BROWSER` where it used `Limits::DEFAULT` (issue 217, no `wasm_abi_version`
  change). A file between the two presets is now refused by the module, and nothing else moves. `docs/WASM_ABI.md`
  (the Limits section) lists every value before and after: for example `max_block_bytes` 256 MiB to 64 MiB,
  `max_expansion_ratio` 64 to 32, `max_alloc_bytes` 2 GiB to 256 MiB and `max_nesting_depth` 4 to 3.

## WASM exports that need a version 2 module

Exports added at `wasm_abi_version` 2 without a bump (additive, probed by the consumer, which tests that the module
exports the first one of a group): `directory_stat`, `directory_bound`, `directory_query`, `cells_*`, `tier0_viewport`,
`tier0_stat`, `tier0_buffer_*` (issue 201); `open_ranged`, `provide_range`, `open_poll`, `needed_*` (issue 205); and
the raster group of issue 221 (`docs/WASM_ABI.md`, "Rasters"), probed by `raster_stat`: `raster_stat`, `raster_skipped`,
`raster_placement`, `raster_plan`, `raster_tile`, `raster_buffer_len`, `raster_buffer_ptr`, `raster_buffer_elem`. A
host built against the version 2 exports before the raster group exists still works, and one that wants rasters
checks for `raster_stat` once at start. The calibration group of issue 269 (`docs/WASM_ABI.md`, "Calibration", ADR 0019)
is the same kind of addition, probed by `calibration_stat`: `calibration_stat`, `calibration_number`, `calibration_point`,
`calibration_text`.

## What requires v2

* Any change to the meaning, size or position of an existing header or directory field.
* Removing a capability, or raising its minimum reader beyond what the compatibility rules allow within a major version.
* Loosening a P1.2 tolerance or a P4.8 limit as a default, or changing the semantic profile.
* A new `wasm_abi_version`, or any change to the packed layouts of version 1.
