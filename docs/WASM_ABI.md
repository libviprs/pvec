# The WASM packed output ABI (issues P7.3, #201 and #205)

`pvec-wasm` exports the `pvec` facade to a browser as four typed arrays per batch and integer handles. No
export returns an object, an array of objects or a string, so a frame does not pay the allocator once
per entity. `libviprs/maplibre-pvec` (EPIC M) binds these arrays directly; the rules below are a
contract it builds against, so a change to them goes through the compatibility rules of P7.6 and
bumps `wasm_abi_version`.

## Version

`wasm_abi_version()` returns **2**. A consumer reads it once at start and refuses a module whose number it
does not know. The number changes for any change to a layout, an element type, an error code, an export
name or a signature below. Adding an export does not change it; removing or re-signing one does.

Version 2 (issue #201) adds the spatial directory and the Tier 0 stream, and I bumped the number on
purpose even though nothing was removed or re-signed: a consumer that needs those exports has to be able to
refuse a version 1 module, which does not have them. Every export of version 1 keeps its name, signature,
array layout and error code, so a consumer that only uses version 1 calls works unchanged against a version 2
module as soon as its version check accepts 2. The one addition that touches an old call is the error code -6,
which only the new calls return. `open` also reads the directory and the Tier 0 leaf table of the file, and the
module then keeps the file's bytes for as long as the file handle lives (a file with no directory is not
kept), because a leaf's Tier 0 block is read when a call asks for it.

## Calls

Every export takes numbers, or a slice of bytes or numbers, and returns a number: a handle (zero or
more), a count, a byte offset into linear memory, or a negative error code.

| export | takes | returns |
|---|---|---|
| `wasm_abi_version` | | the version |
| `open` | the file's bytes | a file handle |
| `query_viewport` | file, `min_x, min_y, max_x, max_y`, `budget` | a batch handle |
| `batch_len`, `batch_truncated` | batch | the count; 1 when the budget cut the batch |
| `tessellate_batch` | batch, `chain` (15 `f64`), `max_segments`, `precision` | a result handle |
| `buffer_len`, `buffer_ptr`, `buffer_elem` | result, array number | length, byte offset, element type |
| `tessellation_failed`, `tessellation_conformance` | result | count of entities with no vertices; 1 |
| `tessellation_origin_x`, `tessellation_origin_y` | result | origin of the `Float32Array` form |
| `memory_epoch`, `view_current`, `force_grow` | | see Memory |
| `release` | any handle | 0 |
| `directory_stat` | file, number | one number about the spatial directory (version 2) |
| `directory_bound` | file, number | one corner of a box of the directory, NaN if none (version 2) |
| `directory_query` | file, `min_x, min_y, max_x, max_y` | a cells handle (version 2) |
| `cells_len`, `cells_ptr`, `cells_elem` | cells handle, array number | length, byte offset, element type (version 2) |
| `tier0_viewport` | file, `min_x, min_y, max_x, max_y` | a Tier 0 handle (version 2) |
| `tier0_stat` | Tier 0 handle, number | one count (version 2) |
| `tier0_buffer_len`, `tier0_buffer_ptr`, `tier0_buffer_elem` | Tier 0 handle, array number | length, byte offset, element type (version 2) |
| `open_ranged` | the file's length in bytes | an opening handle (issue #205) |
| `provide_range` | handle, `offset`, bytes | 0 (issue #205) |
| `open_poll` | the opening handle | the same handle, now a file, or -7 (issue #205) |
| `needed_count`, `needed_offset`, `needed_len` | handle, range number | how many ranges a -7 left to provide, and where and how long each is (issue #205) |

`open` takes the file whole. A file can instead be opened by byte ranges the host supplies (issue #205), which
is the next section.

`query_viewport` keeps the entities whose bounds meet the box, in ascending `EntityId` order, and at
most `budget` of them; `batch_truncated` says when it cut. A curve with no bounds (an unbounded line)
cannot be culled and is kept. A batch holds the curves it needs, so releasing the file does not affect it.

`chain` is `source_to_map` then `map_to_screen`, each `[a, b, c, d, tx, ty]` with rows `[a b tx]` and
`[c d ty]`, then `pixel_ratio`, `max_screen_error_px` and `max_world_error`. They are the fields of
`pvec::TessellationContext`, and the error is measured after the whole chain.

## The four arrays

One buffer per array per result, whatever the number of entities. `buffer_elem` gives the element type
(0 `Float64Array`, 1 `Float32Array`, 2 `Uint32Array`); the arrays are numbered 0 to 3.

| # | array | element | stride | relation to the offset table |
|---|---|---|---|---|
| 0 | vertices | `Float64Array`, or `Float32Array` when `precision` is 1 | 2 (`x`, `y`) | entity `i` owns vertices `offsets[i] .. offsets[i + 1]`, so elements `2 * offsets[i] .. 2 * offsets[i + 1]` |
| 1 | offset table | `Uint32Array` | 1 | `entities + 1` entries, starting at 0 and not decreasing |
| 2 | entity ids | `Uint32Array` | 1 | entry `i` is the `EntityId` of entity `i` |
| 3 | style refs | `Uint32Array` | 1 | entry `i` is the style reference of entity `i`: an opaque index into the style component stream |

An entity whose error budget cannot be met within `max_segments` has an empty range (`offsets[i] ==
offsets[i + 1]`), and `tessellation_failed` counts them. The rest of the batch is unaffected. Text and
style are never strings on this path: they are indices into the component streams.

The `Float32Array` form holds the vertices minus an origin, which `tessellation_origin_x` and
`tessellation_origin_y` return as `f64`, so a GPU with 24 bits of mantissa keeps the precision a
map at a coordinate offset of a million needs. The consumer adds the origin back in its own transform.

A viewport query that returns N entities costs JS four typed arrays, made by the consumer over linear
memory (`new Float64Array(memory.buffer, ptr, len)`), whatever N is. A test counts them at N = 1, 1,000
and 100,000.

## Opening by byte ranges (issue #205)

A browser that opens a container with HTTP range requests cannot hand the module the file, and the module
cannot call back into JS and wait for a `fetch`: the module has no import of its own (the loadable module of
issue 202 has one, the externref table), and a synchronous callback would need a synchronous request. So the
range callback is a loop the host runs, and the module says what it needs. `pvec::PvecReader::open_ranges` is
the Rust side (the `http` feature of the facade, which this made real): the reader reads through a function that
returns the bytes of a range. Here that function reads what the host has provided, and when a range is not
there it notes it and stops the call, which then returns -7.

```
h = open_ranged(length)                    // nothing is read yet
loop: r = open_poll(h)
      if r == -7: for i in 0 .. needed_count(h): provide_range(h, needed_offset(h, i), bytes of that range)
      else break                           // r == h, and h is now a file; r < 0 is an error
// any call that reads (query_viewport, tier0_viewport) on the file does the same:
loop: r = tier0_viewport(h, ...); if r == -7: provide what needed_* names, and call again
```

Rules:

* A call that returns -7 changed nothing, so the same call is made again after the ranges are provided. The
  module names every range that call needs at once (blocks close together are one range, the way the
  reader plans them), so a window costs one round trip, not one per block. Opening is sequential by nature: the
  header, then the table and the directory, then the spatial directory.
* Opening reads the header (4 KiB), the block table and directory, and the spatial directory block, and no
  entity. Over a 109 KB test file that is three requests, the two a container open makes and the one block of the
  spatial directory. `directory_stat`, `directory_bound` and `directory_query` then need nothing more.
* `tier0_viewport` needs the Tier 0 block of each leaf the window touches. A leaf the window holds whole costs
  no hot block, because every entity of it is in the window and the leaf says how many there are. A leaf the
  window only cuts costs its hot blocks, because `uncovered` and the cull by the bounds of each entity need
  them, and the advanced curves cost their chunks when their bounds can meet the window. `query_viewport`
  needs the hot blocks of the leaves it touches and the same advanced chunks. A file with no directory has
  nothing to cull by, so its first `query_viewport` needs all of it. The results are the ones `open` gives for
  the same bytes, which a test compares for several windows.
* What was provided stays: a range may be bigger than what was asked (a host that fetches a larger first
  range saves round trips), a repeated call needs nothing, and the module keeps the provided bytes until the
  handle is released. The host decides what to provide, so it decides the memory.
* A range that is not inside the file is -3. A handle that is not range-opened has no ranges: `provide_range`,
  `open_poll` and `needed_*` on it are -5. A call on a handle that has not finished opening is -5. What the host
  provides is untrusted like any bytes: bytes that are not a PVec file are -4 from `open_poll`, and a block that
  does not check is -4 from the call that read it, whatever the host sent.
* The ABI version stays **2**. The calls are additions, and -7 comes only from a range-opened handle, so every
  call of version 2 on a file made by `open` works as before. A consumer that wants ranges tests that the module
  exports `open_ranged`, which it needs to do once at start, next to the version check.

## The spatial directory and the Tier 0 stream (version 2)

A file may carry a spatial directory (P5.1), an R-tree over the source-space boxes of its hot entities
(segments, polylines, circles, ellipses and arcs of either), and one Tier 0 block per leaf (P5.4b). The
writer puts them there with `PvecWriter::new_indexed` (`unstable-stream`). Splines and composites are the
advanced curves: they are in no leaf, so no leaf has a Tier 0 entry for them.

`directory_stat(file, n)`: 0 whether the file has a directory (0 or 1; the others are 0 when it has not),
1 leaves, 2 height, 3 hot entities in the leaves, 4 leaves with a Tier 0 block, 5 whether the advanced curves
have bounds. `directory_bound(file, n)`: 0 to 3 are `min_x, min_y, max_x, max_y` of the whole tree, 4 to 7
the same for the advanced curves, NaN when there is none.

`directory_query` returns the leaves whose box touches the window (closed boxes), in ascending cell id, as
five arrays (`cells_len`, `cells_ptr` and `cells_elem` take the array number):

| # | array | element | stride |
|---|---|---|---|
| 0 | cell ids | `Uint32Array` | 1 |
| 1 | hot entities in the cell | `Uint32Array` | 1 |
| 2 | 1 when the cell has a Tier 0 block | `Uint32Array` | 1 |
| 3 | bytes the cell's hot blocks take in the file as stored | `Float64Array` | 1 |
| 4 | `min_x, min_y, max_x, max_y` of the cell | `Float64Array` | 4 |

`tier0_viewport` returns the Tier 0 stream of the window: one entry for each hot entity of the leaves the
window touches whose own bounds meet it, ascending by entity id. Five arrays (`tier0_buffer_*`):

| # | array | element | stride |
|---|---|---|---|
| 0 | entity id | `Uint32Array` | 1 |
| 1 | code | `Uint32Array` | 1 |
| 2 | declared error, in world units | `Float64Array` | 1 |
| 3 | offset table, `entries + 1` entries, in vertices | `Uint32Array` | 1 |
| 4 | preview vertices, `x, y` in world space | `Float64Array` | 2 |

Codes: 0 exact (the hot block has the geometry, the declared error is 0 and the range is empty), 1 a preview
that is the control polygon, 2 a preview that is a chord polyline, 3 declared infeasible because the evaluator
refused the curve, 4 because no preview fits the vertex cap, 5 because the preview misses the bound, 6 for a
reason this version does not know. For a preview the declared error is the bound the writer measured and
stored: every point of the curve is within it of the preview polyline, and the vertices are the stored `f32`
offsets added back to their `f64` origin. An infeasible entry has NaN as its declared error and no vertices.
A preview is display fidelity, never exact.

`tier0_stat(result, n)`: 0 entries, 1 exact, 2 previews, 3 infeasible, 4 uncovered, 5 leaves read, 6 leaves of
the window with no Tier 0 block, 7 whether the bounds of the advanced curves meet the window. Uncovered is
what `query_viewport` would return for the window less the entries: the advanced curves, and every hot entity
of a leaf that has no block. So `entries + uncovered` is the entity count of the window, and a window is fully
covered exactly when `uncovered` is 0. The Tier 0 work lives in `pvec-stream`; the evaluator, not this layer,
computes the bounds.

A file with no directory answers `directory_query` and `tier0_viewport` with -6. A file whose directory or a
Tier 0 block the reader refuses still opens (the entities do not depend on it), and those calls return -4.
Windows must be finite and ordered (`min <= max`), or the call returns -3. These are the handles and the
memory rules below, unchanged: `release` frees them, a view is current while `memory_epoch()` is, and both
result kinds keep what they need after the file is released. A file opened by ranges answers these calls too (above).

## Rasters (version 2, additive; issue #221, ADR 0018)

A file may carry raster placements (capability 21) and their tiles (`docs/RASTER.md`). The module stores and returns
the **encoded** tile bytes untouched and decodes nothing: it has no image codec in it, and
`crates/pvec-checks/tests/crate_boundaries.rs` fails if a dependency of `pvec-wasm` is an image crate. The host
decodes with `createImageBitmap`, orientation "none" and colour-space conversion "none". These exports are
**additions to version 2**: `wasm_abi_version` still returns 2, and a consumer finds them by testing that the module
exports `raster_stat`, once at start, next to the version check (the way `open_ranged` is found). A module without
it is a version 2 module that predates rasters, and every other call works. Every test below is in
`crates/pvec-wasm/tests/issue_221_raster_exports.rs` (`issue_221_raster_exports`), named for what it holds.

`raster_stat(file, n)`: 0 rasters in the file, 1 rasters skipped, 2 whether the file has an image this reader cannot
show (`Effect::Unrastered`, 0 or 1: some raster is skipped, or the raster stream is refused), 3 whether the file has a
raster stream at all (0 or 1). `raster_skipped(file, i, n)` for the `i`th skipped raster (ascending raster id): `n` 0
the `RasterId`, 1 the number of the `RASTER-NN` rule that skipped it (4 for `RASTER-04`, an unknown `tile_codec`,
which is known from the placement; the rule of a raster a call found refused, such as a failed `RASTER-03` sniff, is
added to the list by that call). On a range-opened file the first raster call may return -7 until the block that holds
the placements is provided.

`raster_placement(file, r)` returns a result handle for raster number `r` (the dense `RasterId`). Arrays are read with
`raster_buffer_len`, `raster_buffer_ptr` and `raster_buffer_elem` (result, array number):

| # | array | element | stride |
|---|---|---|---|
| 0 | `pixel_to_source`, PVec `Transform2` order `[a b c d tx ty]`, the authored bits | `Float64Array` | 6 |
| 1 | `width, height, visible_box (x0, y0, x1, y1), tile_edge, levels, tile_codec, channels, bit_depth, opacity, band, layer_index` (`layer_index` is 4294967295 when the raster has no layer) | `Uint32Array` | 14 |
| 2 | `attribution`, plain UTF-8 text, the host escapes it | `Uint8Array` | 1 |
| 3 | `source_digest`, 32 bytes, or empty | `Uint8Array` | 1 |

`raster_plan(file, min_x, min_y, max_x, max_y, scale, max_decoded)` returns a result handle: for every raster that is
not skipped and whose footprint meets the window, the level and the tiles the viewport needs, which is
`pvec-stream`'s `plan_placed` (STREAMING.md section 6) and nothing else. `scale` is display pixels per source unit;
`max_decoded` is `LIMIT-16` in bytes, and 0 means the module's (the browser preset). The result is a prefetch hint for
the host's range cache: the tiles are not read. A raster that `LIMIT-16` cannot fit even at its coarsest level is left
out. On a range-opened file the module names the index blocks of every raster the window touches in one round (-7).
Arrays (`raster_buffer_*`; `n` rasters in the result):

| # | array | element | stride |
|---|---|---|---|
| 0 | raster ids, ascending | `Uint32Array` | 1 |
| 1 | level chosen, after `LIMIT-16` | `Uint32Array` | 1 |
| 2 | tile table: `n + 1` offsets into arrays 3 to 7, in tiles | `Uint32Array` | 1 |
| 3 | `col, row` of each tile | `Uint32Array` | 2 |
| 4 | block id of the chunk, or 4294967295 for an inline tile | `Uint32Array` | 1 |
| 5 | file offset of the tile's first byte (an integer below 2^53) | `Float64Array` | 1 |
| 6 | stored length | `Uint32Array` | 1 |
| 7 | 1 for an inline tile (already in the module, not requested), else 0 | `Uint32Array` | 1 |
| 8 | the merged byte ranges `offset, length` of the stored tiles (`ReaderConfig::gap_threshold`, tile with tile only) | `Float64Array` | 2 |
| 9 | range table: `n + 1` offsets into array 8, in ranges | `Uint32Array` | 1 |

`raster_tile(file, r, level, col, row)` returns a result handle whose array 0 is the encoded bytes of that tile
(`Uint8Array`, element type 3), after the index's XXH3-64 check and before anything else. It follows the -7 protocol
of issue #205 on a range-opened file: the module names the ranges it needs in turn, the raster's index block first and
then **the tile's own range**, never the chunk around it (`needed_*`), and there is no second mechanism. The module
reads the tile by range (`ContainerReader::read_raster_tile`) and never through the whole-block `read_stored`, so the
chunk's block checksum is not what vouches for the tile; the index checksum is.

**Cache policy (the alternative to a range-release export).** The bytes of a tile live in three places: the range the
host provided, the module's decoded cache (none for a tile: the index is cached, the tile is not) and the result
handle. `release(handle)` frees the handle **and drops the tile's range from the ranges the host provided**, keeping the
rest of any larger range that held it, so a host that releases a tile handle after `createImageBitmap` has copied the
bytes holds no tile in the module, and asks again (the host's own cache can answer) if it wants the tile once more.
There is no separate range-release export. The #220 measurement decides whether that is enough; if it is not, a
`release_range` export is an addition of the same kind.

Codes (negative, additions to the table below): -8 the file has no raster stream, -9 the tile is not stored (a sparse
pyramid: it draws nothing), -10 the raster is skipped by a `RASTER-NN` rule (`raster_skipped` says which). -3 keeps
meaning a bad id, level, column or row, and -4 a failed checksum (the index's or, for the index block, the block's).

The element type table has four entries: 0 `Float64Array`, 1 `Float32Array`, 2 `Uint32Array`, 3 `Uint8Array`
(`raster_buffer_elem` and the other `*_elem` calls return it).

## Conformance

Every result carries the tag `DisplayFidelity` (`tessellation_conformance` returns 1, and no result
exists without it). The vertices are a polyline for drawing, within the budget the caller gave and not
the curve itself. A consumer must not present them as exact geometry, and there is no call that returns
exact geometry in a packed buffer.

## Memory ownership

* The arrays live in WebAssembly linear memory, owned by the result handle. The typed arrays JS makes
  are views, not copies.
* `release(handle)` frees a handle of any kind. A released handle, a second `release` of it and any use
  of it return -1; they never read freed memory. A handle carries a generation, so a handle that was
  released stays refused for the next 32,767 reuses of its slot, then the generation wraps and the same handle value
  can come back. The generation is 15 bits so a handle is always below 2^31 and never reads as an error code.
* Memory can grow (`memory.grow`) during any call that allocates. When it does, every typed array made over the old
  `memory.buffer` is detached. The rule: read `memory_epoch()` when you read a pointer; a view is
  current while `memory_epoch()` still returns that value. `view_current(handle, epoch)` checks it
  and returns -2 when memory has grown, in which case make the views again from `buffer_ptr` and
  `buffer_len`. The epoch is the size of memory in pages, which only grows. `force_grow()` grows memory by one
  page so a consumer can test its handling.
* Read `buffer_len` before `buffer_ptr`: the length returns the error code, the pointer returns 0.

## Limits (issue 217)

Every open path of the module, `open` and `open_poll` alike, holds the file to `Limits::BROWSER`. Until issue 217 the
module opened with `Limits::DEFAULT`, which is the native preset, so the browser values of `docs/SECURITY_LIMITS.md`
meant nothing in the tab they were written for. This changes the geometry limits the module enforces too, not only
the raster ones: a file that `Limits::DEFAULT` admitted and `Limits::BROWSER` refuses (a block over 64 MiB, an
expansion ratio over 32, a curve of more than 16,384 control points) is now refused with error code -4. The values
that differ, before and after:

| limit | field | `Limits::DEFAULT` (before) | `Limits::BROWSER` (now) |
|---|---|---|---|
| LIMIT-05 | `max_artifact_bytes` | 68,719,476,736 (64 GiB) | 4,294,967,296 (4 GiB) |
| LIMIT-02 | `max_block_bytes` | 268,435,456 (256 MiB) | 67,108,864 (64 MiB) |
| LIMIT-06 | `max_blocks` | 1,048,576 | 131,072 |
| LIMIT-01 | `max_expansion_ratio` | 64 | 32 |
| LIMIT-07 | `max_entities_per_artifact` | 268,435,456 | 16,777,216 |
| LIMIT-08 | `max_entities_per_block` | 268,435,456 | 1,048,576 |
| LIMIT-09 | `max_control_points` | 65,536 | 16,384 |
| LIMIT-10 | `max_knots` | 65,553 | 16,401 |
| LIMIT-11 | `max_nesting_depth` | 4 | 3 |
| LIMIT-03 | `max_alloc_bytes` | 2,147,483,648 (2 GiB) | 268,435,456 (256 MiB) |
| LIMIT-04 | `max_decode_work` | 1,099,511,627,776 | 17,179,869,184 |
| LIMIT-14 | `max_raster_tiles` | 1,048,576 | 131,072 |

The other limits are the same in both presets (LIMIT-12 is the artifact's own length, LIMIT-13 is 4,096 rasters and
LIMIT-15 is a header allowance of 65,536 bytes). The ABI version stays 2: no export or layout changed, and a host
that opens only files inside the browser values sees no difference. `crates/pvec-wasm/tests/issue_217_browser_limits.rs`
holds a file the defaults admit and the browser preset refuses.

## Error codes

| code | meaning |
|---|---|
| -1 | the handle was never issued or has been released |
| -2 | the view is stale: memory has grown since it was read |
| -3 | an argument is out of range or not a number |
| -4 | the bytes are not a PVec file this reader accepts |
| -5 | the handle is live but is not the kind this call takes |
| -6 | the file has no spatial directory, which the call needs (version 2) |
| -7 | the call needs byte ranges of a range-opened file that were not provided yet: `needed_*` names them (issue #205) |
| -8 | the file has no raster stream (issue #221, additive) |
| -9 | the raster tile is not stored: the index says length 0, and it draws nothing (issue #221, additive) |
| -10 | the raster is skipped by a `RASTER-NN` rule, `raster_skipped` names it (issue #221, additive) |

## Size

(Measured at version 1; version 2 adds 11 entry points and I have not measured it again.) `pvec_wasm.wasm`, `cargo build --release --target wasm32-unknown-unknown`, no `wasm-opt` and no
`wasm-bindgen` post-processing, rustc 1.99.0 in a `rust:1.99.0-bookworm` container: 466,775 bytes built on
native x86_64 (MARS) and 466,223 bytes built on native arm64 (mercury). The module has 17 entry points of
ours and 84 exports the toolchain adds. The size is recorded for P8.2, and no ceiling is asserted.

Measured again for issue 217, at version 2: `cargo build --release --target wasm32-unknown-unknown -p pvec-wasm`, rustc
1.99.1 in the `pvec-rust` image on native x86_64 (MARS), the same flags and no post-processing. The module is 658,184
bytes with `Limits::BROWSER` wired in, and the commit before it built to 658,353 bytes under the same toolchain, so
wiring the preset and the raster ceilings cost nothing measurable. The 466,775 bytes above are the version 1 module
and are not comparable with these.

Measured again for issue 221, the raster exports, same command, toolchain and host: 668,425 bytes at the commit before
them (the plan of issue 220, on the raster blocks of issue 218) and 744,291 bytes with the eight `raster_*` exports,
so the group costs 75,866 bytes (+11.3%). Against the 658,184 bytes of the issue 217 measurement that is 86,107 more in
all, and the module is still a range-opened reader with no image codec in it (`crate_boundaries`). The staged
release directory (`tools/wasm-release.sh`, after `wasm-bindgen`) holds a module of 707,544 bytes, sha256
`336ae9a30668b003c7e2325b758a7fb05ee52cde5062a25a385a9030ea554ae1`, commit `149ed15`; `tools/wasm-release-selftest.sh DIR`
probes its raster exports on `tests/compat/raster-probe.pvec` and passes, and a module built without them fails the probe.

## The loadable module (issue 202)

The raw `cargo build --target wasm32-unknown-unknown` output is not loadable: it has hashed export names
and `__wbindgen_describe` imports. The post-processing that fixes it is `wasm-bindgen --target web
--no-typescript`, at the version in `Cargo.lock`, and the result is `pvec_wasm_bg.wasm` (one import,
`__wbindgen_init_externref_table`, the exports above plus memory and the allocator).

`tools/wasm-release.sh` does all of it in a throw-away container on the docker host, from whatever is
checked out, so it can be rerun on any commit. It stages a directory (default `dist/wasm-release/<commit>`)
with `pvec_wasm_bg.wasm`, `pvec-wasm-manifest.json` (commit, dirty flag, `abiVersion` read from
`WASM_ABI_VERSION`, wasm-bindgen and rustc versions, byte count, sha256) and `SHA256SUMS`.
`tools/wasm-release.sh --verify DIR` checks a staged or downloaded directory against its own digests.
Two builds of the same commit gave identical bytes. It never tags, pushes or publishes.

To publish a staged directory, someone with release rights runs, from a clean checkout of the commit in the
manifest: `git tag wasm-v<abi>-<short commit> <commit>`, `git push origin <tag>`, then
`gh release create <tag> --draft --verify-tag dist/wasm-release/<commit>/*`, checks the draft, and publishes it.
