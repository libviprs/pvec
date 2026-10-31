# ADR 0018: Raster graphics inside a PVec artifact

Status: Proposed, 2026-10-30, revised 2026-10-30 after a 7-role expert review (see "Review"). Decides #212.
Awaiting Rom's ratification, and depends on the ratification of ADR 0017, which it amends (see "What this
scopes"). Builds on ADR 0002 (authored bits), ADR 0005 (the index), ADR 0006 (per-block codec and checksum),
ADR 0007 (tiers), ADR 0008 and ADR 0010 (extension rules, the registry), ADR 0013 (governance) and ADR 0014
(component streams).

I ran no benchmark for this ADR. Every number below is a constant or a document I read (file references inline),
or plain arithmetic on them, and each one that I could only estimate is called a proposal. The measurements are
the first stages of the plan.

## Context

PVec holds curves and nothing else. ADR 0017 keeps it that way for instances ("PVec stays curve-only: an artifact
holds no block table, no instance and no transform"), `docs/INTERCHANGE.md` lists raster tiles as out of scope for
the MVT export (chapter 5), raster pyramids as not done for PMTiles packaging (chapter 6) and raster and tile tables
as not read from GeoPackage (chapter 4), and `docs/ISO_PROFILE.md` has no image construct. All of that is right for
geometry, and none of it was ever a decision about pixels: the word appears in those places because the export
chapters stop at linework.

What I want is a drawing that carries its scanned underlay, photo or placed image next to the vectors, in the same
file, with the same range-request story. Today that is two objects: the `.pvec` and a PMTiles archive or tile tree,
joined by hand in the MapLibre style.

What libviprs already does with rasters (read from `libviprs/libviprs` at `a81c9fbc` and `libviprs-cli` at `334ab9a`):

* It cuts a raster into a tile pyramid. `PyramidPlanner::new(w, h, tile_size, overlap, layout)` (`src/planner.rs`)
  gives square tiles of any non-zero size (the CLI default is 256, `libviprs-cli/README.md`), levels that halve with
  a ceiling down to 1x1 for DeepZoom and Xyz, an optional centring that shifts the image inside a padded grid
  (`planner.rs`, `centre_offset_at_level`), and a Google layout on a square power-of-two canvas. The tile formats
  are `Png`, `Jpeg`, `Webp` (lossless only, `src/webp.rs`) and `Raw` (`src/sink.rs`).
* It packs the tiles into one PMTiles v3 archive, now the default (`src/sink_pmtiles.rs`, `src/pmtiles/`). PMTiles
  has no tile format for `Raw` (raw pixels carry neither dimensions nor pixel format, `TileType::try_from_tile_format`),
  addresses tiles as `z/x/y` with row 0 at the top (`COORDINATE_CONVENTION = "zxy"`), and caps the zoom at 31
  (`MAX_ZOOM`). Its reader bounds the root directory, leaf depth, directory and metadata sizes (`src/pmtiles/reader.rs`).
  A directory entry costs 4 to 7 bytes before gzip, consecutive identical tiles collapse into one run-length entry,
  and an identical payload is stored once (`src/pmtiles/directory.rs`, `writer.rs`).
* It georeferences with a six coefficient affine from pixel to geographic space (`src/geo.rs`, `GeoTransform`:
  `geo_x = a*px + b*py + c`, `geo_y = d*px + e*py + f`, origin at the top-left corner of pixel (0,0), GDAL's
  `PixelIsArea`). The CLI builds one from `--geo-origin` and `--geo-scale` and **prints the bounds**
  (`libviprs-cli/src/main.rs:1047-1054`, an `eprintln!`; nothing downstream uses the transform). The transform is
  never written into the archive: the header's E7 bounds are a writer option that defaults to the whole world
  (`src/pmtiles/writer.rs`, `bounds_degrees`) and the `vnd.libviprs` metadata carries source size, pixel format and
  the generation options (tile size, overlap, layout, format, centre, blank strategy), not a transform. One e2e test
  (`libviprs-tests/tests/pipeline_e2e.rs`, the GCP case) passes `image_bounds` into the header by hand.
* **How a libviprs raster pyramid is placed in MapLibre today**, which the first draft of this ADR left open: the
  archive is on MapLibre's grid. `tile_coord_to_zxy` (`src/sink_pmtiles.rs:129-138`) maps level to `z`, column to
  `x` and row to `y` with no inversion, so for the default Xyz layout pixel (0,0) of every level is the north-west
  corner of the Web Mercator world. MapLibre can load that archive as a raster source and will draw the image in
  the top-left corner of the world at the scale the level numbering implies, never where a `GeoTransform` says,
  and never in the vector tiler's synthetic world (`src/vector/tiler.rs`: the drawing fitted centred into the zoom 0
  unit square, y flipped). No code re-grids a raster onto that world, and the reference viewer
  (`web/vector-viewer/`) has one vector source and no raster source (`src/vector/black_on_white.style.json`). So a
  raster pyramid and a vector drawing from the same libviprs cannot be overlaid today; that is the gap this ADR
  closes from the container side.
* Its PDF to PVec mapper walks Form XObjects only and returns without a record for an image XObject
  (`src/pvec/pdf_vector.rs`, the `is_form` test); inline images are lexed past. `docs/adr/pvec-pdf-policy.md` counts
  76 non-uniform scales in one real PDF set and says they are image placements, and 1,035 of 4,456 clips in another
  set are not rectangles. There is no image policy, and no counting of what is dropped.
* A DWG `IMAGE` names an external file and no reader in this workspace opens it: the native decoder refuses it by
  design (`libviprs-dep/acadsharp/native/Adapter/RefusedKinds.cs`, "this decoder opens no path a drawing names")
  and the acadrust provider leaves `RasterImage` unmapped (`src/pvec/dwg_acadrust.rs`). So the v1 producers with
  pixels in hand are a PDF (pixels embedded) and a standalone raster; a DWG image is a placement with no pixels.

On the PVec side, the pieces a raster has to live with:

* Everything that is an entity has an `EntityId` and resolves to a `CurveKind` (`crates/pvec-entity`, `EntityTable`),
  ids are dense `0..n` (`docs/ENTITY_ID.md`).
* Components are optional streams that refer to entities by `EntityId`, embed no geometry and are never required
  (ADR 0014, `NEGOTIATE-04`, `COMPONENT_STREAMS.md` "by reference, never by value"). The set is one block read whole.
* A block has a 64 byte directory entry with kind, subtype, cell, count, bounds, codec and capability; the directory
  and the 8 byte per block checksum table are fetched whole at open (`LIMIT-06`, `engine.rs`); block kinds 1 to 8 exist
  and a reader skips an unknown optional kind (`docs/CONTAINER.md` chapter 4, ADR 0008). DIR-02 refuses a non-zero
  capability id on a kind the format defines (`directory.rs`), and chapter 5.4 item 2 says a capability-owned block
  names its capability in the entry: the two agree today only because no capability owns a block kind yet. LAYOUT-04
  refuses two blocks with the same (group, cell, kind, subtype) key; the tie-break inside a key is arrival order.
* Each block has an XXH3-64 over its stored bytes, verified before it is decompressed when the file carries a
  checksum table (`require_integrity` is off by default, `reader.rs`), and a block is limited to `max_block_bytes`
  (`LIMIT-02`: 256 MiB, browser 64 MiB). The straddler lists (STREAMING.md 3.8) already read ranges inside one block
  without the whole-block checksum; only full validation checks the whole block.
* The WASM module opens with `Limits::DEFAULT`, not the browser preset: nothing in `pvec-wasm` names `Limits::BROWSER`.
* The numeric policy says coordinates are authored bits and one place defines tolerances (ADR 0002). An affine is
  validated by NP-19 (`Policy::validate_affine`, layout `[a, b, c, d, tx, ty]`, linear entries at most 2^20,
  singularity relative to the magnitudes). `libviprs` has its own singularity test with a relative `1e-12` in
  `src/geo.rs`; PVec does not adopt that.

## Options

| option | what it is | for | against |
|---|---|---|---|
| A. One block per tile | a `raster` stream holding the placements, and a new optional block kind, one block per tile, each with the tile's encoded bytes (PNG, JPEG or WebP) | per-tile range fetch through the directory the reader already has; the artifact digest covers every tile; skipping is already defined for an unknown optional kind | LAYOUT-04 refuses equal keys and every tile of one level shares a key, so the layout does not open; the order inside a level is arrival order and cannot be checked; the directory is fetched whole at open at 72 bytes a block (entry plus checksum), so one A0 scan at 400 dpi (18,720 x 13,240 px) adds 1,308 tiles at 512 px (94 KB, about 4.7 times the roughly 20 KB geometry cold open of the million entity corpus after P8.6) or 5,156 at 256 px (371 KB), ten of them 3.7 MB and the P8.5 `directory-300ms` verdict fails; the 0.5% directory line of ADR 0008 is crossed by tiles under 12.8 KB, which is the normal tile of a mostly white scan; DIR-08 forbids payload sharing, so a blank tile is never free |
| A'. A per-raster index block plus packed tile blocks (**chosen**) | one `raster_index` block per raster (level table, per-tile offset, length and XXH3-64, the coarsest levels' tiles inline for Tier 0) and `raster_tiles` blocks holding the encoded bytes of one (raster, level, chunk), read by range through the index | two to a dozen directory entries per raster instead of thousands, so the cold open barely moves; per-tile checksum in the index; a tile not stored is a zero-length index entry, so sparse pyramids are free; the index is read lazily, like the reference arrays (STREAMING.md 2.3.1) and the straddler pages (3.8), which are the precedent for ranged reads inside a block; all index blocks are adjacent in the canonical order, so every raster's overview comes in one range | a second index format to specify, fuzz and validate; a ranged tile read is not covered by the block's whole checksum (the index entry's XXH3 is the per-tile check) |
| B. An embedded PMTiles archive as one block | the whole archive from `PmTilesSink` stored as an opaque block | byte-identical with what libviprs writes and `viprs pmtiles extract` reads; the `pmtiles` JS reader works on an offset-shifted range source; a directory entry costs 4 to 7 bytes and identical tiles are stored once | `LIMIT-02` (64 MiB in the browser) caps the image; a second directory system with its own gzip, which has no uncompressed length (`Compression::decompress` makes the caller name a ceiling), sits inside the first and PVec's planner cannot see its ranges; hostile-input bounds have to be reimplemented in two parsers (Rust and the browser's); the archive carries its own world grid and header bounds, which say nothing about source space |
| C. An external reference (URL or path plus a digest) | the placement and georeference live in the PVec, the pixels stay in a PMTiles or tile tree | file stays small; PMTiles hosting and CDN caching are reused as they are | a second object to host, with CORS and range support; the artifact digest no longer covers the pixels; a file that makes the viewer fetch a URL it chooses is a privacy and request-forgery surface that no PVec reader has today (`PVecSource` takes its URL from the constructor and reads nothing else); no offline use. A same-origin relative reference (C') removes the forgery surface but keeps the other three |
| D. Status quo | raster stays a MapLibre source beside the PVec layer | works now: ADR 0001 of maplibre-pvec adds the custom layer with `beforeId` among style layers, so draw order against a raster layer is already controllable | two objects, two caches, two failure modes; the georeference is wired by hand twice, and today it cannot be wired at all because the archive is anchored at the world corner (Context); a converted PDF that had images cannot carry them in its own artifact |
| D'. D plus a stored georeference | libviprs writes the `GeoTransform` into the PMTiles metadata and maplibre-pvec composes it with the drawing's calibration | most of the user value for a standalone scan with no format change; needed by A' anyway (stage 6 has to compose the same transform) | still two objects, no PDF images, no offline use |
| E. Resample to an axis-aligned placement | the producer warps the image into an axis-aligned source-space grid at write time, the placement becomes a box, no six coefficients | the raster analogue of ADR 0017's flattening; MapLibre's own raster source could then draw it through a protocol | resampling pixels is lossy and not reversible, so ADR 0002's authored-bits rule argues against it where a transform keeps the bytes exact; a rotated scan grows by its bounding box |

The reviewed options table has native x64 and arm64 numbers only where it cites them: the cold open costs come
from `docs/BENCHMARKS.md` (P8.5, measured on native x64 and arm64), the rest is arithmetic.

## Decision

**A'**, with the changes the review forced on how I phrased it in the brief.

1. **The placement is not an entity, and the stream is not a component.** `EntityTable` resolves ids to a
   `CurveKind`, so giving an image an `EntityId` would put a non-curve into the type the geometry oracle iterates.
   Every written definition of a component (ADR 0008, ADR 0014, `COMPONENT_STREAMS.md`, `CONTAINER.md` 5.4 item 1,
   `COMPATIBILITY.md`) is "entity-keyed information about entities", and a raster is neither. So capability **21**
   `raster` is the first entry of a new registry class, **`Raster`** (the `Class` enum is `#[non_exhaustive]`):
   optional by construction, never required, refers to no entity, and may own block kinds. ADR 0010's class list and
   chapter 5.4 item 1 ("adds information about the entities") get a dated amendment ("about the drawing") in stage 1.
   The stream is keyed by its own dense `RasterId`, assigned by the producer in source draw order. Layer visibility is
   a `layer_index` into the layer stream's name table (a table row, not an entity), ignored if that stream is skipped.
2. **Two things, not one, and the tiles are not blocks.** The placements (small, read whole at open) are a stream of
   the component set under capability 21, next dense id after `semantic_class` (20). The pixels live in two
   **capability-owned** block kinds: **9** `raster_index`, one per raster, and **10** `raster_tiles`, one per
   (raster, level, chunk). Neither is added to `BlockKind::ALL`: the entry carries capability 21 and the kinds are
   reserved in the kind table, so DIR-02 stands as written, an old reader skips them as `UnknownBlockKind` naming
   capability 21 from the entry (chapter 5.3), and a reader that implements 21 is handed them by negotiation. This is
   the first use of the chapter 5.4 item 2 path, and `negotiate` has to learn it (today it marks every kind outside
   the table unknown). A component stream cannot hold the tiles because the set is read whole; a block per tile
   cannot either (option A).
3. **PVec stores the encoded bytes and never encodes or decodes pixels.** The producer hands over tiles in the codecs
   it already writes, and the reader returns them untouched after the index checksum. The browser decodes with its
   own decoder (`createImageBitmap`), which is sandboxed and already there. This keeps image codecs out of the PVec
   crates and out of the WASM module (`pvec_wasm.wasm` is 466,775 bytes today, `WASM_ABI.md`; the 29 KB figure of my
   first draft was ADR 0004's spike module), and means PVec has no canonical encoding to pin: the bytes are authored,
   like coordinates.

PMTiles stays the way to ship a **standalone** raster pyramid, and D stays valid for a viewer that wants only that.
D' is cheap and I recommend opening it as a libviprs issue in parallel, because stage 6 has to compose the same
transform. A' is for a drawing that owns its raster. C is not taken now; it fits later as a placement whose tiles are
external, but only after a host policy for fetching exists, which is not this ADR's to write. E is rejected on
lossiness, and that rejection is the argument for carrying a transform at all (next section).

### What this scopes

ADR 0017 refused to store a per-object transform for three reasons: a reader would have to resolve instances to
draw, the spatial index is over world-space extents, and flattening is the source mapping's job. It could refuse
because flattening a curve through a `Transform2` is exact (ADR 0012): the world-space curve is derived, not
approximated. A raster has no exact flattening. Resampling pixels into source space is lossy and not reversible, so
ADR 0002's authored-bits rule points the other way: keep the bytes and carry the map. That is why this ADR stores a
2x3 affine for a content type where 0017 stores none, and it is a real exception to 0017, not a relabelling. The
other two reasons still hold and are honoured: tiles are kept out of the spatial index, and every viewer applies one
affine per raster, not a tree of instances.

ADR 0017's rule stands for **geometry**: no instance, no transform, no block table, every entity a curve. On
acceptance I amend 0017 with a dated section that says so (stage 1), after 0017 is itself ratified; I do not rewrite
its Decision. The same amendment touches the "curve-only" sentences of `INTERCHANGE.md`. The MVT, GeoPackage and
Part 21 exports stay raster-free (see "Interchange" below)...

Out of scope by name: `WIPEOUT` (a mask with a draw-order position and no pixels, it belongs to a fill or mask
component), `OLE2FRAME` (the one DWG candidate whose pixels are in the file, deferred), PDF, DWF and DGN underlays
(vector references), paper space (a placement is in the model-space view the artifact holds), and a scan-only sheet
with no vectors, which WRT-01 refuses today ("component streams alone are not an artifact") and which is open
question 1.

### Data layout

All integers and floats are little endian, as in the other streams. The byte layout goes in a new
`docs/RASTER.md` in stage 1; what is decided here is what it holds.

**Placement** (one per `RasterId`, in the stream):

| field | decision |
|---|---|
| `pixel_to_source` | six `f64` stored as authored bits in PVec's `Transform2` order `[a b c d tx ty]`, rows `[a b tx]` and `[c d ty]`: `x = a*px + b*py + tx`, `y = c*px + d*py + ty`, pixel origin at the top-left **corner** of pixel (0,0) (GDAL `PixelIsArea`; a world file or a `PixelIsPoint` GeoTIFF is shifted by half a pixel first), pixel y down. The target is PVec **source space**, the same space as the vectors, so alignment with the linework holds by construction. libviprs's `GeoTransform` `[a b c d e f]` maps to `[a b d e c f]`; maplibre-pvec's calibration uses SVG order (`x' = a*x + c*y + e`), and `RASTER.md` gives both conversions, the DXF `IMAGE` (insertion point, U and V vectors) and PDF CTM conversions, and one rotated, sheared test vector. Source space is y-up, so an upright image has `d < 0` and a negative determinant: that is the normal case, not a corner case, and a renderer must not cull by winding |
| validity | NP-19 by name: each coefficient passes NP-01 and NP-02, the four linear entries are at most 2^20 in magnitude, and the map is invertible under the rule ADR 0012 applies to `affine_transform_2d` (capability 13). A reflection or a non-uniform scale is legal (the PDF image placements are non-uniform); a singular map is refused, with PVec's rule and not libviprs's `1e-12`. The writer normalises `-0.0` to `+0.0` (NP-03) before the bits are compared. The four corners of the full-resolution footprint must also pass NP-02 |
| `width`, `height` | pixels of the full resolution level, at least 1, `u32` |
| `visible_box` | a pixel-space rectangle (`u32` x 4) inside `width` by `height`: the area the renderer draws and the footprint that goes into the reported extent. A clipped image (DXF clip boundary, PDF clip) has its clip baked into the alpha channel by the producer and this box set to the clip's bounds; an unclipped image has the full rectangle |
| `tile_edge` | a power of two from 256 to 1024. This is PVec's bound, justified by the decode and texture budget (a 1024 RGBA tile is 4 MiB decoded, a 2048 one is 16 MiB and breaks the per-plan budget below), not a producer default: libviprs accepts any size and defaults to 256. Tiles are full squares; an edge tile is padded by the producer, and the renderer clips to `visible_box` so a padded margin never draws |
| pyramid | `levels` at least 1, level `levels-1` is full resolution, each level above halves with a ceiling, the pyramid **stops at the first one-tile level**, no overlap, grid anchored at pixel (0,0) with `cols = ceil(level_width / tile_edge)`. `levels` is bounded by `width`, `height` and `tile_edge` (RASTER-01), not by PMTiles's 31, which is an export constraint. There is no `first_level`: the PMTiles export derives `z` from the Xyz numbering (`z_max = ceil(log2(max(width, height) / tile_edge))`) |
| `tile_codec` | 1 PNG, 2 JPEG, 3 WebP (lossless or lossy; libviprs writes lossless only), the values PMTiles types 2, 3 and 4 name. `Raw` is not allowed (no dimensions). AVIF and MLT are not v1: libviprs has no encoder for them. An unknown codec id skips that raster and reports it. JPEG is legal only with `gray` or `rgb` |
| `pixel_format` | channels `gray`, `gray_alpha`, `rgb`, `rgba`, and a bit depth of **1 or 8** (scanned plans are bilevel, and a 1-bit PNG is a valid PNG the browser decodes; 16-bit and palette tiles are converted by the producer). Colour is treated as sRGB display colour, as MapLibre raster sources do; no ICC profile in v1, and the host decodes with colour-space conversion off and orientation "none" so an embedded chunk cannot move or recolour a tile |
| `opacity` | `u8`, 255 is opaque, multiplied into the fragment alpha. A DXF `fade` is a constant alpha over an opaque background, so this is its exact equivalent; brightness and contrast are per-pixel and the producer bakes them |
| `band` | 0 underlay (below hatch fills and every stroke), 1 overlay (above both). Nothing finer in v1; per-entity interleaving (DRAWORDER) is not carried for strokes either, so the raster is no worse than the linework |
| `layer_index` | optional, see the decision above. A raster on a layer the view profile excludes is not written |
| `source_digest`, `attribution` | optional SHA-256 of the source raster for provenance (hex, never a link), and an attribution string of at most 255 bytes of plain UTF-8 text: no C0 or C1 controls, no U+2028/2029, no bidi overrides, refused as COMP-07 style rules; a host escapes it before any HTML sink such as MapLibre's attribution control |

**Georeference.** The placement stores no CRS and no geographic transform. maplibre-pvec takes the georeference of
the whole drawing as a constructor input and nothing reads one out of a `.pvec` (`docs/georeferencing.md`, a test
keeps it so), and a raster in the same source space rides that single georeference. A producer that starts from a
geo-referenced raster composes `pixel -> geo` with the inverse of the drawing's `source -> geo`, folding in the
metres-per-unit factor. That composition is affine only when the raster's CRS is the calibration's native CRS and the
calibration is affine; a raster in another CRS, or a drawing with a `projective` calibration, is reprojected by the
producer first (a resample, outside this ADR). This is deliberately **not** libviprs's pixel to geo shape: two
georeferences in one file could disagree. Where a CRS identifier should live so the out-of-band georeference can be
rebuilt is open question 2.

**Index block (kind 9, `raster_index`).** One per raster, capability 21 in the entry, `cell` = `RasterId` (the cell
field is namespaced by kind; readers filter by kind before cell, and the spatial directory only ever counts hot
blocks, so no leaf id collides), `subtype` 0, `count` = tiles stored, bounds = the axis-aligned box of the
`visible_box` footprint, bounds-meaningful set, codec 0. It holds a header, a level table (per level: `cols`, `rows`,
chunk count, first tile ordinal) and a tile table in Hilbert order inside each level (a viewport is two or three
runs rather than one per row; the writer owns the order, so it is free), one entry per tile: chunk, byte offset
inside the chunk, stored length and XXH3-64 of the tile's bytes. A length of 0 means the tile is not stored and
draws nothing, which is the sparse pyramid of `skip_blanks`. The coarsest levels, in order, up to the Tier 0 cap
(below) are stored **inline** at the end of the index block and their entries point there, so the first paint of
every raster is one range over the adjacent kind 9 blocks.

**Tile blocks (kind 10, `raster_tiles`).** One per (raster, level, chunk): capability 21, `cell` = `RasterId`,
`subtype` = `level * 1024 + chunk` (unique and ascending by level under LAYOUT-04), `count` = tiles in the chunk,
bounds = the box of the chunk's tiles, codec 0 (PNG, JPEG and WebP are already compressed, and the writer's "only
when smaller" rule keeps it at 0 anyway). A chunk never exceeds `max_block_bytes`; a level that would is split. A
tile read is a range inside the chunk at the index's offset, checked against the index's XXH3-64 before the bytes
leave the reader; the chunk's whole-block checksum is checked by full validation only, exactly as the straddler
pages are read today.

### Spatial index and range fetch

Tiles are **not** put in the Hilbert R-tree of ADR 0005. The tree is over entity centres and every payload is
owned by one leaf; a placement's bounds usually cover most of the drawing, which is the "long entity" case that
ADR 0005 measured congesting leaves (0.958 of the world on `straddler`), and a tile is addressed through the index,
not by search. The viewport plan for a raster is: fetch the index block if it is not cached (lazily, like the
reference arrays); choose the level from the larger singular value of the pixel-to-device Jacobian (so the image is
never under-sampled in its sharper direction, and one number exists under a rotated or non-uniform placement); map
the four viewport corners through the inverse of `pixel_to_source`, take their pixel-space box (it over-fetches
under rotation, about 1.8x in maplibre-pvec's measurement, and a renderer may cull tiles against the viewport
polygon), compute the (col,row) range in f64, saturate and clip it to the level; read the index entries; and hand
the byte ranges to the container's planner (`ReaderConfig::gap_threshold`, 64 KiB). Tile ranges merge only with
tile ranges: kinds 9 and 10 sort after every geometry block, so no gap ever bridges a tile and a hot block, and the
"same planner" is reuse of code, not of requests. The `visible_box` footprint is added to the extent a reader
reports for fit-to-view; the spatial directory's root box is left as it is, a union of entities (this was both a
decision and an open question in my first draft; it is a decision).

### Tier assignment

Progressive tiers (ADR 0007) are about a preview that is never labelled exact. A pyramid is that already: every
level above full resolution is a derived display image. The raster share of Tier 0 is the inline part of each index
block: the coarsest levels of a raster in order, until a per-file cap, **proposed 256 KB** over all rasters, is
reached (basis: `tier0-4s-5mbit` passed at 3.5353 s against 4 s on native x64, `docs/BENCHMARKS.md`, which is 290 KB
of headroom at 625 KB/s; a 512 px JPEG overview of a plan is tens of KB, a PNG one can be hundreds). The cap counts
against `tier_budget` in `plan_tiered` (extended in stage 4), the index blocks are one contiguous run by the
canonical order, and the renderer draws the finest inline level it has while finer tiles are in flight. There is no
`tier0_level` field: the reader sees what is inline. Nothing else about tier 0 changes except its timing, and the
P8.5 verdicts are re-run with a raster fixture in stage 4. No overview is described as exact anywhere.

### Compression and checksum

Per tile, XXH3-64 in the index entry, checked before the tile is handed on; per block, the ADR 0006 rule for the
index block (read whole) and for full validation of a chunk. What that buys, said honestly: XXH3 catches corruption
when the file carries a checksum table and is not a defence against an author, who writes the matching hash
(ADR 0006 chose it as a non-security hash on purpose); the artifact SHA-256 covers tile bytes exactly as it covers
blocks today, verified on request and never by a ranged viewer; `require_integrity` is the host's choice. The
hostile-input defences are the limits and rules below and the host decoder's sandbox. The container codec is 0.
The image codec lives in the placement, not in the directory's codec id, so `CODEC-01` to `CODEC-03` and `LIMIT-01`
keep meaning what they mean.

## Extension rules

* **Governance (ADR 0013).** Not a geometry extension: the gate in `Registry::register_extension` and the Part 42
  proposal are for `Class::Extension`, and `docs/EXTENSIONS.md` says style, text and hatch are not geometry
  extensions. 0013 is silent on who reviews a non-geometry entry; the precedent is an accepted ADR each (0014, 0015),
  and this ADR is that review. Registering 21 is a new registry entry with the new class, and its restriction text
  points at this ADR. ADR 0008's own reversal line on the flat component set ("more than three independent component
  kinds before v1") was passed at 14 to 20 and nobody reopened it; I record that here rather than pretend it did
  not happen. The process that applies is this ADR's status: Proposed until Rom ratifies.
* **Versioning.** The container stays 1.0. `FORMAT_VERSION` goes to 1.1.0, entry 21 is registered with `min_reader`
  and `status_since` 1.1.0, and this is "new optional blocks and new capabilities" under `MIGRATION.md`. The WASM
  ABI stays at **2** (below). `FREEZE-v1.md` pins the registry by sha256 and marks the extension rules FROZEN, so
  stage 1 updates that page on purpose: the sha, and a line that a capability may own block kinds.
* **Optional only.** Capability 21 is optional and kinds 9 and 10 clear must-understand. A file cannot declare it
  required: the `Raster` class is never required, by its own rule and not by inheriting NEGOTIATE-04.
* **The `min_reader` hoist.** The writer takes the manifest's minimum reader as the maximum over every declared
  capability, optional ones included (`writer.rs`), so an honest `min_reader` 1.1.0 for 21 would make a 1.0.0 reader
  refuse every raster file at NEGOTIATE-03 before it reaches the skip path. Stage 1 changes the hoist to required
  capabilities only (a CONTAINER.md 4.3 amendment, an optional capability is skippable by definition) and adds a
  compat matrix case "1.0.0 reader, raster file, `open_degraded`".
* **Old readers.** One that knows neither opens the file `OpenDegraded`: the capability is listed under
  `NotImplemented`, kind 9 and 10 blocks are listed as `UnknownBlockKind` skips by their directory entries, naming
  capability 21, the payload is never fetched, and the geometry is bit-identical. I add an `Effect` value
  (`Unrastered`) beside `Unstyled` and `Uncomponented`, with its `effect_of` branch and `describe` sentence, so a
  viewer can say "this file has an image your reader cannot show". `Effect` is reachable from neither the `pvec`
  facade nor the WASM module today, so stage 1 adds a facade report accessor and stage 5 a stat export, or the
  promise is empty. A reader that knows the capability at an older minor reads what it understands; a newer major is
  skipped for that capability only (the decision table of `COMPATIBILITY.md`). A raster whose `tile_codec` it does
  not know is a raster-level skip with its own reason.
* **Reader limits and rules (hostile input).** All checked before the allocation or work they protect. Ceilings are
  `LimitKind` variants with a `Limits` field, a default and a browser value, rendered into `SECURITY_LIMITS.md` as
  the others are (`LimitKind::ALL` grows and the table is re-blessed). Structural refusals are `RASTER-NN` rules of a
  `RasterError` with its own rendered table in `RASTER.md`, the way `DIR-NN` and `COMP-NN` work; they refuse the
  raster, not the file, so a bad image never costs the linework. The browser values are what the browser enforces
  only once `Limits::BROWSER` is wired into `pvec-wasm`, which is a stage 2 dependency and its own issue (it affects
  geometry too). The values after "proposed" are starting points I could not measure, to be fixed in stage 2.

| rule | kind | limit | basis |
|---|---|---|---|
| LIMIT-13 `max_rasters` | ceiling | 4,096 placements per artifact (proposed) | 4,096 index entries of 64 bytes is 256 KiB of directory, about 0.5% of a 50 MB file |
| LIMIT-14 `max_raster_tiles` | ceiling | 1,048,576 tiles per artifact, browser 131,072 (proposed) | the index entry is about 24 bytes, so 24 MiB and 3 MiB of index at the caps, inside `LIMIT-02`; the numbers borrow `LIMIT-06`'s |
| LIMIT-15 `max_tile_bytes` | ceiling | stored bytes of one tile at most `2 * 4 * tile_edge^2 + 65,536` (proposed): 589,824 at 256, 2,162,688 at 512, 8,454,144 at 1024 | an uncompressed RGBA PNG of the tile is `4 * edge^2` plus framing, so twice that plus a header allowance admits every honest encoding and refuses a 64 MiB progressive JPEG whose only purpose is decode time |
| LIMIT-16 `max_plan_decoded_bytes` | ceiling | decoded bytes a plan may hand the host at once, 256 MiB (proposed, the browser `LIMIT-03`): 256 tiles of 512 px, 1,024 of 256, 64 of 1024 | the host decoder is outside the accountant, so the plan is where the bound lives; a placement with a tiny scale puts every tile of the finest level in one viewport |
| RASTER-01 | rule | `tile_edge` in {256, 512, 1024}; `levels >= 1` and `2^(levels-1) * tile_edge >= max(width, height)` with the pyramid stopping at the first one-tile level; `visible_box` inside `width` by `height` | the grid is fixed by the placement |
| RASTER-02 | rule | per level, `cols * rows` computed in checked `u64` equals the index's entries for that level; chunk offsets ascending, non-overlapping and inside the chunk; the sum over levels equals the index `count` | a placement that lies about size cannot cause an allocation, it is refused |
| RASTER-03 | rule | the encoded header is read by a bounded sniffer and must say `tile_edge` by `tile_edge`: PNG `IHDR` with bit depth equal to the placement's (1 or 8), colour type 0, 2, 4 or 6 matching the channels, no interlace; JPEG SOF0, SOF1 or SOF2 reached within a 64 KiB segment walk, `Nf` 1 for gray and 3 for RGB, nothing else; WebP `VP8`, `VP8L` or `VP8X` dimensions, the `ANIM` flag refused, no channel check (the header has none). The stream ends at the tile's last byte (`IEND` last, `EOI` last, RIFF size equal to the length): no trailing payload | the decoded size is fixed by the placement, not by the stream, and a tile that is also something else is refused before a protocol could ever serve it as an image |
| RASTER-04 | rule | `(tile_codec, pixel_format)` pairs: JPEG only with gray or RGB; an unknown codec id skips the raster | JPEG has no alpha; a baked clip or a transparent colour needs PNG or WebP |
| RASTER-05 | rule | `pixel_to_source` passes NP-19 with the 2^20 bound, the footprint corners pass NP-02, `-0.0` is normalised | the inverse map of a viewport must land in a finite (col,row) range |
| RASTER-06 | rule | consistency from the directory and the stream alone: one placement per `RasterId` named by any kind 9 or 10 block, one kind 9 block per placement, `subtype` levels below `levels`, a kind 9 or 10 entry with a capability other than 21 is refused | the same pattern as CONSIST-04 |
| RASTER-07 | rule | `attribution` plain text as defined above; `source_digest` 32 bytes | the one place option C's surface could come back through the front door |

`LIMIT-01` (expansion ratio 64, browser 32) is about the block codec and does not apply to pixels: a blank 512
by 512 RGBA tile is 1,048,576 decoded bytes from about a kilobyte stored, so a ratio would refuse honest blank tiles.
The bound on pixels is the exact size the placement fixes (RASTER-03), the stored cap (LIMIT-15) and the plan cap
(LIMIT-16). The encoded bytes PVec does handle go through `read_block`'s accounting against `LIMIT-03` and `LIMIT-04`
like any block; a host that decodes natively is outside the accountant, like the browser, and gets the same three
bounds.

## Effect elsewhere

**Geometry oracle.** Raster stays outside it. There is no entity, no capability among 1 to 13, nothing in the
equivalence layer (`pvec-harness`) or in `NUMERIC_POLICY.md` changes, and no tolerance is added. The independence is
checked as ADR 0014 and 0015 check it (`to_bits` of evaluation and bounds, the semantic class pattern): decode and
evaluate the geometry of a fixture with the raster stream and blocks, with them removed, and with unknown kind 9
and 10 blocks skipped by an old reader, and require bit-identical results; the first two live in `pvec-components`'s
tests, the third needs negotiation and lives in `pvec-container`'s. How fidelity is judged instead, with no numeric
tolerance of mine: (1) the container is exact for what it stores, so a tile's bytes after a write and read are
equal to the bytes written (compared by digest), and the six placement coefficients are equal as bits after NP-03;
(2) for a lossless codec, the decoded pixels of a stored tile equal the decoded pixels of the producer's tile,
checked on the producer's side (`libviprs-tests`), and PVec makes no claim about the source raster beyond that;
(3) a lossy codec is the producer's choice and PVec claims only that it keeps the bytes; (4) display correctness (the
quad lands where the placement says, under the strokes) is a rendered snapshot in maplibre-pvec, against the
snapshot threshold that repository already owns (`docs/deferred.md` mentions its 0.1 percent), not one defined here.
Raster fixtures are container tests (digest equality), not harness fixtures: `KNOWN_TRUTH_EXACT` supports source
intent, and byte preservation is not one of the corpus claims.

**Rust surface.** No new crate: the placement stream is `raster.rs` in `pvec-components` beside `semantic_class`
(its allow-list is already `pvec-capabilities` and `pvec-entity`, so it has no image or geometry dependency and
validates the affine through `pvec-numeric::Policy::validate_affine`), kinds 9 and 10 and the index parser are in
`pvec-container`, the viewport plan is in `pvec-stream`, and the facade additions land in `pvec::unstable` first
(`API.md` snapshot-checks the frozen-intent list). Under the no-wrappers rule: `RasterId` stays because `new` enforces
LIMIT-13 and it converts to the cell (invariant and conversion); the placement struct is a grouping; `TileCodec`,
`PixelFormat` and `Band` are wire tags with `word()`/`from_word()` like `ElementClass`; `Effect::Unrastered` is a
contract. `BlockKind` does not grow (the kinds are capability-owned); `LimitKind` and `Effect` do, and they are
exhaustive public enums, so `kind_name` in `pvec-conformance`, `effect_of`, `LimitKind::ALL` and seven `api.txt`
snapshots move on purpose. Nothing needs `unsafe`.

**WASM ABI.** Additive exports on the same pattern as the Tier 0 and directory calls: raster stats and placement
buffers by number, a plan call that takes a viewport and returns the tile list with block ids and byte offsets as a
prefetch hint for the host's range cache, and a tile call that returns the encoded bytes of one tile after the index
checksum as a buffer handle the host releases. The tile call follows the -7 protocol of #205 on a range-opened file
(the module names the ranges it needs, there is no second mechanism), goes through `read_block` and never
`read_stored` (which is unchecked), and the bytes exist in the range cache, the reader's decoded cache and the handle
until the host releases the handle and the range cache drops the tile's range, so stage 5 adds a range-release export
or states the cache policy. New: element type `U8` (3) for byte buffers, and negative codes for "no raster stream",
"tile not stored" and "raster skipped by RASTER-NN"; -3 and -4 cover a bad id and a failed checksum. **The version
stays 2.** `WASM_ABI.md` says adding an export does not change it, #205 added five exports without a bump and told
consumers to probe, `MIGRATION.md` puts a new `wasm_abi_version` under "requires v2", and maplibre-pvec hard-codes
`SUPPORTED_ABI_VERSIONS = [1, 2]`, so a bump would be a coordinated two-repository release for an additive change. A
consumer that needs raster probes for the exports.

**maplibre-pvec.** The custom layer (ADR 0001 there) draws each visible tile as a textured quad through the same
matrix chain and hi/lo position split as the strokes, so a rotated, sheared or flipped placement costs nothing extra
and lines up with the linework. The quad corners go through the full chain and the interior is bilinear, which is
exact when the chain is affine across a tile; Mercator curvature across one tile of a sheet hundreds of metres wide
is far below a pixel, and a `projective` calibration is out of scope for raster in v1. Draw order: `band` 0 rasters,
then hatch fills and strokes, then `band` 1; ordering inside a band is by `RasterId`. There is no interleaving with
strokes in v1, because the stroke pass is one instanced draw. Decode runs in the worker pool with
`createImageBitmap` (ADR 0005 there keeps main-thread work under 1 ms a frame, and a single 512 px JPEG decode
exceeds that), the `ImageBitmap` is transferred, textures are entries of the existing `gpu` tier of `TieredCache`
(128 MiB default) and uploads go through the existing budget (4 MiB and 2 ms a frame, `config/defaults.ts`; a 1080p
view of 20 tiles at 512 px is five frames of uploads). Filtering is linear with `CLAMP_TO_EDGE` and no mipmaps (the
pyramid is the mip chain); the one-texel seam of edge-to-edge quads is accepted in v1 and recorded. Beyond full
resolution the level clamps and the tile is magnified. The texture bound is stated as a formula now and its constant
is stage 7: tiles in view per raster per level is at most `(ceil(W/edge)+1) * (ceil(H/edge)+1)` (20 at 1080p and
512 px, 54 at 4K), times the rasters drawn, times two for a retained coarser level, and a raster whose in-view tiles
do not fit the budget is drawn from a coarser level. The alternative, handing tiles to MapLibre's own raster source
through a `pvec-raster://` protocol, is only possible for a placement that is axis aligned and on MapLibre's grid in
the drawing's synthetic world, which no libviprs pyramid is today (Context), so I keep it as a later adapter and not
the v1 path; such an adapter serves bytes only from tiles RASTER-03 accepted, with an image content type.

**Producers.** The v1 producers are a PDF with embedded images and a standalone raster through libviprs; a DWG
`IMAGE` has no pixels any reader here will fetch, so it is reported with its frame (the acadrust `Unmapped` record)
and no placement is written, as is an image with `SHOW_IMAGE` clear or `is_loaded` false. The producer plans with
centring off and overlap 0, drops Google-layout tiles beyond `cols` by `rows`, writes blank tiles or omits them
(`skip_blanks` maps to a zero-length index entry; `BlankTileStrategy::Placeholder` is not a tile and is refused by
RASTER-03), bakes clip boundaries (rectangular, polygonal and inverted; the DXF boundary is in pixel-centre
coordinates with a row flip, half a pixel off the placement origin) into alpha and sets `visible_box`, bakes
brightness, contrast and the transparent colour of a bitonal image, and maps `fade` to `opacity`. For PDF the bake
rules are `/SMask` into alpha, `/ImageMask` into RGBA in the fill colour, `/Indexed`, `/ICCBased`, `/Lab` and
`/CalRGB` into sRGB (lossy, which answers the colour question of my first draft better than a gray gamma would),
`/Decode` arrays applied, and the unit-square image space through the CTM and the page flip. Stage 6 starts by
counting image XObjects and inline images with their CTMs in the conversion report, beside the fill constructs.

**Licensing and size.** PVec stores the bytes it was given and takes no position on who owns the imagery; the
optional `attribution` string and `source_digest` exist so a producer can record it, and PMTiles `attribution` maps
to it as plain text. A raster dwarfs vectors: the measured 65.8 MB for a million entities (ADR 0003) is small next to
a few scans. `LIMIT-05` (artifact bytes, browser 4 GiB) becomes the practical ceiling for rasters. The directory
share is no longer the thing to watch: at two to a dozen entries per raster it stays where ADR 0008 measured it, and
the per-raster index (about 24 bytes a tile, read lazily) is where the tile count goes.

**Interchange.** Export to PMTiles is a passthrough (tile bytes unchanged, tile type from the codec, `min_zoom` and
`max_zoom` from the Xyz numbering, `x = col`, `y = row`, header bounds the whole world square as the vector export
already decides in `INTERCHANGE.md` chapter 6, the placement written under a `vnd.pvec` metadata key beside the
existing `vnd.libviprs` and designed next to the vector tiler's `viprs_cad` key), which is `Exact` in the sense of
`FIDELITY_TAXONOMY.md` for bytes; the archive is a byte round-trip and is not georeferenced unless the caller sets
bounds, exactly like a libviprs archive today. A re-encode is not offered, since the taxonomy's
`BoundedApproximation` carries a bound in drawing units that pixels do not have. The MVT export, the GeoPackage
feature tables and the Part 21 writer do not carry a raster: the last reports an `Unsupported` record with a reason
and no geometry is affected. PMTiles to PVec import is the stage 6 path. GeoPackage tile tables stay out of scope
(`INTERCHANGE.md` chapter 4).

## Consequences

* The container gains two capability-owned block kinds, one registry class and one registry entry. What exists and
  moves, said plainly: `negotiate` learns capability-owned kinds, the `min_reader` hoist narrows to required
  capabilities, chapter 5.4 item 1 and ADR 0010's class list are amended, `FORMAT_VERSION` becomes 1.1.0,
  `FREEZE-v1.md`'s registry sha and extension-rules row are updated on purpose, `LimitKind` and `Effect` grow, the
  compat matrix's kind 9 "unknown kind" fixture is renumbered, `CONFORMANCE.md`'s forward-compatible gate gets a
  fixture with the new blocks, `docs/README.md` links `RASTER.md`, and the golden file bless for a file without a
  raster stays byte-identical (a test, since a canonical form must not move for files that do not use the feature).
* No PVec crate depends on an image codec. The cost is that PVec cannot tell a tile that decodes badly from one that
  decodes fine beyond the header check; a tile with a valid header and a corrupt body is the host decoder's failure
  and is shown as a missing tile.
* Files with a raster are larger; the cold open reads a few directory entries more per raster and the index only
  when a raster is drawn. Both are arithmetic here and measured in stage 4.
* A reader that keeps a texture cache has a new memory consumer outside the accountant's reach, bounded by LIMIT-16
  at the plan and by the formula above at the renderer, and I would rather say that now than find it in a profile
  later...

## Risks

* The per-tile cost in the browser is decode and upload, not hashing: XXH3-64 runs at 12,856 MB/s in wasm (ADR 0006),
  so a 1080p viewport of 20 tiles at 50 to 100 KB is 80 to 160 microseconds of hashing, while each tile is a
  millisecond-scale `createImageBitmap` and a 1 MiB texture upload against a 4 MiB per frame budget. Stage 7 measures
  decode milliseconds per tile per codec and uploads per frame; ADR 0006's reversal threshold (verification opt-in
  per file) is not the lever here.
* The index is a second parser over hostile bytes, and the sniffers are the first image-header parsers in the
  workspace (libviprs identifies containers by magic, not dimensions). The fuzz targets are named in the plan;
  without them stage 2 does not close.
* A raster fixture class has to exist in `pvec-gold-fixtures` with an evidence class that is honest (byte
  preservation as a container claim, not a harness truth).
* The georeference gap in libviprs (it prints bounds and does not store the transform) means a producer that wants
  georeferenced rasters needs the source mapping work in stage 6 before it has anything to write, and that work
  depends on a drawing calibration that is itself ingestion work (`libviprs/libviprs#1039`).
* Baking clips into alpha forces PNG or WebP on a clipped aerial that was JPEG; a polygon clip in the placement
  (open question 3) is the way out if that cost shows up.

## Open questions

1. Scan-only sheets: WRT-01 refuses a file with no entity. Should v1 allow an artifact of placements only, and if so
   may it require capability 21? I lean towards no for v1 and this needs Rom's decision, since it changes the use case.
2. Where does the CRS identifier live (placement, manifest or sidecar) so the out-of-band georeference of a
   geo-referenced raster can be rebuilt reproducibly?
3. A polygon clip in the placement with a stencil pass, as a v1.x addition, if baking into alpha proves too costly.
4. Is the Hilbert order inside a level worth the writer complexity on real scans (stage 4 measures requests per
   viewport against row-major)?
5. The constants: the texture budget, the per-plan cap and the Tier 0 cap, after measurement.

## Reversal threshold

Move to B (an embedded PMTiles block) if on a real scan from the corpus the per-raster index of A' costs more than
the PMTiles directory of the same tiles by a factor above 4 (24 bytes against 4 to 7 per entry is the arithmetic
ceiling; dedupe and run-length could widen it) **and** the range planner can be taught to see inside a PMTiles
directory, which removes B's main objection. Move the index back into the block directory (option A) only if
LAYOUT-04 and the arrival-order tie-break are changed for other reasons and a measured cold open on a ten-scan sheet
stays under the P8.5 `directory-300ms` line, which the arithmetic above says it does not. Drop the optional-only rule
and allow a required `raster` if a producer needs scan-only sheets to refuse in old readers and Rom accepts the cost
ADR 0008 names.

## Staged plan

Follow-up issues to open after ratification (I have not opened any of them):

1. **Spec and registry.** `docs/RASTER.md` (placement and index byte layout, the coefficient conversions and test
   vector, the `RASTER-NN` table), capability 21 under the new `Raster` class, kinds 9 and 10 reserved, the
   `negotiate` change for capability-owned kinds, the `min_reader` hoist change, the `Unrastered` effect and its
   facade report accessor, `FORMAT_VERSION` 1.1.0, the dated amendments to ADR 0010, ADR 0017, CONTAINER.md 4.3 and
   5.4, `INTERCHANGE.md`, FREEZE-v1.md, MIGRATION.md, the compat matrix rows and renumbering, the README link.
   Red tests first.
2. **Placement and index parsers.** `raster.rs` in `pvec-components`, the index parser and sniffers in
   `pvec-container`, LIMIT-13 to 16 in `Limits` and `SECURITY_LIMITS.md`, `RasterError`, wiring `Limits::BROWSER` into
   `pvec-wasm` (own issue), and the fuzz targets: `fuzz_raster_headers` (input `[codec][edge][format][bytes]`, seeded
   with one tile of each codec and variant libviprs writes), `fuzz_raster_index`, kind 9 and 10 seeds for
   `fuzz_directory_traversal`, the plan arithmetic; `TARGETS` grows to nine and `FUZZING.md` and `tools/fuzz.sh` follow.
3. **Container.** Writer and reader for kinds 9 and 10, the chunking, the Hilbert order, the inline Tier 0 part, the
   per-tile checksum path through `read_block`, the unchanged golden file for a file without a raster, a gold fixture
   class, a test-only tiler so stage 4 has a scan to measure.
4. **Streaming and measurement.** Raster plan in `pvec-stream`, the tier-aware plan with the Tier 0 cap, and a MARS
   benchmark of cold open, index bytes, requests per viewport (Hilbert against row-major) and the P8.5 verdicts with a
   raster fixture.
5. **WASM exports.** The exports at ABI version 2, the `U8` element type, the error codes, the stat export, the
   range-release export or cache policy, `WASM_ABI.md`, the release staging.
6. **libviprs source mapping.** D' first (the `GeoTransform` into PMTiles metadata), then a pyramid to PVec path with
   tiles passed through, the PDF image count and bake rules, the georeference composition with its precondition, and
   the dependency on `libviprs#1039`.
7. **maplibre-pvec.** Textured quads in the custom layer, worker decode, the `gpu` tier and upload budget, the level
   rule, the bands and `opacity`, the texture constant, snapshot tests including an asymmetric clipped fixture.
8. **Interchange.** PMTiles passthrough both ways with the `vnd.pvec` key, the `Unsupported` record for Part 21, the
   chapter in `INTERCHANGE.md`.

## Review

A 7-role expert review (API design, performance, GIS, CAD, Rust, software architecture, security) read the first
draft against the code and the other ADRs on 2026-10-30. What changed: the decision moved from one block per tile
(A) to a per-raster index block plus packed tile blocks (A'), because LAYOUT-04 refuses the tile layout, the
directory is fetched whole at open and a single A0 scan already crossed my own reversal line; the block kinds became
capability-owned (DIR-02 refuses the kind I had), the stream got its own registry class (it is not a component by
any written definition), the `min_reader` hoist was found to refuse the file in old readers, the WASM ABI stays at 2,
the limits were split into ceilings and rules with a stored-byte and a decoded-byte cap, the sniffer got a per-codec
contract, the coefficient order became PVec's own, the placement gained `visible_box`, `opacity` and a bit depth,
and the question of how libviprs places a pyramid in MapLibre today was settled from the code (Context). Dissents
that remain: the tile edge range (256 to 512 only, against up to 2048; I chose 256 to 1024), clip boundaries baked
into alpha rather than carried as a polygon, a byte-capped Tier 0 instead of a screen-target level rule, and
whether D' should land before A' is decided (I decided A' and recommend D' in parallel).
