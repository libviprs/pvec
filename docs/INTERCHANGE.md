# Interchange exports

What PVec writes into other formats, and what each export promises about the geometry it wrote. The
vocabulary every chapter uses is `docs/FIDELITY_TAXONOMY.md`; the code is the `pvec-interchange`
crate.

Chapters 1 to 4 belong to the exports that have not landed yet (P6.2 STEP, P6.3 round-trip
validation, P6.4 GML, P6.5 GeoPackage) and are added by those issues. Chapter 5 is the vector tile
export (P6.6, pvec#23).

## 5. Display-only MVT export

A Mapbox Vector Tile cannot carry a curve: everything in it is quantized integer geometry in tile
space. So this export is display-only by construction. It is a derivative for drawing, never a source
for geometry comparison, and never an oracle.

### What it promises

Every entity that reaches a tile is recorded in that tile's ledger as `BoundedApproximation`, with
its achieved world-space deviation as the error bound, or as `Unsupported`, with a reason. No tile
entity can be recorded as `Exact`, `ExactReparameterized` or `EquivalentCompound`: `MvtLedger::record`
refuses those three at construction, on top of the taxonomy rule that a tessellated target cannot
claim exactness. Unsupported today: an unbounded line (no finite extent), a curve the exact
evaluator refuses, a curve that cannot reach the tolerance inside the subdivision depth limit, and a
feature that collapses below one tile unit.

The recorded bound is the tessellation deviation plus the quantization error. Both are in world
(drawing) units, and the total is always at or below the screen budget converted to world units.

### The pipeline

Five stages, each a public function in `pvec_interchange::mvt`, each testable alone:

1. Tolerance: the screen budget (default 0.25 px of a 512 px tile) at the tile's zoom, converted
   through the source-to-map-to-screen transform (`Projection`) into world units, less the
   quantization error. The source-to-map transform must be a similarity (rotation, mirror, uniform
   scale, shift); a shear or non-uniform scale is refused, because the budget in world units would
   depend on the direction.
2. Tessellate: adaptive subdivision of the exact curve (`pvec-geometry`'s evaluator), from eight equal
   parameter spans. A polyline passes through, a trimmed line is its two ends, a composite is its
   segments joined. Tessellation depends on the zoom and the entity, never on the tile address.
3. Transform: world points into tile units.
4. Clip: Liang-Barsky against the tile plus a buffer (default 64 units), in floating point, before
   rounding. Because stage 2 does not depend on the tile address, neighbouring tiles clip the same
   polyline and the union has no seam.
5. Quantize and encode: round to the integer grid, drop repeated points, write geometry commands.

### Output

* One layer per component grouping (the layer name the caller gives each entity), written in name
  order, features in input order, so the same input gives byte-identical tiles.
* Every feature is a linestring (one or more pieces) with attributes `entity_id` (uint, the file-local
  `EntityId`, for picking in P7.5), `world_deviation` (double, equal to the ledger bound) and
  `display_only` (true). The feature id is the entity id.
* Layer version 2. A tile with nothing in it is zero bytes, which is a valid tile.

### Numbers

* Tile extent: extent 4096, clip buffer 64.
* Screen budget: 0.25 px at 512 px tiles.
* Quantization error: half the diagonal of one tile unit. In screen space that is 0.0884 px at
  extent 4096 and 512 px tiles, at every zoom. In world units it depends on the zoom and the map.
* Tested zoom range 0 to 14; maximum tested zoom 14.
* On the named fixture `crossing-curves` (a drawing 200 units across, scale 2 onto a 1024 unit map,
  rotated 0.4 rad), the quantization error at zoom 14 is bounded by 5.395e-6 world units. The test
  measures 5.274e-6 over 5000 points on the fixture circle, so the bound is tight to about 2%.

### What is measured, and how

* Tessellation deviation is measured against the exact evaluator at 7 interior points of every final
  chord and stored as the bound. The acceptance test cross-checks against 4000 dense exact samples
  per curve at every zoom 0 to 14. Seven samples is a measurement, not a proof of a maximum for an
  arbitrary spline; a curve with features narrower than an eighth of a final chord can hide one.
* Seams: for the fixture, at zoom 2 with buffer 0 and with the default buffer, every point of the
  exact curve lies within the recorded bound of a segment recovered from the union of all tiles.
* Validity: tiles are decoded in the test-suite by a decoder written from the vector tile
  specification that shares no code with the encoder. It checks command ids and counts, parameter
  availability, key and value indices, and that every vertex lies inside the buffered tile.

### Out of scope here

PMTiles packaging (P6.7), MapLibre styling, raster tiles, polygon and fill output, and any use of an
MVT as a source for comparison.
