# ADR 0020: The viewer is a page over pvec-wasm, not a map library layer

Status: Accepted, 2026-11-07. Decides the viewer half of #263 (the owner picked option 1 there). Builds on ADR 0018
(rasters in the artifact) and on the module contract in `docs/WASM_ABI.md`. ADR 0019 (the calibration stream) is the
other half of #263 and is decided separately.

I ran no benchmark for this ADR. The sizes below are the module as `tools/viewer-module.sh` built it, and the rest is
what I read in this repository and in the archived `libviprs/maplibre-pvec`.

## Context

Nothing in the org displays a `.pvec` today. `libviprs/maplibre-pvec` was the MapLibre GL JS layer that did, and it is
archived: we are not going to use MapLibre to display PVecs (#263). Since ADR 0018 a file can carry rasters next to the
vectors, and libviprs converts a PDF with both into one `.pvec` (libviprs#1491), so a converted drawing is a file that
only a reader of our own can show whole: the vectors exactly, the rasters from their tile pyramid, both in the one
source space that `RASTER.md` 1.1 defines.

The module already does the hard part. `open` (or `open_ranged` over HTTP ranges), `query_viewport` and
`tessellate_batch` cull and tessellate with the error measured on screen after the whole transform chain, and the
raster group (`raster_plan`, `raster_tile`, `raster_placement`, `raster_stat`) plans the tiles of a window and returns
their encoded bytes for the host to decode. What is missing is a host.

## Options

1. **A static page over the module** (`viewer/`): plain ES module, a 2D canvas, no framework, no map library and no
   build step. It calls the exports above and draws in source space. The module is the one `tools/wasm-release.sh`
   stages, 802,203 bytes after `wasm-bindgen` at commit `902d9c3` (rustc 1.99.1, built on native x64 on the NAS); the
   page itself is two files.
2. **Lift the archived layer's WebGL renderer out of MapLibre.** It has the raster quads and the georeferencing
   (`maplibre-pvec`, `docs/georeferencing.md`), but every part of it is written against a map camera (Web Mercator,
   `CrsAdapter`), which is the dependency #263 rules out, and it would be our only WebGL code to carry.
3. **Only export the PMTiles pair** (`INTERCHANGE.md` chapter 8) for clients that are not ours. That already exists
   and stays, but a tile archive is display fidelity: it cannot show the vectors exactly, and it is not a viewer.

## Decision

Option 1. The page lives in `viewer/` and is what a person opens to look at a `.pvec`; it is a reader of the format,
so it holds itself to `WASM_ABI.md` and refuses a module whose ABI version it does not know. It is built in slices:
the vectors (#271), the rasters under them by their band (#272), opening by HTTP ranges with `If-Range` (#273), and
the scale and georeference from the calibration block (#274, once ADR 0019's exports are on main).

Rules the page keeps:

* It draws in **source space** and never reprojects. A raster lands through `pixel_to_source`, a curve through the
  module, both under one view transform. A georeference, when the file has one, is shown (the CRS id, the position
  under the pointer through the stored affine), not applied; there is no CRS database in the page.
* Every number the module returns that is an error code is shown as one, with the code, never swallowed.
* Captures from its tests stay out of git.

Its browser tests are Playwright specs in `viewer/tests/`, named by issue, that `tools/viewer-test.sh` runs in the
Playwright container on a docker host after building the module there. On this repository's hosts that is native x64
(the NAS) or arm64 (mercury); the specs say nothing about either.

## Consequences

* `docs/INTERCHANGE.md` chapter 9 says which route a reader takes: this page for our files, the PMTiles pair for a
  client that is not ours.
* The repository gains a small JavaScript surface (two page files, the specs and a test server) and one pinned test
  dependency, `@playwright/test` 1.50.1, used only inside the container.
* Line styles, hatches and text from the component streams are not drawn yet: every curve is one stroke colour. They
  come with a later slice and change nothing above.

## Reversal threshold

I would revisit option 2 (a WebGL renderer, ours) if a converted drawing of 1,000,000 entities cannot pan at 30
frames a second in this page on a mid-range laptop once Tier 0 previews are drawn, or if more than 3 of the slices
above need a feature the 2D canvas does not have.
