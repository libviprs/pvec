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

## Reading curves back

`PvecReader::entities()` returns every entity as `(EntityId, Curve)` in ascending id order. A curve
comes back as the kind that describes it: a Bezier curve is stored as a B-spline and is read as one,
a stored segment is a trimmed line, a circle or ellipse arc is a trimmed conic. Writing what was read
writes the same bytes, which `tests/public_roundtrip.rs` checks on the P2.4 canonical fixtures
(writer, reader, writer). Two limits remain: a composite whose segments are trimmed is refused, and
component streams are written but not yet read back.

## Exact evaluation and tessellation (issue P7.2)

`CurveEval` is exact: `domain`, `point_at`, `derivative` (two-sided, refused where only one-sided limits
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

## Never exported

Anything from `pvec-container` (`BlockKind`, `DirectoryEntry`, `Header`, `ContainerWriter`,
`ContainerReader`), `pvec-codec`, `pvec-sources` and the directory internals of `pvec-stream`. A test
greps both snapshots for those names.

## Features

`std` (default), `alloc`, `mmap`, `http` and `wasm`. The exported type list is the same under each,
and `tests/feature_matrix.rs` is built under every one. Today the I/O features are markers: the
reader works over bytes in memory.

## Consuming it from libviprs

libviprs imports the crate renamed, `pvec_facade = { package = "pvec", ... }`, and lists `pvec_facade`
in its allowed roots, so a bare `pvec::` stays its own module. Only `src/pvec/writer.rs` names
`PvecWriter`.
