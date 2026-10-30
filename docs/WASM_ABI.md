# The WASM packed output ABI (issues P7.3 and #201)

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

`open` takes the file whole. The reader works on bytes in memory, and the `http` feature of the facade
is a marker, so a range callback is not here yet (issue #205).

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
result kinds keep what they need after the file is released. Byte ranges of blocks, and a `open` that takes a
range callback, are not here: the second waits for the `http` feature of the facade.

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

## Error codes

| code | meaning |
|---|---|
| -1 | the handle was never issued or has been released |
| -2 | the view is stale: memory has grown since it was read |
| -3 | an argument is out of range or not a number |
| -4 | the bytes are not a PVec file this reader accepts |
| -5 | the handle is live but is not the kind this call takes |
| -6 | the file has no spatial directory, which the call needs (version 2) |

## Size

(Measured at version 1; version 2 adds 11 entry points and I have not measured it again.) `pvec_wasm.wasm`, `cargo build --release --target wasm32-unknown-unknown`, no `wasm-opt` and no
`wasm-bindgen` post-processing, rustc 1.99.0 in a `rust:1.99.0-bookworm` container: 466,775 bytes built on
native x86_64 (MARS) and 466,223 bytes built on native arm64 (mercury). The module has 17 entry points of
ours and 84 exports the toolchain adds. The size is recorded for P8.2, and no ceiling is asserted.

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
