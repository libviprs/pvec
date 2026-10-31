# ADR 0018: Raster graphics inside a PVec artifact

Status: Proposed, 2026-10-30. Decides #212. Awaiting Rom's ratification. Scopes ADR 0017 and the raster line of
`docs/INTERCHANGE.md` (see "What this scopes"). Builds on ADR 0002 (authored bits), ADR 0005 (the index), ADR 0006
(per-block codec and checksum), ADR 0007 (tiers), ADR 0008 and ADR 0010 (extension rules, the registry), ADR 0013
(governance) and ADR 0014 (component streams).

I ran no benchmark for this ADR. Every number below is a constant or a document I read (file references inline), or
plain arithmetic on them, and each one that I could only estimate is called a proposal. The measurements are the
first stages of the plan.

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

What libviprs already does with rasters (read from `libviprs/libviprs` at `a81c9fbc` and `libviprs-cli`):

* It cuts a raster into a tile pyramid. `PyramidPlanner::new(w, h, tile_size, overlap, layout)` (`src/planner.rs`)
  gives square tiles, 256 by default and 512 for HiDPI (`README.md`, `libviprs-cli/README.md`), levels that halve
  down to 1x1 for DeepZoom and Xyz, and an optional centring in the tile grid. The tile formats are `Png`, `Jpeg`,
  `Webp` and `Raw` (`src/sink.rs`).
* It packs the tiles into one PMTiles v3 archive, now the default (`src/sink_pmtiles.rs`, `src/pmtiles/`). PMTiles
  has no tile format for `Raw` (raw pixels carry neither dimensions nor pixel format, `TileType::try_from_tile_format`),
  addresses tiles as `z/x/y` with row 0 at the top (`COORDINATE_CONVENTION = "zxy"`), and caps the zoom at 31
  (`MAX_ZOOM`). Its reader bounds the root directory, leaf depth, directory and metadata sizes (`src/pmtiles/reader.rs`).
* It georeferences with a six coefficient affine from pixel to geographic space (`src/geo.rs`, `GeoTransform`:
  `geo_x = a*px + b*py + c`, `geo_y = d*px + e*py + f`, pixel origin at the top left). The CLI builds one from
  `--geo-origin` and `--geo-scale` and **prints the bounds** (`libviprs-cli/src/main.rs`, near line 1048). I found no
  place where that transform is written into the archive: the PMTiles header's E7 bounds are a writer option and the
  `vnd.libviprs` metadata carries source size, pixel format, tile size, layout and format, not a transform.
* Its vector tiler places a drawing in a synthetic world (the drawing fitted into the zoom 0 unit square, same scale on both axes,
  y flipped) so that "MapLibre sees an ordinary Web Mercator tileset" (`src/vector/tiler.rs`). The reference viewer
  (`web/vector-viewer/`) loads the archive through the `pmtiles://` protocol into a MapLibre source.
* `docs/adr/pvec-pdf-policy.md` counts 76 non-uniform scales in one real PDF set and says they are image
  placements. I found no policy for what happens to those images when a PDF is mapped to PVec.

What I could **not** find is the code that hands a raster pyramid to MapLibre as a raster source with a georeference
(see the open questions)... I am not assuming it exists.

On the PVec side, the pieces a raster has to live with:

* Everything that is an entity has an `EntityId` and resolves to a `CurveKind` (`crates/pvec-entity`, `EntityTable`),
  ids are dense `0..n` (`docs/ENTITY_ID.md`).
* Components are optional streams that refer to entities by `EntityId`, embed no geometry and are never required
  (ADR 0014, `NEGOTIATE-04`). The set is one block read whole.
* A block has a 64 byte directory entry with kind, subtype, cell, bounds, codec and capability; the directory is
  fetched whole at open (`LIMIT-06`); block kinds 1 to 8 exist and a reader skips an unknown optional kind
  (`docs/CONTAINER.md` chapter 4, ADR 0008).
* Each block has an XXH3-64 over its stored bytes, verified before it is decompressed (ADR 0006), and a block is
  limited to `max_block_bytes` (`LIMIT-02`: 256 MiB, browser 64 MiB).
* The numeric policy says coordinates are authored bits and one place defines tolerances (ADR 0002). `libviprs`
  has its own singularity test with a relative `1e-12` in `src/geo.rs`; PVec does not adopt that, it uses ADR 0012's rule.

## Options

| option | what it is | for | against |
|---|---|---|---|
| A. A raster stream plus tile blocks in the container | a `raster` component holding the placements, and a new optional block kind, one block per tile, each with the tile's encoded bytes (PNG, JPEG or WebP) | per-tile range fetch through the directory the reader already has; per-tile XXH3 checked before use; the artifact SHA-256 covers every tile; tile bounds are in the 64 byte entry; one file; skipping is already defined for an unknown optional kind | the directory gains 64 bytes a tile (a 20000 x 14000 px scan at 512 px tiles is about 1,500 tiles over all levels, about 96 KB; at 256 px about 6,000, about 384 KB; arithmetic, not measured); files get much larger |
| B. An embedded PMTiles archive as one block | the whole archive from `PmTilesSink` stored as an opaque block | byte-identical with what libviprs writes and `viprs pmtiles extract` reads; the `pmtiles` JS reader works on an offset-shifted range source | the block checksum is over the whole block and ADR 0006 verifies before use, so either a multi-hundred MiB block is read to check one tile, or the rule is waived for this kind; `LIMIT-02` (64 MiB in the browser) caps the image; a second directory system with its own gzip, which has no uncompressed length (`Compression::decompress` makes the caller name a ceiling), sits inside the first and PVec's range planner cannot merge its ranges with vector ranges; hostile-input bounds have to be reimplemented in two parsers (Rust and the browser's) |
| C. An external reference (URL or path plus a digest) | the placement and georeference live in the PVec, the pixels stay in a PMTiles or tile tree | file stays small; PMTiles hosting and CDN caching are reused as they are | a second object to host, with CORS and range support; the artifact digest no longer covers the pixels (a digest of a whole archive cannot be checked by a ranged reader); a file that makes the viewer fetch a URL it chooses is a privacy and request-forgery surface that no PVec reader has today; no offline use |
| D. Status quo | raster stays a MapLibre source beside the PVec layer | works now: ADR 0001 of maplibre-pvec adds the custom layer with `beforeId` among style layers, so draw order against a raster layer is already controllable | two objects, two caches, two failure modes; the georeference is wired by hand twice; a converted PDF or DWG that had images cannot carry them in its own artifact |

## Decision

**A**, with three changes to how I phrased it in the brief.

1. **The placement is not an entity.** `EntityTable` resolves ids to a `CurveKind`, so giving an image an
   `EntityId` would put a non-curve into the type the geometry oracle iterates. The stream is keyed by its own
   dense `RasterId` and the placement refers to nothing in the entity space. Layer visibility is a `layer_index`
   into the layer stream's name table (a table row, not an entity), ignored if that stream is unknown.
2. **Two things, not one.** The placements (small, read whole at open) are a component stream, capability **21**
   `raster`, class component, **optional**, no ISO construct, next dense id after `semantic_class` (20). The pixels
   are a new block kind **9** `raster_tile`, optional and skip-safe (must-understand bit clear), one tile per block,
   capability id 21 in the entry so a reader that does not know it can name what it skipped (ADR 0008, chapter 5
   of `CONTAINER.md`). A component stream cannot hold the tiles, because the set is read whole and a viewer needs
   one tile.
3. **PVec stores the encoded bytes and never encodes or decodes pixels.** The producer (libviprs) hands over tiles
   in the codecs it already writes, and the reader returns them untouched after the checksum. The browser decodes with its
   own decoder (`createImageBitmap`), which is sandboxed and already there. This keeps image codecs out of the
   PVec crates and out of the WASM module (ADR 0004 prices the module at 29 KB), and means PVec has no canonical
   encoding to pin: the bytes are authored, like coordinates.

PMTiles stays the way to ship a **standalone** raster pyramid, and D stays valid for a viewer that wants only that.
A is for a drawing that owns its raster. C is not taken now; it fits later as a placement whose tiles are external,
but only after a host policy for fetching exists, which is not this ADR's to write.

### What this scopes

ADR 0017's rule stands for **geometry**: no instance, no transform, no block table, every entity a curve. This ADR
adds a second kind of content that is not geometry and is not an entity, in the same way ADR 0014 added style and
text. Where 0017 or `INTERCHANGE.md` says "curve-only" it now means "the geometry is curve-only". On acceptance I
change those sentences (stage 1); I do not edit them in this PR, because 0017 is itself still awaiting ratification.
The MVT, GeoPackage and Part 21 exports stay raster-free (see "Interchange" below)...

### Data layout

All integers and floats are little endian, as in the other streams. The byte layout goes in a new
`docs/RASTER.md` in stage 1; what is decided here is what it holds.

**Placement** (one per `RasterId`, in the stream):

| field | decision |
|---|---|
| `pixel_to_source` | six `f64` stored as authored bits, `[a b c d e f]` with `x = a*px + b*py + c`, `y = d*px + e*py + f`, pixel origin at the top left of pixel (0,0), y down: libviprs's `GeoTransform` layout, so the six numbers pass through. The target space is PVec **source space**, the same space as the vectors, so alignment with the linework holds by construction |
| validity | each coefficient passes NP-01 and NP-02, and the map is invertible under the rule ADR 0012 already applies to `affine_transform_2d` (capability 13). A reflection or a non-uniform scale is legal (the PDF image placements are non-uniform); a singular map is refused, with PVec's rule and not libviprs's `1e-12` |
| `width`, `height` | pixels of the full resolution level, at least 1 |
| `tile_edge` | 256 or 512 in v1 (the two values libviprs documents and the tiler uses). Tiles are full squares; an edge tile is padded by the producer, and the renderer clips to `width` by `height` so a padded margin never draws |
| pyramid | `levels` (at most 31, `MAX_ZOOM`), level `levels-1` is full resolution, each level above halves with a ceiling, the pyramid **stops at the first one-tile level**, no overlap. `first_level` records libviprs's level number of the coarsest stored level so a PMTiles export keeps its `z` |
| `tile_codec` | 1 PNG, 2 JPEG, 3 WebP (lossless or lossy), the values libviprs writes and PMTiles types 2, 3 and 4 name. `Raw` is not allowed (no dimensions). AVIF and MLT are not v1: libviprs has no encoder for them. An unknown codec id skips that raster and reports it |
| `pixel_format` | 8 bit gray, gray with alpha, RGB, RGBA. Colour is treated as sRGB display colour, as MapLibre raster sources do; no ICC profile in v1 |
| `band` | 0 underlay (below every stroke), 1 overlay (above). Nothing finer in v1 (see maplibre-pvec below) |
| `layer_index` | optional, see the decision above |
| `source_digest`, `attribution` | optional SHA-256 of the source raster for provenance, and an attribution string of at most 255 bytes (the limit the layer name uses in `COMPONENT_STREAMS.md`). Neither is read by the renderer |
| tiers | `tier0_level`: the level whose tiles are the progressive tier, by default the one-tile overview |

**Georeference.** The placement stores no CRS and no geographic transform. maplibre-pvec takes the georeference of
the whole drawing as a constructor input and nothing reads one out of a `.pvec` (`docs/georeferencing.md`, a test
keeps it so), and a raster in the same source space rides that single georeference. A producer that starts from a
geo-referenced raster composes `pixel -> geo` with the inverse of the drawing's `source -> geo` and writes the
result; that composition is source mapping, EPIC N's job. This is deliberately **not** libviprs's pixel to geo
shape: two georeferences in one file could disagree. Whether a declared CRS tag is worth carrying as metadata is an
open question.

**Tile blocks.** One block per tile, holding exactly the encoded bytes. Directory entry: kind 9, `cell` = the
`RasterId` offset into a reserved range (as the straddler pages use `STRADDLER_CELL_FIRST`; the number is fixed in
`RASTER.md`), `subtype` = level, bounds = the axis aligned box in source space of the tile's footprint (the
affine image of its pixel rectangle clipped to `width` by `height`), bounds-meaningful flag set, codec id 0 (stored:
PNG, JPEG and WebP are already compressed, and the writer's "only when smaller" rule keeps it at 0 anyway), capability 21.
Within a (raster, level) the tiles are in row-major order, so the canonical order of `CONTAINER.md` chapter 4 (kind,
cell, subtype) places the coarse levels of one raster next to each other and tile (col,row) is the
`row * cols + col`-th entry of its level: a lookup is arithmetic on the placement plus one directory read, with no
search. A fully blank tile is still stored; sparse pyramids are an open question.

### Spatial index and range fetch

Tiles are **not** put in the Hilbert R-tree of ADR 0005. The tree is over entity centres and every payload is
owned by one leaf; a placement's bounds usually cover most of the drawing, which is the "long entity" case that
ADR 0005 measured congesting leaves (0.958 of the world on `straddler`), and a tile is addressed by arithmetic,
not by search. The viewport plan for a raster is: choose the level from the screen scale, map the viewport through
the inverse of `pixel_to_source` to a (col,row) range, read those directory entries, and hand the ranges to the
same merge planner (the 64 KiB gap of ADR 0005) together with the vector ranges. The raster footprint is added to the
extent a reader reports for fit-to-view; the spatial directory's root box is left as it is, a union of entities.

### Tier assignment

Progressive tiers (ADR 0007) are about a preview that is never labelled exact. A pyramid is that already: every
level above full resolution is a derived display image. `tier0_level` (the one-tile overview by default) is fetched
with the first paint, in the same tier-aware plan as Tier 0 geometry (`ViewportQuery::plan_tiered`, extended in
stage 4), and the overview of every raster sits in one contiguous run by the canonical order. Nothing else about
tier 0 changes, and no overview is described as exact anywhere.

### Compression and checksum

Per tile, as in ADR 0006: XXH3-64 over the stored bytes, verified before the tile is handed on, and the artifact
SHA-256 of the whole file covers every tile with no new rule. The container codec is 0. The image codec lives in
the placement, not in the directory's codec id, so `CODEC-01` to `CODEC-03` and `LIMIT-01` keep meaning what they mean.

## Extension rules

* **Governance (ADR 0013).** Not a geometry extension: the gate in `Registry::register_extension` and the Part 42
  proposal are for geometry, and `docs/EXTENSIONS.md` says style, text and hatch are not geometry extensions but
  component streams. 0013 requires no panel review for a component; it does require that a required capability
  list an ADR. Registering 21 is a new component entry in the registry data, as 14 to 20 were (it does not go through
  `register_extension`), and its restriction text points at this ADR. The process that applies is this ADR's status: Proposed until Rom ratifies.
* **Optional only.** Capability 21 is optional and block kind 9 clears must-understand. A file cannot declare it
  required, because a component is never required (`NEGOTIATE-04`).
* **Old readers.** One that knows neither opens the file `OpenDegraded`: the capability is listed under
  `NotImplemented`, kind 9 blocks are listed as `UnknownBlockKind` skips by their directory entries, the payload is never fetched,
  and the geometry is bit-identical. I add an `Effect` value (`Unrastered`) beside `Unstyled` and `Uncomponented`
  so a viewer can say "this file has an image your reader cannot show". A reader that knows the capability at an older minor reads what it
  understands; a newer major is skipped for that capability only (the decision table of `COMPATIBILITY.md`).
* **The honest gap.** A scan-only sheet opened by an old reader shows nothing and says why in the report; it is not refused.
  Whether a file with no geometry should be allowed to require 21 (ADR 0013 allows a required entry that lists an ADR) is
  an open question, and I lean towards no for v1.
* **Reader limits (hostile input).** All checked before the allocation or work they protect, each a distinct
  `LimitExceeded`. The values after "proposed" are starting points I could not measure, to be fixed in stage 2.

| rule (proposed names) | limit | basis |
|---|---|---|
| RASTER-01 | tile edge is 256 or 512; levels at most 31; no overlap | libviprs defaults; `MAX_ZOOM` |
| RASTER-02 | tiles per level and per image computed from `width`, `height`, `tile_edge` in checked `u64`; the count of tile blocks must match it | a placement that lies about size cannot cause an allocation, it is refused |
| RASTER-03 | tiles per artifact are bounded by `max_blocks` (`LIMIT-06`: 1,048,576, browser 131,072) | reuses the directory limit, so no new number |
| RASTER-04 | one tile's stored bytes at most `max_block_bytes` (`LIMIT-02`) | reused |
| RASTER-05 | the encoded header is read (PNG `IHDR`, JPEG start-of-frame, WebP `VP8`/`VP8L`/`VP8X`, bounded hand-written sniffers) and its width, height and channel count must equal `tile_edge` and the format; checked before the bytes leave the reader | the decoded size is fixed by the placement, not by the stream |
| RASTER-06 | placements per artifact at most 4,096 (proposed; the component set already caps its frames at 4,096) | |
| RASTER-07 | a native decoder counts decoded bytes against `LIMIT-03` and `LIMIT-04` before decoding | the browser's decoder is outside the accountant, which is why RASTER-05 exists |
| RASTER-08 | the six coefficients of `pixel_to_source` obey NP-01, NP-02 and ADR 0012's invertibility | |

`LIMIT-01` (expansion ratio 64, browser 32) is about the block codec and does not apply to pixels: a blank 512
by 512 RGBA tile is 1,048,576 decoded bytes from a few hundred stored ones, so a ratio would refuse honest blank tiles.
The bound on pixels is the exact size the placement fixes (RASTER-05), not a ratio.

## Effect elsewhere

**Geometry oracle.** Raster stays outside it. There is no entity, no capability among 1 to 13, nothing in the
equivalence layer (`pvec-harness`) or in `NUMERIC_POLICY.md` changes, and no tolerance is added. The independence is
checked as ADR 0014 checks it: decode and evaluate the geometry of a fixture with the raster stream and blocks, with
them removed, and with unknown kind 9 blocks skipped by an old reader, and require bit-identical results. How fidelity is judged
instead, with no numeric tolerance of mine: (1) the container is exact for what it stores, so a tile's bytes after a
write and read are equal to the bytes written (compared by digest), and the six placement coefficients are equal
as bits; (2) for a lossless codec, the decoded pixels of a stored tile equal the decoded pixels of the producer's
tile, checked on the producer's side (`libviprs-tests`), and PVec makes no claim about the source raster beyond that;
(3) a lossy codec is the producer's choice and PVec claims only that it keeps the bytes; (4) display correctness (the
quad lands where the placement says, under the strokes) is a rendered snapshot in maplibre-pvec, against the
snapshot threshold that repository already owns (`docs/deferred.md` mentions its 0.1 percent), not one defined here.

**WASM ABI.** Additive exports on the same pattern as the Tier 0 and directory calls: raster stats and placement
buffers by number, a plan call that takes a viewport and returns the tile list and byte ranges for the host's
range cache (this works with `open_ranged`, #205), and a tile call that returns the encoded bytes of one tile after
the checksum as a buffer handle that the host releases, so a frame does not hold tile bytes in linear memory longer than needed.
Nothing existing is removed or re-signed. I follow the precedent of version 2 (#201) and bump `wasm_abi_version`
to 3 on purpose, so a consumer that needs raster can refuse a version 2 module; a version 2 consumer keeps working
against a version 3 module.

**maplibre-pvec.** The custom layer (ADR 0001 there) draws each visible tile as a textured quad through the same
matrix chain and hi/lo position split as the strokes, so a rotated, sheared or flipped placement costs nothing extra
and lines up with the linework. Draw order: `band` 0 rasters, then strokes, then `band` 1; ordering inside a band is by
`RasterId`. There is no interleaving with strokes in v1, because the stroke pass is one instanced draw. Memory: a
decoded 512 by 512 RGBA tile is 1,048,576 bytes (262,144 at 256 by 256), so the texture cache has a byte budget
with LRU eviction and a count of in-flight decodes. The value is a stage 5 decision from a measurement on the reference
machine (`docs/deferred.md` records that no GPU numbers exist yet), not a number I pick here. The alternative, handing tiles to MapLibre's
own raster source through a `pvec-raster://` protocol, is only possible for a placement that is axis aligned and on
MapLibre's Web Mercator grid, which a libviprs raster pyramid is not today (it is in image pixel space, not in the
vector tiler's synthetic world), so I keep it as a later adapter and not the v1 path.

**Licensing and size.** PVec stores the bytes it was given and takes no position on who owns the imagery; the
optional `attribution` string and `source_digest` exist so a producer can record it, and PMTiles `attribution` maps to it. A raster dwarfs vectors:
the measured 65.8 MB for a million entities (ADR 0003) is small next to a few scans. `LIMIT-05` (artifact bytes,
browser 4 GiB) becomes the practical ceiling for rasters. The directory share is the thing to watch: ADR 0008
accepts 0.17% of the file and sets 0.5% as its reversal line. At 64 bytes an entry that line is crossed by tiles
smaller than about 12.8 KB, which small JPEG tiles can be.

**Interchange.** Export to PMTiles is a passthrough (tile bytes unchanged, tile type from the codec, `z` from
`first_level`, the placement written under a `vnd.pvec` metadata key beside the existing `vnd.libviprs`, the same
way it ignores keys it does not know), which is `Exact` in the sense of `FIDELITY_TAXONOMY.md` for bytes; a re-encode
is not offered, since the taxonomy's `BoundedApproximation` carries a bound in drawing units that pixels do not have. The MVT export, the GeoPackage
feature tables and the Part 21 writer do not carry a raster: the last reports an `Unsupported` record with a reason and no geometry is
affected. PMTiles to PVec import is the stage 6 path. GeoPackage tile tables stay out of scope (`INTERCHANGE.md`
chapter 4).

## Consequences

* The container gains one block kind and one registry entry and nothing that exists changes. The
  compatibility matrix gets a raster-unaware reader row, and the golden file bless for a file without a raster
  stays byte-identical (a test, since a canonical form must not move for files that do not use the feature).
* No PVec crate depends on an image codec. The cost is that PVec cannot tell a tile that decodes badly from one that
  decodes fine beyond the header check; a tile with a valid header and a corrupt body is the host decoder's failure
  and is shown as a missing tile.
* Files with a raster are larger and the cold open reads more directory. Both are arithmetic here and measured in stage 4.
* A reader that keeps a texture cache has a new memory consumer outside the accountant's reach, and I would rather
  say that now than find it in a profile later...

## Risks

* The XXH3 and SHA-256 passes now run over far more bytes. ADR 0006 measured them cheap against LZ4 and deflate
  decode, but a stored tile has no decode to hide behind, so the checksum is the dominant cost of a tile in wasm.
  The reversal threshold of ADR 0006 (make verification opt-in per file) is the answer if stage 4 shows it.
* A raster fixture class has to exist in `pvec-gold-fixtures` with an evidence class that is honest (`KNOWN_TRUTH_EXACT`
  for byte preservation, not for pixels of a source nobody can state exactly).
* The georeference gap in libviprs (it prints bounds and does not store the transform) means a producer that wants
  georeferenced rasters needs the source mapping work in stage 6 before it has anything to write.

## Open questions

1. How does libviprs hand a raster pyramid to MapLibre today? I found the vector viewer and the PMTiles tile types,
   not a raster source wiring or a re-grid onto the synthetic world. If one exists, the v1 render path may be
   able to reuse it.
2. Sparse pyramids: omit blank tiles (a presence bitmap per level, as `BlankTileStrategy::Placeholder` hints) or
   always store them?
3. Should a CRS identifier be carried as metadata, given that maplibre-pvec owns the single georeference?
4. May a geometry-free file require capability 21?
5. Does the root box of the spatial directory include raster footprints, or only the reported extent?
6. Is a second directory level for tiles needed, and at what tile count? Stage 4 answers it from the 0.5% line.
7. Colour: is sRGB-only enough for scanned plans, or does a gray gamma need to be said?

## Reversal threshold

Move to B (an embedded PMTiles block) if both hold on a real scan from the corpus: the per-block checksum rule is
relaxed so a tile of a large block can be verified on its own (removing B's main objection), and A's tile entries push
the block directory above 0.5% of the file (ADR 0008's own line) so a second directory level would be needed anyway.
Move the tile index out of the block directory into a per-image directory block if the cold open on that scan reads
more than twice the bytes of the same drawing without the raster. Drop the optional-only rule and allow a required
`raster` if a producer needs scan-only sheets to refuse in old readers and Rom accepts the cost ADR 0008 names.

## Staged plan

Follow-up issues to open after ratification (I have not opened any of them):

1. **Spec and registry.** `docs/RASTER.md` byte layout, capability 21, block kind 9, the `Unrastered` effect, the
   compatibility matrix rows, and the one-line scoping edits to ADR 0017 and `INTERCHANGE.md`. Red tests first.
2. **`pvec-raster` crate.** Placement encode and decode, the header sniffers, the RASTER limits in `Limits` and
   `SECURITY_LIMITS.md`, zero dependencies, a fuzz target.
3. **Container.** Writer and reader for kind 9, canonical order, checksums, the unchanged golden file for a file without a raster, a gold
   fixture class.
4. **Streaming and measurement.** Raster plan in `pvec-stream`, the tier-aware plan, and a MARS benchmark of cold open, directory share and the checksum cost in wasm on a real scan.
5. **WASM ABI 3.** The exports, `WASM_ABI.md`, the release staging.
6. **libviprs source mapping.** A pyramid to PVec path with tiles passed through, the PDF image placements, the missing georeference composition.
7. **maplibre-pvec.** Textured quads in the custom layer, the texture budget, the bands, snapshot tests.
8. **Interchange.** PMTiles passthrough both ways, the `Unsupported` record for Part 21, the chapter in `INTERCHANGE.md`.
