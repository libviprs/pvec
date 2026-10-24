# Interchange exports

What PVec writes into other formats, and what each export promises about the geometry it wrote. The
vocabulary every chapter uses is `docs/FIDELITY_TAXONOMY.md`; the code is the `pvec-interchange`
crate.

Chapters 1 to 3 belong to the exports that have not landed yet (P6.2 STEP, P6.3 round-trip
validation, P6.4 GML) and are added by those issues. Chapter 4 is the GeoPackage export (P6.5,
pvec#22). Chapter 5 is the vector tile export (P6.6, pvec#23).

## 4. GeoPackage export

A GeoPackage is a SQLite file that GIS desktops open without argument. Its geometry encoding has
circular arcs (`CIRCULARSTRING`), chains of arcs and lines (`COMPOUNDCURVE`) and polygons bounded by
them (`CURVEPOLYGON`), and it has nothing for an ellipse, a Bezier curve or a spline. So this export
uses the nonlinear types where they say the source exactly, and everywhere else it writes a
`LINESTRING` that stands in for the curve within a tolerance the caller chose, and says how far it
actually is.

`pvec_interchange::gpkg::export` takes the entities (an `EntityId` and a `Curve` each) and a
`GpkgOptions` (the world-space tolerance, in drawing units, and an optional `last_change` stamp). It
returns the file as bytes and a `FidelityLedger` with one record per entity, in the order the entities
were given. The crate does no I/O, and an entity that cannot be translated is in the ledger as
`Unsupported` with its reason and has no row in the file.

### The decision table

Which geometry each curve capability becomes, and which fidelity value (`docs/FIDELITY_TAXONOMY.md`)
the ledger carries for it. The code is `src/gpkg/translate.rs`; the tests in `tests/gpkg.rs` pin every
row.

| Source capability | Written as | Fidelity | When |
|---|---|---|---|
| polyline | `LINESTRING` | `Exact` | always; the vertices are copied bit for bit |
| trimmed line | `LINESTRING` of two points | `ExactReparameterized` | the origin, direction and trim become the two end points |
| circle | `CIRCULARSTRING`, closed, five points | `ExactReparameterized` | whole circle; the last point is the first, exactly |
| trimmed circle (an arc) | `CIRCULARSTRING`, three points | `ExactReparameterized` | the arc spans at most half a turn: start, middle and end, in the direction of travel |
| trimmed circle (an arc) | `CIRCULARSTRING`, five points | `ExactReparameterized` | the arc spans more than half a turn: two arcs of the one circle |
| ellipse, trimmed ellipse | `LINESTRING` | `BoundedApproximation` | always; GeoPackage has no elliptic arc |
| Bezier curve, degree 1 | `LINESTRING` | `EquivalentCompound` | a straight segment, written as its end points |
| Bezier curve, degree 2 or more | `LINESTRING` | `BoundedApproximation` | always |
| B-spline, degree 1 | `LINESTRING` through the knots | `EquivalentCompound` | piecewise straight, so the polyline through the curve at its knots is the same locus |
| B-spline, degree 2 or more | `LINESTRING` | `BoundedApproximation` | always |
| rational B-spline, degree 1 | `LINESTRING` through the knots | `EquivalentCompound` | as for the non-rational spline |
| rational B-spline, every span a circular arc | `CIRCULARSTRING`, one arc per span | `EquivalentCompound` | degree 2, clamped, every interior knot of multiplicity 2, and exact evaluation follows the circle through each span's end points and middle to the numeric policy's tolerance (NP-04) |
| rational B-spline, anything else | `LINESTRING` | `BoundedApproximation` | always; a rational B-spline is never `Exact` or `ExactReparameterized` |
| trimmed Bezier, B-spline or rational B-spline | as the basis, for degree 1; otherwise `LINESTRING` | `EquivalentCompound` for degree 1, otherwise `BoundedApproximation` | a trimmed rational arc spline is approximated, not decomposed |
| composite curve | `COMPOUNDCURVE` of the members' pieces | the weakest member's value | `Exact` only when every member is; the bound is the largest member bound |
| line (unbounded) | none | `Unsupported` | there is no finite geometry to write; trim it first |

A few consequences. A result written with a `LINESTRING` that stands for a curve is `Tessellated` in
the ledger and so can only ever be `BoundedApproximation` (the taxonomy refuses anything stronger when
the record is built). `EquivalentCompound` is claimed only where the locus is the same and the target
is `Native`, and for the rational arcs it is claimed only after the exact evaluator has been compared
with the arcs at 65 points of every span. `CURVEPOLYGON` is not written: the curve model has no area,
and a closed curve stays a curve. When a region entity arrives (a room ring, a hatch boundary) its
mapping to `CURVEPOLYGON` is added here with its own row, and the extension row appears only for a
file that uses it.

An arc keeps its direction. `CIRCULARSTRING` reads its three points in order, so a clockwise arc is
written from where it starts to where it ends and a reader draws the same arc, not its complement.

### Approximation and the measured deviation

The caller's tolerance is the most a written `LINESTRING` may be from the exact curve, in drawing
units. The export evaluates the curve only through `pvec-geometry`'s exact evaluator and subdivides:

1. It starts from the curve's knots and from eight equal parameter spans, so a closed or periodic curve
   is never one chord.
2. For each chord it measures the deviation both ways: the largest distance from the exact curve to the
   chord (24 samples, then the best sample refined in three rounds of eight), and the largest distance
   from chord points back to the curve, against a 64-segment reference polyline of the curve.
3. It multiplies the measure by 1.001, so the recorded value errs high, and accepts the chord if that
   is at or below the tolerance; otherwise it splits the parameter span in two and measures each half.
4. The ledger's `BoundedApproximation` carries the largest accepted value over the curve. It is the
   measured number, never the requested one, and it is always below the request. The same number is in
   the `error_bound` column of the feature.

The bound is measured, not proven: a feature narrower than the sample spacing (a cusp inside one
chord) could be missed. The tests compare it against an independent measurement at 6000 samples,
which has to stay at or below the tolerance and not above the recorded value by more than one
percent.

A tolerance the coordinates cannot resolve is refused with `ToleranceUnreachable` rather than
looped on: below eight times the spacing of the floating point numbers at the curve's magnitude, or
when the subdivision would pass a million vertices or sixty levels. A tolerance that is zero,
negative or not finite is refused when the options are built.

### What is in the file

| Table | What it holds |
|---|---|
| `gpkg_spatial_ref_sys` | the three rows the specification requires: -1 (undefined Cartesian), 0 (undefined geographic) and 4326 |
| `gpkg_contents` | one `features` row for `pvec_features`, its bounds (the union of the feature envelopes) and `last_change` |
| `gpkg_geometry_columns` | `pvec_features.geom`, type `GEOMETRY` (it holds several), XY, SRS -1 |
| `gpkg_extensions` | `gpkg_geom_CIRCULARSTRING` and `gpkg_geom_COMPOUNDCURVE` (read-write) for each of the two the file actually uses, a compound curve holding an arc counting as using both, and `gpkg_rtree_index` (write-only) |
| `pvec_features` | `fid`, `geom`, `entity_id`, `fidelity`, `error_bound`, `source_capability` |
| `rtree_pvec_features_geom` and its `_rowid`, `_node` and `_parent` tables | the spatial index, with the seven triggers GeoPackage 1.4 specifies |

The attribute columns are what lets a desktop user tell an approximation from an exact curve: `entity_id`
is the file-local `EntityId`, `fidelity` is one of the five taxonomy names, `error_bound` is the
measured deviation (empty unless the value is `BoundedApproximation`), and `source_capability` is the
registry id of the curve kind. Features are in the order the entities were given, with `fid` counting
from 1, so a feature's row does not depend on anything but its input.

The features are in the "undefined Cartesian" system because a drawing is in its own units. A
transform to a real CRS, and the reprojection that goes with it, are out of scope.

The geometry is GeoPackageBinary: header (magic `GP`, version 0, little endian, XY envelope, SRS id)
and ISO WKB. The envelope is computed from what was written, and for an arc from the circle through its
three points, so an arc that bulges past its end points still has a box that contains it. The index
boxes are rounded outward when narrowed to 32-bit floats.

### Reproducibility

The SQLite file is written by hand (`src/gpkg/sqlite.rs`): the 100 byte header, table and index
b-trees, overflow chains, `sqlite_master`. Nothing reads a clock, a random number or the environment,
and pages are numbered in the order the trees are built, so identical input gives identical bytes. The
settings the digest is stable under are recorded in `gpkg::WRITE_SETTINGS`: page size 4096, rollback
journal (not WAL), schema format 4, UTF-8, `application_id` `GPKG`, `user_version` 10400 (1.4.0), and a
fixed SQLite version number in the header. The `last_change` stamp is a caller input (default
`1970-01-01T00:00:00.000Z`) for the same reason. The test suite pins the digest of a file made of
straight lines only; a file with arcs or approximations goes through the platform's sine, cosine and
square root, so its bytes are the same on one build and not guaranteed identical across libm versions.

The crate depends on nothing but the workspace's own crates. There is no SQLite library, no C source,
no build script and no library discovered on the build machine, which is what the downstream
`libviprs` dependency policy needs. A test reads `Cargo.lock` to keep it that way.

### Validation

Two kinds of evidence, and neither is the writer's own opinion of itself:

* `cargo test -p pvec-interchange` reads every file back with a small SQLite reader written separately
  from the writer (`tests/common`), and checks the layout, the rows, the extension rows, the spatial
  index structure, the geometry blobs and the measured deviations against exact evaluation.
* `tools/gpkg-validate.sh` writes sample files (`examples/gpkg_samples.rs`) inside a Rust container and
  checks them inside a throw-away GDAL container: no network, a read-only root, the samples mounted
  read-only, the image pinned by digest. The checks are the GeoPackage validator that ships with GDAL
  (`validate_gpkg.py`, which checks the 1.4 requirements including the extension rows and the
  R-tree triggers), SQLite's `integrity_check`, `foreign_key_check` and `rtreecheck`, and OGR as the
  independent GIS reader: it opens each file, counts the features, answers a spatial-filter query
  through the R-tree, reads every geometry (`CIRCULARSTRING` and `COMPOUNDCURVE` included), and
  measures the arcs (length and envelope) against the numbers the source circles give. The reader is
  GDAL 3.9.3 (released 2024-10-07, `ghcr.io/osgeo/gdal:ubuntu-small-3.9.3`) with SQLite 3.45.1.

### Not done

Reading GeoPackage, raster and tile tables in the same file, styling, reprojection, and the
display-only tiled paths (chapter 5) are out of scope. `CURVEPOLYGON`, `MULTICURVE` and the surface
types wait for an entity that needs them. Per-layer feature tables are not written: every entity is
in `pvec_features`, and the layer or component grouping would be one more attribute column when an
export wants it.

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
* The tested zoom range 0 to 14; maximum tested zoom 14.
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
