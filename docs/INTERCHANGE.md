# Interchange exports

This is the normative text for what PVec writes when it hands geometry to another format. Chapter 1
is the vocabulary every export reports in, which lives in `docs/FIDELITY_TAXONOMY.md` and is not
repeated here. Chapter 2 is the STEP export (P6.2). Chapter 3 is the GML export, which is this file's
first chapter to land, so the numbering leaves the others a place.

## 3. GML export

The GML export is `pvec_interchange::gml`. It turns a list of `(EntityId, Curve)` into one GML
document and a `FidelityLedger`, using the richest GML construct that says the same thing as the curve
and saying so in the ledger when none does.

### 3.1 Target profile and version

The declared target is **GML 3.2.1** (ISO 19136:2007), namespace `http://www.opengis.net/gml/3.2`, with
the schema documents OGC publishes under `https://schemas.opengis.net/gml/3.2.1/`. Those documents
carry `version="3.2.2"` (editorial revisions published in place); that is the same schema and the same
namespace. I vendor them unmodified in `tests/schemas/gml-3.2.1/` (digests in `SHA256SUMS`, which the
validation test checks) so validation needs no network.

The profile is the geometry part of GML, nothing application specific:

- The root is a `gml:MultiCurve` with `srsName` and a `gml:description` (see 3.2).
- Each entity is one `gml:curveMember` holding a `gml:Curve` with `gml:id="e<index>"` and a
  `gml:identifier` in code space `urn:pvec:entity` that carries the entity index, so a consumer can
  find the PVec entity again.
- A curve's `gml:segments` holds the segments the mapping table below picks: `gml:LineStringSegment`,
  `gml:Arc`, `gml:Circle`, `gml:Bezier` and `gml:BSpline`.

Constructs of the profile the export does not emit, and why:

| Construct | Why not |
|---|---|
| `gml:ArcString` | every PVec arc is a single arc, so `gml:Arc` already says it; merging neighbouring arcs would change structure for no gain |
| `gml:CubicSpline` | it interpolates points with end tangent vectors, which is a different curve family from a Bezier or B-spline of the same degree, so no PVec curve maps onto it exactly |
| `gml:CircleByCenterPoint` | it needs a unit of measure the caller has not given, and the export does not guess one |
| `gml:Curve` with `gml:OffsetCurve`, clothoids and the rest | PVec has no such curve |

### 3.2 Coordinate reference handling

The caller hands the export a `CrsTransform`: a CRS name and a `Transform2`, the source-to-CRS affine
map. There is no default. A blank name or one with a control character is refused, and the name is
written as the `srsName` of the `MultiCurve`, escaped.

The transform is applied to every curve exactly, through `Curve2d::transform` of the shared curve model.
A curve that cannot represent its own image is refused there and becomes an `Unsupported` record with the
rule that refused it. The matrix is written into the `gml:description` (`a`, `b`, `tx`, `c`, `d`, `ty`),
together with the deviation budget, so the file says what was done to the coordinates.

Nothing here knows what a CRS name means. There is no CRS database and no reprojection beyond the affine
map: the caller who needs a datum shift or a projection does it before, or hands over the matrix that
matches. The fidelity values and every error bound describe the curve after the transform, in the units
of the output CRS, which is what a GIS consumer measures against.

### 3.3 The mapping table

Every capability of the v1 registry has at least one row, and every row carries exactly one fidelity
value from `docs/FIDELITY_TAXONOMY.md`. A capability whose curve has no exact GML construct is
`BoundedApproximation` with a measured bound, or `Unsupported` with a reason, never `Exact`. The table is
generated from `gml::MAPPING` and a test fails if this copy drifts
(`PVEC_BLESS_INTERCHANGE=1 cargo test -p pvec-interchange --test gml_21` rewrites it).

<!-- mapping-table:start -->
<!-- mapping-table:end -->

The registry's GIS column (`docs/CAPABILITIES.md`) says "densified, error reported" for a circle, a
trimmed curve and the splines. That is the conservative statement the profile made before this mapping
existed. Three-point arcs and circles, `gml:Bezier` and `gml:BSpline` are exact in GML, so this export
does better than the column for those, and densifies only the rows marked `Tessellated` above.

### 3.4 What each value rests on

- **Exact** (`polyline`, `bezier_curve`, `b_spline_curve`, a point): the numbers are the source numbers.
  Degree, control points and knots come out as the shortest decimal that reads back to the same `f64`,
  and the tests read the XML back and compare them to the source bit for bit (with the transform
  applied first, since that is the geometry being exported). A knot vector is written as distinct knots
  with multiplicities, which is how GML wants it and loses nothing.
- **ExactReparameterized** (a trimmed line): the two end points come from the shared evaluator, so the
  origin and direction of the line become the segment's end points.
- **EquivalentCompound** (circles, arcs, and a rational spline whose weights are all equal): a circle or
  arc is three points and GML defines the curve as the circle through them. The export reconstructs that
  circle from the three points it wrote and compares centre and radius with the source under NP-04
  (`Policy::tol`). It keeps the construct only if they agree, and puts the measured difference in the
  record's note. A very small arc, where three nearby points cannot pin the circle down in floating
  point, falls back to a bounded polyline rather than claiming a circle it cannot show. Equal weights
  cancel in a rational spline, so the polynomial spline with the same control points and knots is the
  same locus.
- **BoundedApproximation**: see 3.5.
- **Unsupported**: an unbounded line (GML has no infinite line), a curve whose budget needs more
  vertices than a polyline may have (NP-18), and the capabilities that are not curves.

A composite curve is one `gml:Curve` whose segments are its members in order. Each segment gets its own
record, under the capability of the member (a circle arc inside a composite is a `trimmed_curve`). A
composite with a member that cannot be written is left out as a whole, with one `Unsupported` record,
because half a curve would be worse than none.

### 3.5 Approximation

GML 3.2.1 has no ellipse, and no way to carry the weights of a rational B-spline: `gml:Knot` has a
`weight`, but it is the ISO 19107 averaging weight of the knot, not a control point weight, so the
export writes 1 there and it carries no information. A trimmed ellipse, a trimmed spline and a rational
spline with unequal weights are therefore a `gml:LineStringSegment` with `BoundedApproximation`.

The caller gives a deviation budget (`Budget`), in output CRS units. The export cuts the curve at its
knots (a polyline must not cut across a corner) and subdivides every piece uniformly, doubling until the
bound is inside the budget. The bound it records is 1.25 times the largest distance from 64 interior
samples per piece, taken through the shared evaluator, to that piece's chord, and never below the NP-04
tolerance for the curve's size and magnitude (a curve flat to the last bit does not get a bound of
zero). The factor covers the distance from the chord back to the curve and the gap between samples. This
is a measured bound, the way `docs/FIDELITY_TAXONOMY.md` allows, not a proof, so the tests check it from
outside: they sample the exact curve 20000 times and the exported polyline's segments nine times each,
measure the distance in both directions, and require it to be at or below the recorded value.

If the budget cannot be met inside the polyline vertex limit, or is below what the numeric policy
resolves for that curve, the entity is `Unsupported` with that reason. No tolerance is written in the
module: all of them come from the numeric policy, and the one number the caller supplies is the budget.

### 3.6 Determinism

The same input gives the same bytes. Attribute order is fixed in the writer, entities keep the caller's
order, numbers are the shortest round-trip decimal with negative zero written as `0`, and nothing in the
output depends on a clock, a hash order or the host. A test compares two runs byte for byte and another
pins the exact document for a small input.

### 3.7 Validation

Every fixture's output is validated against the vendored GML 3.2.1 schema in a container that the test
starts and removes (`crates/pvec-interchange/tests/gml_21_schema.rs`). The container is the public
`eclipse-temurin` JDK image, pinned by the digest of its multi-platform index, run with no network, a
read-only root, and the schemas and documents mounted read-only. The validator is the JDK's own XML
Schema validator (`tests/support/Validate.java`, a single-file program), so nothing is installed at run
time, and a resolver maps the few absolute `schemaLocation` URLs onto the vendored files. The test also
feeds the validator a document the schema must refuse, so a validator that accepts anything cannot pass.

The test needs a docker daemon, so it is `#[ignore]` by default and runs with `--include-ignored` on a
host that has one. `tools/schema-validate.sh` does that on MARS from a toolchain container that shares the
host's docker socket and mounts the work tree path-identically (the daemon resolves bind-mount paths on
the host). Without docker the test follows the `pvec-hermetic` policy: a developer run
(`PVEC_DEV_RUN=1`) skips, anything else fails.

### 3.8 Not covered

Reading GML, GML features and attributes (the component capabilities are `Unsupported` rows, because
they are not geometry), exact trims of splines and rational B-splines, a GML profile that carries
rational weights, and the tiled display-only outputs (P6.6, P6.7).
