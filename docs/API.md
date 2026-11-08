# The `pvec` facade API (issue P7.1)

The `pvec` crate is what a consumer outside this workspace depends on. It exposes curve semantics and
nothing of the block encoding, so the encoding can be replaced without a major version.

## Frozen intent (stable)

`PvecReader`, `PvecWriter`, `Manifest`, `CapabilityId`, `EntityId`, `Curve`, `CurveRef`, `Domain`,
`Bounds2`, `Transform2` and `PvecError`, plus the component payloads under `pvec::components`
(`LayerStream`, `LineStyleStream`, `TextStream`, `HatchStream`, `ProvenanceStream`,
`SemanticClassStream`, `Fill`, `Stream`, `Skipped`).

Until P8.7 these are stable intent: `public-api.txt` is checked by `tests/public_api.rs`, so any add,
remove, rename or re-sign turns it red and has to be reviewed. After P8.7 a change is a major version.
Each component stream carries its own stream version (ADR 0014), and that is its versioning unit.
`PvecError` is `#[non_exhaustive]` and carries rules as text, so no container type is reachable.

## Picking (issue P7.5)

`PickIndex`, `PickTolerance`, `Pick`, `RectMode`, `EntityIdBatch` and `Point2` answer spatial queries with
file-local `EntityId`s and source-space bounds. Hit testing on exact geometry is analytic, never against a
tessellation, and a Tier 0 preview and its exact replacement answer with the same id and bounds. The
tolerance is in screen pixels and converts through the scale of a `Transform2`. `EntityIdBatch` is one
flat `u32` array. The index is filled by the caller for now, because the facade reader does not decode
curves yet, and it refuses any id the file's entity count does not cover. The anisotropic screen context of
P7.2 and the full packed layout of P7.3 are not here yet.
## Reading curves back

`PvecReader::entities()` returns every entity as `(EntityId, Curve)` in ascending id order. A curve
comes back as the kind that describes it: a Bezier curve is stored as a B-spline and is read as one,
a stored segment is a trimmed line, a circle or ellipse arc is a trimmed conic. Writing what was read
writes the same bytes, which `tests/public_roundtrip.rs` checks on the P2.4 canonical fixtures
(writer, reader, writer). One limit remains: a composite whose segments are trimmed is refused.

`PvecReader::streams()` reads the component streams back: it returns the ones the reader knows, in file order, and the ones it stepped over, from the file's component block (two empty lists when there is none).

## Exact evaluation and tessellation (issue P7.2)

`CurveEval` is exact: `domain`, `point_at`, `derivative_at` (two-sided, refused where only one-sided limits
exist), `derivative_from` (names the `Side`), `bounds`, `is_closed` and `transformed`. Nothing in it
tessellates, and no tessellation leaves anything behind that evaluation reads.

`Tessellate::tessellate(&TessellationContext)` is the one approximation entry point. The context is the
whole chain (`source_to_map`, `map_to_screen`, `pixel_ratio`) and the budget (`max_screen_error_px` in
device pixels, `max_world_error` in map units, `max_segments`), never a bare tolerance. The result is a
polyline in source space tagged `Conformance::DisplayFidelity`, the only value that type has, with the
bound it claims on screen and in map units. Deviation is measured after the chain, so a large source
offset costs nothing. Knots, polyline vertices and composite joints are vertices. A budget the segment
limit cannot reach returns `PvecError::ErrorBudgetUnmet { achieved, requested }`.

The deviation of a segment is found at seven interior points and the claimed bound is that measure times
1.05. It is a measured bound, not a proof, which is why the tag says display fidelity.

## Unstable (`unstable-stream`)

`ViewportQuery`, `ReadPlan`, `Tier0Plan`, `Scene`, `Generation`, `GenerationClock` and `CancelToken`,
in `pvec::unstable`. They can change in a minor version. Their snapshot is `public-api-unstable.txt`,
so churn there never touches the core snapshot.

Since the raster work (ADR 0018, issue 216) the same module holds `Effect`, `ReaderCapabilities` and `ReaderConfig`, the
container types a degraded open and a limited open need, and two `PvecReader` calls: `PvecReader::open_with_config(bytes,
config)`, which opens with the capabilities and limits the config names, and `PvecReader::skipped_effects()`, which lists each skipped capability with what it leaves
undone (`Unstyled`, `Uncomponented`, `Unrastered`, `Uncalibrated` or `Unknown`). `Unrastered` is the "this file has an image
your reader cannot show" promise, and the list is empty for a file that was read whole.

Since issue 218 it also holds the raster calls. `PvecWriter::rasters(placements)` attaches the raster stream (capability 21) and
`PvecWriter::raster_tiles(raster, tiles)` hands over the encoded tiles of one raster, each a `TileIn` (level, column, row and the
PNG, JPEG or WebP bytes, stored as they are; no bytes means a tile that is not stored). `PvecReader::raster_placements()` gives
the placements back, with the six coefficients as the same bits, and `PvecReader::raster_tile(raster, level, col, row)` gives one
tile's bytes after the index's XXH3-64 has been checked against them, `None` for a tile that is not stored. `Placement`,
`RasterId`, `TileCodec`, `PixelFormat`, `Band` and `TileIn` come with them. A raster the file refuses is an error for that
raster; the geometry and the other rasters read.

Since issue 267 it also holds the calibration calls (ADR 0019, `docs/CALIBRATION.md`). `PvecWriter::calibration(c)` sets the
file's scale, georeference or both, written as the last block; a calibration the rules refuse is an error there with its
`CAL-NN` rule. `PvecReader::calibration()` and `PvecReader::georeference()` give them back, `None` when the file has none or the
reader was configured without capability 22, and `PvecReader::effective_scale()` is an `EffectiveScale`: the calibration's
`metres_per_source_unit` with its `Provenance` and uncertainty when there is a scale record, the manifest's `metres_per_unit`
otherwise, never a bare number. `Calibration`, `ScaleClaim`, `Provenance`, `Georeference`, `CrsKind`, `FitMethod`,
`CrsDefinition`, `DefinitionFormat` and `ControlPoint` come with them. Nothing is applied to coordinates.

## Never exported

Anything from `pvec-container` (`BlockKind`, `DirectoryEntry`, `Header`, `ContainerWriter`,
`ContainerReader`), `pvec-codec`, `pvec-sources` and the directory internals of `pvec-stream`. A test
greps both snapshots for those names.

## Features

`std` (default), `alloc`, `mmap`, `http` and `wasm`. The exported type list is the same under each,
and `tests/feature_matrix.rs` is built under every one. `http` is real since #205: it adds
`PvecReader::open_ranges(len, read)`, which reads the file through `read(offset, len)` (an HTTP range
request, a cache) instead of from bytes in memory, and adds no type. The other I/O features are still
markers: the reader works over bytes in memory.

## Consuming it from libviprs

libviprs imports the crate renamed, `pvec_facade = { package = "pvec", ... }`, and lists `pvec_facade`
in its allowed roots, so a bare `pvec::` stays its own module. Only `src/pvec/writer.rs` names
`PvecWriter`.
