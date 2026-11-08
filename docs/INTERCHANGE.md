# Interchange exports

What PVec writes into other formats, and what each export promises about the geometry it wrote. The
vocabulary every chapter uses is `docs/FIDELITY_TAXONOMY.md`; the code is the `pvec-interchange`
crate.

Chapter 1 is the STEP AP242 export (P6.2, pvec#19). Chapter 2 belongs to the round-trip validation (P6.3)
and is added by that issue. Chapter 3 is the GML export (P6.4, pvec#21), chapter 4 the
GeoPackage export (P6.5, pvec#22), chapter 5 the vector tile export (P6.6, pvec#23)
chapter 6 the PMTiles packaging (P6.7, pvec#24), chapter 7 the fidelity reports every export emits
(P6.8, pvec#25), chapter 8 what every export does with a raster (R2.3, pvec#222) and chapter 9 which route
a reader takes to display a file (ADR 0020, pvec#271).

## 1. STEP AP242 export

The STEP export is `pvec_interchange::step`. It turns a list of `StepEntity` (an id and an exact curve)
into one ISO 10303-21:2016 file and a `FidelityLedger`. Every parameter comes off the curve model
(`pvec-geometry`) and goes onto the Part 42 entity that `docs/ISO_PROFILE.md` names for it, so the file
is the mathematics of the artifact and not a picture of it.

### 1.1 Target profile

The file declares the AP242 schema in `FILE_SCHEMA`
(`AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }`) and uses the constructs of the
2D profile only:

- `#1` is the plane angle unit (radians, the only one the profile allows) and `#2` the 2D representation
  context (`GEOMETRIC_REPRESENTATION_CONTEXT(2)`), so every item lives in a context whose coordinate
  space dimension is 2.
- Each entity's instances follow, children before their parent, and the parent carries the name `e<index>`
  so a reader can find the PVec entity again. Everything else has an empty name.
- A `GEOMETRIC_CURVE_SET` collects the parents and a `SHAPE_REPRESENTATION` ties the set to the context.

I claim no more than the profile does: the curves are aligned to named ISO 10303-42 constructs and checked
by reading them back, not certified against the standard (see the claims section of `ISO_PROFILE.md`).

### 1.2 The mapping table

This table is `pvec_interchange::step::mapping_table_markdown()`; a test compares it with this file byte
for byte, and the writer takes each fidelity value and entity name from the same `MAPPING` constant. Every
capability of the v1 registry has a row, and a capability that cannot be written has an `Unsupported` row
with its reason, so nothing is dropped without saying so.

| capability | case | Part 21 entity | fidelity | how |
|---|---|---|---|---|
| `cartesian_point` | Point | `CARTESIAN_POINT` | `Exact` | two coordinates, each the shortest decimal that reads back to the same f64 |
| `direction_vector` | Direction | `DIRECTION, VECTOR` | `Exact` | the stored unit direction; a line's vector has magnitude 1 so the parameter stays arc length |
| `axis_placement_2d` | Placement | `AXIS2_PLACEMENT_2D` | `Exact` | location and stored first axis; the reference direction is always written |
| `line` | Line | `LINE` | `Exact` | origin and unit direction carried; the unbounded carrier is a valid STEP curve |
| `polyline` | Polyline | `POLYLINE` | `Exact` | every vertex carried in order; a closed polyline repeats its first vertex, as in the container |
| `circle` | Circle | `CIRCLE` | `Exact` | placement and radius carried |
| `ellipse` | Ellipse | `ELLIPSE` | `Exact` | placement and both semi-axes carried, not reordered |
| `trimmed_curve` | Trimmed | `TRIMMED_CURVE` | `ExactReparameterized` | the stored (start, span, reversed) becomes the trim pair (start, start + span) with sense .T., or (start + span, start) with sense .F.; basis and start are carried bit for bit and the end is one addition |
| `composite_curve` | Composite | `COMPOSITE_CURVE` | `Exact` | one COMPOSITE_CURVE_SEGMENT per member in order, parents carried unchanged, .CONTINUOUS. transitions, self_intersect .U. |
| `composite_curve` | CompositeTrimmed | `COMPOSITE_CURVE` | `ExactReparameterized` | as above, and at least one segment is a trimmed curve, so the entity takes the value of its weakest member |
| `bezier_curve` | Bezier | `B_SPLINE_CURVE_WITH_KNOTS` | `Exact` | degree and control points carried; the clamped single span is the explicit knots (0, 1) with multiplicity degree + 1 |
| `b_spline_curve` | BSpline | `B_SPLINE_CURVE_WITH_KNOTS` | `Exact` | degree, control points, knot values and multiplicities and the declared-closed flag carried |
| `rational_b_spline_curve` | RationalBSpline | `B_SPLINE_CURVE_WITH_KNOTS, RATIONAL_B_SPLINE_CURVE` | `Exact` | a complex entity: the B-spline of the row above plus one weight per control point; equal weights stay rational |
| `affine_transform_2d` | Transform | `none` | `Unsupported` | the only 2D operator in Part 42 carries one scale factor and is an instancing mechanism the profile leaves out; the map is applied to the geometry exactly (a circle becomes an ellipse where the promotion table says so) and the matrix is stated in FILE_DESCRIPTION |
| `layer` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `line_style` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `text` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `hatch` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `source_provenance` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `dwg_semantics` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `semantic_class` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `paint_order` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `stroke_paint` | Component | `none` | `Unsupported` | a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it |
| `raster` | Component | `none` | `Unsupported` | a raster is pixels, not geometry; the AP242 reference export writes the 2D curve profile only and records the raster as Unsupported (ADR 0018) |
| `calibration` | Component | `none` | `Unsupported` | a calibration is a scale and a georeference for the page, not geometry; the AP242 reference export writes the 2D curve profile only and records it as Unsupported (ADR 0019) |

### 1.3 What each value rests on

Every curve case is `Exact` except two, and neither of those is an approximation.

- `Exact` means the parameter list is carried bit for bit: degree, control points, knot values and
  multiplicities, weights, radii, placements, vertices and the declared-closed flag. The only change is
  that `-0.0` is `0.` (NP-03), which the curve model never stores anyway.
- `ExactReparameterized` is the trimmed curve. PVec stores a trim as `(start, span, reversed)`, and
  Part 42 wants two parameter values and a sense. The map is `(start, start + span)` with sense `.T.`, or
  `(start + span, start)` with sense `.F.` when reversed. The basis and the start are bit for bit; the end
  is one floating point addition, so it can differ from the end the author typed by a rounding. Reading the
  file back normalises the trim again (NP-11), and the test compares the span inside the policy
  tolerance. A composite with a trimmed segment takes the same value, because an entity carries the
  value of its weakest member.
- Bezier is written as `B_SPLINE_CURVE_WITH_KNOTS` with the explicit knots `(0, 1)` of multiplicity
  `degree + 1`. That is the form the profile gives, and the parameters are the same, so it is `Exact`.
  `BEZIER_CURVE` is never written, so the interchange form has one spelling.
- `closed_curve` is `.T.` exactly when the curve was declared closed and the claim was checked (NP-20).
  `self_intersect` is `.U.` because PVec does not compute it. The profile's table for the composite curve
  shows `.F.` there; I wrote `.U.` for the same reason as for splines, and `ISO_PROFILE.md` should be
  amended when someone with the standard checks it.
- There is no periodic wrapped form. A periodic spline reaches the writer already unwrapped
  (`BSpline::from_periodic`), clamped, with its locus and domain kept, which is what P3.8's table says.

No row is a tessellation and no row is a `BoundedApproximation`; a test asserts both.

### 1.4 Transforms

`StepOptions::with_transform` applies a 2D affine map to every curve with `Curve2d::transform`, which
returns the exact image or a typed refusal. A similarity keeps a circle a circle; any other map turns a
circle into an ellipse, exactly as the promotion table in `ISO_PROFILE.md` says, and the ellipse is
written as such, so the entity is still `Exact` for the curve it now is.

I did not write `CARTESIAN_TRANSFORMATION_OPERATOR_2D`. It carries one scale, it is only meaningful inside
a mapped item or an item defined transformation, and the profile puts instancing outside the geometry
oracle. The transform row is therefore `Unsupported` with that reason, the matrix is stated in
`FILE_DESCRIPTION` (so the file says what was done to the coordinates), and the curves in the file are the
image. If the curve model refuses a transform for an entity (NP-07 for an ellipse that gets too eccentric,
say), that entity gets an `Unsupported` record with the rule and nothing is written for it, and the counts
still add up.

### 1.5 Determinism

The same input gives the same bytes:

- Instance numbers start at 1, have no gaps, and follow the order entities are passed, children first.
  Nothing is shared between entities, so a curve's instances do not change when another entity does.
- A real is the shortest decimal that reads back to the same `f64` (Rust's `{:?}`), always with a decimal
  point and an upper-case `E` for the exponent, so `1e-7` (a ten-millionth) is written `1.E-7`.
- Strings double apostrophes and backslashes, and anything outside printable ASCII is written as
  `\X2\hhhh\X0\` (or `\X4\` past the BMP), so the file is 7-bit.
- The header has no clock. `FILE_NAME` carries the timestamp the caller passed (`YYYY-MM-DDThh:mm:ss`),
  the file name the caller passed and the crate version. Nothing depends on the host, so the arm64 and
  x86_64 lanes produce the same digest; `step_19.rs` pins the SHA-256 of the fixture export and the
  `docker-tests` stage of the gate runs it on both.

### 1.6 Nothing tessellated gets in

The writer's input type is `StepEntity`. It is an id and a `Curve`, built either with `StepEntity::exact`
or with `StepEntity::from_table`, which refuses an entity the entity table marks as `Preview`. `export`
takes `&[StepEntity]`, so a list of sampled points, a bare `(EntityId, Curve)` pair or the output of a
display export does not type-check. Two `compile_fail` doctests on `export` show it, next to a passing
one with the same boilerplate so the failure can only be the type.

### 1.7 Validation

Three layers, from cheapest to most independent:

1. The in-process round trip (`tests/step_19.rs`, with a small Part 21 reader in `tests/step_reader`)
   parses the file back, rebuilds the PVec curves through the public constructors and compares them:
   degree, control points, knots, multiplicities, weights, closed state and trim domain. This reader is
   mine, so it proves what survives and says nothing about other implementations.
2. The independent reader (`tests/step_19_reader.rs`) feeds the same files to a third-party Part 21
   parser in a container, and checks that it accepts them, that it refuses a file that is not Part 21,
   and that the entities it reports carry the parameters we wrote. The test prints the reader and its
   version, and `READER` in that file is the record of which one it was.
3. The digest test pins the bytes.

The container tests need a docker daemon, so they are `#[ignore]` and run in the `docker-tests` stage of
`tools/gate-fleet.sh local-ci pvec` on mars (x86_64) and mercury (arm64).

### 1.8 Not covered

Reading STEP into PVec, AP242 features outside the 2D profile (product structure, PMI, presentation, the
component streams), `CARTESIAN_TRANSFORMATION_OPERATOR_2D` and the GIS formats. A raster is not carried: the
writer reports an `Unsupported` row for each one and the geometry is untouched (chapter 8). The comparison against a CAD kernel is chapter 2.

## 2. Independent STEP round-trip validation

A writer checked by its own reader only proves it agrees with itself. This chapter is the check by something else.

### 2.1 Independence statement

The STEP files from chapter 1 are evaluated by OpenCASCADE (OCCT 7.6.3, the geometry kernel under FreeCAD), reached
through the FreeCAD 0.20.2 Python module that Debian bookworm ships as `freecad-python3`. The suite prints both
versions and asserts them (`EVALUATOR` in `tests/step_roundtrip.rs`). OCCT shares no source with
`pvec-interchange/src/step/writer.rs`, with its mapping table or with the in-process reader of `tests/step_reader`.

It runs out of process, in a container built from `tools/step-eval/Dockerfile` (Debian bookworm-slim pinned by digest,
nothing else but `freecad-python3` and its dependencies), published as `ghcr.io/libviprs/pvec-step-eval` and pinned by
the index digest in the test. The container has no network and a read-only root, is started for the run and removed
afterwards, on a pass and on a failure (`pvec-hermetic`). Nothing links it. FreeCAD is LGPL-2 or later, OCCT is
LGPL-2.1 with the OCCT exception, and the image carries their licence texts under `/usr/share/doc`.

Both architectures come from the same apt line, so the write-up of a run says which it was: the `docker-tests` stage of
`gate-fleet.sh local-ci pvec` runs it on mars and nas (native x86_64) and on mercury (native arm64).

### 2.2 The one thing I do to the file first

The writer's file is a 2D profile: a 2-dimensional representation context, two-component points and
`AXIS2_PLACEMENT_2D`. OCCT's STEP reader only imports 3D representations that hang off a product, and given the 2D
file it returns a null shape. So `tools/step-eval/evaluate.py` lifts the text before OCCT sees it: a zero third
coordinate on points and directions, each `AXIS2_PLACEMENT_2D` becomes an `AXIS2_PLACEMENT_3D`, and the units and the
product chain OCCT wants are added. No curve parameter (degree, knot, multiplicity, weight, radius, trim) is touched.
The parsing, the parameterisation and the evaluation after that are OCCT's. The edge count is asserted against the
number of curve pieces written (a composite is one edge per segment), so a merged or dropped edge fails.

### 2.3 What is compared

`step::verify` keeps three classes apart and reports each on its own, never as one verdict:

- Structural: degree, control points, knots with multiplicities, weights, radius, centre and axis, and the trim domain,
  for the edges where the parameters are directly comparable. Some are not, and the verdict lists them with the
  reason: a closed spline (OCCT stores it periodic), a trim that runs backwards (OCCT re-parameterises it), an
  ellipse written with the shorter semi-axis first. Rational weights are compared up to a common factor.
- Geometric: PVec's own `eval` and `derivative` against OCCT's `value` and `tangent` at nine deterministic fractions of
  the domain, at every interior knot (and polyline vertex) and one step either side of it, with the end points and the
  arc length where a closed form exists (circle arcs, lines, polylines). An unbounded line has no domain to sample.
- Display: only for an entity whose export was a `BoundedApproximation`, compared against its recorded bound. It is
  never counted as an exact round trip (`Verdict::exact_passes`).

Every tolerance comes from `pvec_numeric::Policy::V1`; the module declares none, and `step_roundtrip.rs` scans its
source for a float constant or an exponent literal. A disagreement is a record with the entity, the PVec parameters, the
evaluator's parameters, the deviating sample, the tolerance applied and the class that failed, and the suite fails on
any of them.

### 2.4 The corrupted mapping

`occt_reads_every_fixture_back_and_a_corrupted_mapping_turns_it_red` also writes the fixture file twice with one
change each: one knot multiplicity flipped on the cubic spline, one weight dropped on the rational curve. The evaluator
must report both as disagreements in both classes. That is what shows the comparison is load-bearing.

### 2.5 The corpus run and the gates

`pvec_harness::step_roundtrip` runs the same harness over the r0-open corpus (pvec#150). `curves_of` reads each
fixture's authored mathematics as PVec curves, `transform_of` its composed world map, and the test
`issue_150_occt_reads_every_corpus_fixture_back_and_the_gates_record_it` writes each fixture under that map, has OCCT
read it back and compares it with the moved curves. `Runner::step_roundtrip` then records the result as one
structural row and one geometric row with the fixture's own evidence class. A disagreement fails its row, a verdict
that compared nothing fails both (silence is not agreement), a fixture with no directly comparable parameter is routed
to the geometric gate, and a quarantined fixture is refused. The corrupted multiplicity of 2.4 is run on the cubic
spline of the corpus as well and must turn both rows red.

What the corpus run does not cover:

- `near-singular-001` (a mapper must not emit a curve for it), and the quarantined entries, which have no record.
- Anything that needs a vendor-written DWG or a converter's STEP output: those are `libviprs/pvec-gold-fixtures#42`
  (the vendor-qualified probes and O1.8 verdicts). This run exports PVec's own curves, so it is `KNOWN_TRUTH_EXACT`
  evidence for the writer and says nothing about a converter, and it never produces `DUAL_ORACLE_EQUIVALENT`.
- A periodic spline is compared in the clamped form `BSpline::from_periodic` makes of it, not in the wrapped form of
  the record.

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
| capability | case | GML construct | fidelity | how |
|---|---|---|---|---|
| `cartesian_point` | Point | `gml:pos` | `Exact` | both coordinates are written as the shortest decimal that reads back to the same f64 |
| `direction_vector` | Direction | `none` | `Unsupported` | a direction is not a curve and GML has no direction construct; it is consumed by the circle and line mappings |
| `axis_placement_2d` | Placement | `none` | `Unsupported` | a placement is not a curve and GML has no placement construct; it is consumed by the circle mapping |
| `line` | LineUnbounded | `none` | `Unsupported` | an unbounded line has no GML construct; a bounded one is a trimmed curve |
| `polyline` | Polyline | `gml:LineStringSegment` | `Exact` | every vertex is carried unchanged |
| `circle` | CircleFull | `gml:Circle` | `EquivalentCompound` | three points on the circle; the circle through them is checked against the source under NP-04 and the start point is the first one |
| `circle` | CircleTessellated | `gml:LineStringSegment` | `BoundedApproximation` | taken only when the three points cannot reproduce the circle under NP-04 |
| `ellipse` | EllipseTessellated | `gml:LineStringSegment` | `BoundedApproximation` | GML 3.2.1 has no ellipse; a sampled polyline with a measured bound |
| `trimmed_curve` | TrimmedLine | `gml:LineStringSegment` | `ExactReparameterized` | the two end points, evaluated by the shared evaluator; origin and direction become the end points |
| `trimmed_curve` | TrimmedArc | `gml:Arc` | `EquivalentCompound` | start, middle and end point of the arc; checked against the source under NP-04 |
| `trimmed_curve` | TrimmedFullCircle | `gml:Circle` | `EquivalentCompound` | as a circle; the start point and direction are kept |
| `trimmed_curve` | TrimmedSpline | `gml:BSpline` | `ExactReparameterized` | the basis with both trim parameters inserted as knots, then cut to a clamped spline over the trimmed curve's own parameter (tau = s - start, or start + span - s when reversed) |
| `trimmed_curve` | TrimmedTessellated | `gml:LineStringSegment` | `BoundedApproximation` | an ellipse or rational spline trim, or an arc that failed the three-point check; a sampled polyline with a measured bound |
| `composite_curve` | Composite | `gml:Curve` | `Exact` | the segment list keeps the order and the joins; each member has its own record |
| `bezier_curve` | Bezier | `gml:Bezier` | `Exact` | degree and control points carried; the two knots are the fixed domain 0 to 1 |
| `b_spline_curve` | BSpline | `gml:BSpline` | `Exact` | degree, control points and knots (value and multiplicity) carried |
| `rational_b_spline_curve` | RationalEqualWeights | `gml:BSpline` | `EquivalentCompound` | equal weights cancel, so the polynomial spline has the same locus |
| `rational_b_spline_curve` | RationalTessellated | `gml:LineStringSegment` | `BoundedApproximation` | GML 3.2.1 cannot carry control point weights; a sampled polyline with a measured bound |
| `affine_transform_2d` | Transform | `none` | `Unsupported` | GML has no transform construct; the matrix is applied to the geometry and written into the description |
| `layer` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `line_style` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `text` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `hatch` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `source_provenance` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `dwg_semantics` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `semantic_class` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `paint_order` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `stroke_paint` | Component | `none` | `Unsupported` | a component stream is not geometry; the GML export writes curve geometry only |
| `raster` | Component | `none` | `Unsupported` | a raster is pixels, not geometry; the GML export writes curve geometry only and records the raster as Unsupported (ADR 0018) |
| `calibration` | Component | `none` | `Unsupported` | a calibration is a scale and a georeference for the page, not geometry; the GML export writes curve geometry only and records it as Unsupported (ADR 0019) |
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
- **ExactReparameterized** (a trimmed Bezier or B-spline, `TrimmedSpline`): both trim parameters are
  inserted as knots up to multiplicity `degree + 1` (Boehm, `pvec_geometry::BSpline::sub_range`), and
  the control points and knots outside the trim are cut away. A Bezier goes through its clamped B-spline
  form first. The result is a `gml:BSpline` whose parameter is the trimmed curve's own one, so its first
  knot is exactly 0 and its last is exactly the span. The map from the basis parameter `s` is written
  into the record's note (`tau = s - start`, or `tau = start + span - s` for a reversed trim, in which
  case the control points are written in reverse so the curve starts where the trimmed curve starts).
  The only inexactness is the rounding of the insertions, which is bounded in the doc of `sub_range`,
  so the tests compare the written spline with the trimmed source through the shared evaluator.
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
export writes 1 there and it carries no information. A trimmed ellipse, a trimmed rational spline and a
rational spline with unequal weights are therefore a `gml:LineStringSegment` with `BoundedApproximation`.
A trimmed polynomial spline or Bezier is not in that list any more, see 3.4.

On control point weights (pvec#134): the vendored 3.2.1 schemas have no control point weight on
`gml:BSpline` (the only `weight` is the knot's), and I did not find an agreed newer profile or
application schema that carries one, so I did not add a second declared profile. GML 3.2.1 stays the
only (and so the default) profile. If a target that carries weights is agreed, it goes in here as a
second declared profile and 3.2.1 stays the default. Where a rational spline is a conic, an
`EquivalentCompound` of exact arcs is the other route, and it is not done here.

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
chord) could be missed. The tests compare it against an independent measurement on its own grid of 1500 samples,
which has to stay at or below the tolerance and not above the recorded value by more than one
percent.

A tolerance the coordinates cannot resolve is refused with `ToleranceUnreachable` rather than
looped on: at or below four times the numeric policy's tolerance (NP-04) for the curve's magnitude, or when
the subdivision would pass 250,000 vertices or sixty levels. An entity this happens to is recorded in
the ledger as `Unsupported` with the reason, and the rest of the file is still written. A tolerance that is zero,
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
  from the writer (`tests/gpkg_reader`), and checks the layout, the rows, the extension rows, the spatial
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

Reading GeoPackage, GeoPackage tile tables, styling, reprojection, and the
display-only tiled paths (chapter 5) are out of scope. The feature tables carry no raster; a raster in the
source artifact is counted as left out in the report (chapter 8). `CURVEPOLYGON`, `MULTICURVE` and the surface
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
  specification that shares no code with the encoder (and by a third-party one, see below). It checks command ids and counts, parameter
  availability, key and value indices, and that every vertex lies inside the buffered tile.

### Second-opinion decoder

The decoder in the tests is independent of the encoder, but I wrote it, so it is not a second opinion.
The ignored test `issue_133_gdal_mvt_driver_agrees_on_every_tile_the_export_produces` writes every
non-empty tile at zoom 0, 2 and 3 of the `crossing-curves` fixture and reads them with the MVT driver of
GDAL 3.9.3, in the pinned `ghcr.io/osgeo/gdal` image that `tools/gpkg-validate.sh` already uses. It
runs in a throw-away `pvec-hermetic` container (no network, read-only root), not as a Cargo dependency,
because the tree may hold only the pure Rust crates ADR 0006 and ADR 0016 allow. It compares the
layers (names), the features (count), the ids, the attributes (`entity_id`, `world_deviation` to the
last digit, `display_only`) and the geometry (every linestring piece, vertex for vertex in tile units)
against our test decoder, and it checks that GDAL refuses a file that is not a tile. They agree on all
of it. The `docker-tests` stage of `gate-fleet.sh local-ci pvec` runs it.

### Why the tessellation bound is not proven

The bound in the ledger is a measurement. `tessellate` compares each final chord with the exact
evaluator at `DEVIATION_SAMPLES` (7) interior points, and the acceptance test cross-checks with 4000
dense samples per curve at every zoom. Neither is a proof for an arbitrary spline: a proof needs a
bound on a derivative of the curve (the second derivative for chord deviation, so a chord of parameter
width h deviates by at most h^2/8 times that bound), and the evaluator in `pvec-geometry` does not
expose one. Circles, lines and polylines would have a closed form, but a rational spline or a trimmed
ellipse would not, and a bound that holds for only some curves would have to be a second code path with
a second claim. The adaptive tessellator with derivative-bound subdivision is P3.6/P7.2. When it lands
this export switches to it, the sampling goes, and the ledger bound becomes a proven one. Until then the
ledger says what it is, a measured bound, and `docs/FIDELITY_TAXONOMY.md` allows exactly that.

### Why anisotropic maps are refused

`Projection::new` returns `NotSimilarity` for a shear or a non-uniform scale. The screen budget is
a length on the screen, and the tessellation tolerance is one scalar per zoom in world units. Under a
similarity those are the same number in every direction. Under a stretch a world deviation of d costs
between d times the smallest and d times the largest scale in screen pixels, depending on its direction,
so one scalar is either too loose in one direction or too tight (and wasteful) in the other. Supporting
it means a tessellator that takes a tolerance per direction, which is a change to `tessellate` and its
bound, not to the projection. Right now no source needs it (the fixtures and the corpus place a drawing with
a rotation, a uniform scale and a shift), so it stays refused, loudly, and the test
`issue_133_shear_and_non_uniform_scale_are_refused_with_the_similarity_error` keeps it that way. If a
real source turns up with one, that is a new issue with that source attached.

### Out of scope here

PMTiles packaging (chapter 6), MapLibre styling, polygon and fill output, and any use of an
MVT as a source for comparison. A raster in the source artifact is not drawn into a tile: the report counts it
as left out, and the tiles are byte-identical to the ones of the same drawing without it (chapter 8).

## 6. PMTiles packaging

A directory of tiles is not a deliverable, a single archive a client can stream with HTTP range
requests is. `pvec_interchange::pmtiles::write_pmtiles` packs the tiles of chapter 5 into one PMTiles
v3 archive per exported view. It packs them and nothing more: the archive stays a derivative of the
`.pvec` artifact.

### The non-canonicality rule

* A PMTiles archive is display fidelity only. Everything in it is a chapter 5 tile, so every entity
  in it is a bounded approximation or unsupported, and none is exact.
* A PMTiles archive is not an input to any conformance comparison. No conformance result may cite
  one, and no comparison, reference reader or container reader accepts one (a test feeds an archive
  to the reference reader and the container header parse, and both refuse it).
* A PMTiles archive is not re-imported as geometry. The module can write an archive and has no
  function that reads one back into curves.
* `libviprs/libviprs#1028` (EPIC L) already ships a vector PMTiles writer for its own tessellated
  path. This writer neither changes nor depends on it, and the two archives are not interchangeable
  evidence of anything.

The rule is written down because the alternative was an assumption (panel finding 8).

### Layout

* One archive per view. A 127 byte header, the root directory, the JSON metadata, the leaf
  directories if there are any, then the tile data in tile id order.
* The header and the root directory sit in the first 16 KiB (16384 bytes), which is the one request a
  client has to make before it can find any tile. If all the directory entries do not fit in what is
  left after the header, the writer moves them into leaf directories of 1024 entries, doubling the
  leaf size until the root fits.
* Tile ids follow the specification: the Hilbert curve index inside a zoom, after the tiles of all
  smaller zooms. Tile data and directories are stored uncompressed (compression none), so the writer
  needs no codec. Tiles with nothing in them are not addressed.
* The header bounds are the whole web mercator world square and the centre is its middle. The map of
  a drawing is a plain square, not a place on Earth, and the archive does not pretend otherwise.

### Metadata

* `vector_layers` is derived from the tile bytes the writer is given, never from a list the caller
  keeps: one entry per layer that some tile holds, with the attribute names and types that some
  feature really uses (`entity_id` and `world_deviation` are `Number`, `display_only` is `Boolean`)
  and the lowest and highest zoom the layer appears at. A layer nobody emitted is not declared.
  Bytes that are not a vector tile, or a key used with two types in one layer, fail the writer.
* `pvec` states what the archive approximated:
  * the source artifact digest (`sha256:` and 64 hex digits, from P4.6);
  * the tessellation tolerance, as the screen budget in pixels;
  * the tested zoom range (`tested_zoom_min`, `tested_zoom_max`), and a tile outside it fails the writer;
  * the fidelity summary: the number of ledger records of each of the five taxonomy values, the
    total, and the largest error bound recorded;
  * `display_only: true` and `canonical: false`.
  A missing digest, tolerance or zoom range fails the writer.

### Determinism

The same tile set gives a byte-identical archive. Tiles are sorted by id before anything is written,
so the order the caller passes them in does not matter, directories are built from that order alone,
and metadata keys are written sorted. There is no timestamp unless the caller passes `created`,
which is copied in as given. The writer never reads a clock.

### Validation

* An independent reader in the test suite (written from the specification, sharing no code with the
  writer) finds every tile through the root and the leaf directories.
* `go-pmtiles verify` (the protomaps project's own tool, in a container pinned by digest) accepts
  the archive, and refuses two damaged copies.
* The archive is put in an S3 compatible object store (a container pinned by digest, started and
  removed inside the test, including when it fails) and read back with HTTP range requests, the first
  16 KiB first and then each tile by the directory entry a client would decode. Both container tests
  run in the `docker-tests` stage of the merge gate.

### Not done

Serving, CDN and cache headers, MapLibre style authoring and mobile offline packaging. A raster
pyramid is not part of the vector packaging above; it has its own passthrough in both directions, chapter 8.
Identical tile contents are not deduplicated, so the archive has no run lengths above one.

## 7. Fidelity reports

An export that only says "done" hides what a reviewer needs. Every export can hand back a
`pvec_interchange::report::Report`: STEP, GML and GeoPackage through `report()` on their export value,
MVT through `mvt::report` and PMTiles through `pmtiles::report` (both take the tiles). A report is a value;
`to_json()` and `to_markdown()` render the same data, so the two never disagree. Each export has a test
that fails if it writes output without producing one.

### Counting

The ledger of an export can hold several records for one entity: GML writes one per segment, and a tile set
repeats an entity in every tile it reaches. The report counts each entity once, in the weakest class any of
its records has (the order is the taxonomy order: `Exact`, `ExactReparameterized`, `EquivalentCompound`,
`BoundedApproximation`, `Unsupported`), with the largest error any of them measured. The per-class counts
therefore sum to the entity total, no entity is in two classes and none is missing. `Report::new` takes the
artifact's entity count and refuses a ledger that covers a different number of entities.

For a tile set the count is the entities that reach at least one tile. An entity outside every tile has no
record, and the tiles say nothing about it.

### The JSON document

The document is UTF-8 with a trailing newline, keys in a fixed order, and numbers written as the shortest
decimal that reads back to the same `f64`. It carries no timestamp and does not depend on the order the
records arrived in, so the same artifact and export options give a byte-identical report. The schema is
version `1.0`.

| key | meaning |
|---|---|
| `schema` | always `pvec-fidelity-report` |
| `schema_version` | `major.minor` of this schema; any change to the keys bumps it |
| `source_digest` | `sha256:` and 64 hex digits, the digest of the source artifact (P4.6) |
| `target` | the export target |
| `name` | the target, for example `GML` |
| `version` | the version of the target format the export wrote |
| `representation` | `Native`, `Tessellated` or `Mixed`, from the target constructs of the records |
| `conformance` | which claims the report may make, one object per conformance class |
| `structural` | structural exactness: entities that are `Exact` or `ExactReparameterized` |
| `geometric` | geometric exactness under P1.2: the above plus `EquivalentCompound` |
| `display` | display fidelity only: `BoundedApproximation` |
| `claimed` | whether any entity makes a claim of that class |
| `entities` | the entity accounting; inside `conformance` it is the number of entities making that claim |
| `total` | the artifact's entity count |
| `by_fidelity` | the entity count per fidelity value, with a key for each of the five values |
| `Exact` | entities in that class (so for `ExactReparameterized`, `EquivalentCompound`, `BoundedApproximation` and `Unsupported`) |
| `rows` | one row per entity, ordered by entity id |
| `entity` | the entity id (`EntityId` index) |
| `fidelity` | the entity's class |
| `capability_id` | the capability of the record that decided the class |
| `target_construct` | the construct that record wrote, for example `ARCS` |
| `records` | how many ledger records the entity had |
| `measured_error` | the largest measured error among the entity's records, or `null` |
| `non_exact` | for each class other than `Exact`, the full list of entity ids in it |
| `approximation_error` | the error summary over the entities that have a measured error |
| `basis` | always `measured` |
| `max` | the largest measured error, or `null` |
| `p99` | the nearest-rank 99th percentile of the measured errors (the value at rank ceil(0.99 n)), or `null` |
| `unsupported` | one entry per `Unsupported` entity |
| `capability` | the registry name of the capability, or `null` if it is not registered |
| `reason` | why the entity was not translated; never blank |
| `rasters` | what the export did with the artifact's rasters (schema 1.1, chapter 8) |
| `count`, `exported` | rasters in the artifact, and rasters the export carried |
| `unsupported` (inside `rasters`) | one entry per raster left out: `raster` (its id) and `reason`, never blank |

### What a report may claim

A tessellated target (MVT, PMTiles) records only `BoundedApproximation` and `Unsupported`, because the
ledger refuses anything else for a tessellated construct. Its report therefore has `claimed: false` and
zero entities for `structural` and `geometric`, and only `display` can be claimed. A test asserts this over
the serialized document of both exports. Rows of the other classes cannot appear in it.

### Measured, not requested

A `BoundedApproximation` row carries `measured_error`, the deviation the export measured against the source
curve (GML and GeoPackage measure it per segment or entity, MVT adds the tessellation and quantization
errors of the tile). The tolerance the caller asked for is never copied in. A test checks that the reported
value equals what the ledger measured and is not the requested tolerance.

### Unsupported features

Each `Unsupported` entity appears in `unsupported` with the `reason` and the `capability_id` of the source
capability (and its registry name), so a conversion's losses can be enumerated and diffed.

### Versioning

`SCHEMA_VERSION` is the schema version. `crates/pvec-interchange/tests/report_schema_25.txt` records the key
paths of every version, one line each, and a test compares the keys of a report that exercises every class
with the line for the current version. Adding, removing or renaming a key without bumping the version fails
that test. A new version appends a line and never edits an old one. Version `1.1` adds the `rasters` object
(chapter 8) and nothing else.

### Not done

The CLI that prints these reports (P7.7), report hosting and dashboards, the fixture evidence classes, and
benchmark numbers.

## 8. Rasters

A `.pvec` can carry a raster next to its vectors (ADR 0018, `docs/RASTER.md`): a placement in the `raster`
stream (capability 21) and the encoded tiles, untouched, in kind 9 and kind 10 blocks. PVec never decodes or
encodes a pixel, so what an export can do with a raster is limited to moving the bytes it was given or not
moving them. This chapter says which, for every export, so a raster never disappears without a line in the
report. The vocabulary is `docs/FIDELITY_TAXONOMY.md`.

### What each export does with a raster

| Export | A raster in the artifact | Report |
|---|---|---|
| PMTiles raster passthrough (this chapter, `pvec_interchange::pmtiles::export_raster`) | **carries** it: one archive per raster, every stored tile byte for byte | `exported`; `Exact` for bytes, see below |
| PMTiles raster import (`pmtiles::import_raster`) | **carries** it in: an archive and a placement become kind 9 and 10 blocks and a stream entry, bytes unchanged | a refusal names its reason |
| STEP AP242 Part 21 (chapter 1) | **drops** it, with an `Unsupported` row per raster naming the reason (the AP242 profile has no image construct); the geometry output is byte-identical to the file of the same drawing without the raster | `unsupported` with the reason |
| GML (chapter 3) | **drops** it, reported | `unsupported` with the reason |
| GeoPackage feature tables (chapter 4) | **drops** it, reported; GeoPackage tile tables are not written and not read | `unsupported` with the reason |
| MVT tiles (chapter 5) | **drops** it, reported; the tiles are byte-identical to the ones of the same drawing without the raster | `unsupported` with the reason |
| PMTiles vector packaging (chapter 6) | **drops** it, reported; same bytes as the drawing without the raster | `unsupported` with the reason |
| A raster whose `tile_codec` the export does not know, or whose tiles fail the header checks of `docs/RASTER.md` (`RASTER-03`) | **refuses** it, never writes a partial archive | the reason is the `RASTER-NN` rule |

"Drops" always comes with a report line: the count of rasters in the artifact, how many the export carried, and one
`unsupported` entry per raster left out with its reason. An export of a drawing with no raster reports a count of
zero. Dropping a raster changes no geometry and no tile of a vector export; a test per export compares the bytes
with the raster-free drawing.

### PMTiles passthrough, out

`export_raster` writes one PMTiles v3 archive for one raster, with the layout of chapter 6 (127 byte header, root
directory in the first 16 KiB, metadata, leaf directories if the root does not fit, tile data in tile id order)
and these rules:

* **Tile bytes.** Every stored tile is copied unchanged. Compression is none for the tiles (PNG, JPEG and WebP are
  compressed already) and none for the directories, as in chapter 6. No pixel is read, resampled or re-encoded, and
  no re-encode is offered.
* **Tile type.** From the placement's `tile_codec`: 1 PNG is PMTiles tile type 2, 2 JPEG is 3, 3 WebP is 4. There is
  no `Raw` tile type; a `Raw` archive is refused on import.
* **Address.** The pyramid has `levels = z_max + 1` and level `l` is `z = l`, where
  `z_max = ceil(log2(max(width, height) / tile_edge))` (zero when the image fits one tile). `x` is the tile column
  and `y` the tile row, rows counted down from the top as the Xyz numbering does, and the tile id is the one of
  chapter 6. A worked case: a 3000 by 1900 image at edge 1024 has `z_max = 2`, three levels, 3 by 2 tiles at
  `z = 2`, 2 by 1 at `z = 1` and 1 by 1 at `z = 0`. `min_zoom` is the coarsest level that stores a tile, `max_zoom`
  the finest. `z` is at most 31 in PMTiles; with a legal edge of 256 or more and a `u32` image size it never
  exceeds 24.
* **Sparse stays sparse.** A tile that the index stores with length zero has no entry in the directory, and the
  archive's addressed-tile count is the number of stored tiles. A client asking for it gets "no such tile", which
  draws the same as nothing.
* **Bounds.** The header bounds are the whole web mercator world square and the centre its middle, exactly as
  chapter 6 decides for the vector export. The archive is **not** georeferenced: the caller sets bounds if it knows
  a place, the way a libviprs archive does today. PVec stores no CRS (ADR 0018, "Georeference").
* **Metadata.** The placement goes under the `vnd.pvec` key, beside the `vnd.libviprs` key libviprs writes and
  designed next to the vector tiler's `viprs_cad` key: `{"raster": {...}}` with one member per field of the
  placement table in `docs/RASTER.md`, named as that table names them. The six `pixel_to_source` coefficients are
  written as 16 lowercase hex digits of the `f64` bits, so a JSON number never rounds them, and `-0.0` is already
  `+0.0` (NP-03). `attribution` is also written as the TileJSON `attribution` string at the top level, plain text,
  never HTML. `source_digest` is the optional hex SHA-256 of the placement. Keys are written sorted and the
  archive is deterministic: the same raster gives the same archive, with no clock.

### PMTiles passthrough, in

`import_raster` takes an archive and, optionally, a placement, and returns the placement (for the raster stream) and the
tiles (for `ContainerWriter::push_raster_tiles`), with every tile's bytes as they are in the archive. `placement_json`
and `placement_from_json` convert a placement to and from the `vnd.pvec` text, and `RasterRow::refused` makes the
report line of a refusal, whose text starts with the rule.

* **Placement.** When the archive carries `vnd.pvec`, that is the placement, validated like any stream row (NP-19,
  `RASTER-01`, `RASTER-05`, `RASTER-07`). A placement the caller passes is checked against it and a disagreement is a
  refusal; with neither, the import is refused, since pixels without a map to source space cannot be placed. Composing
  a placement from a georeference is the producer's job (`libviprs/libviprs#1328`), not this import's.
* **Tile type.** A `Raw` or unknown tile type is refused. MVT is not a raster and is refused with its own reason.
* **Edge.** The placement's edge is one of 256, 512 and 1024 like any stream row, and every tile's header must say
  it: a tile that says another edge, legal or not, is refused (`RASTER-03`). That is how the edge is sniffed from the
  tiles without a second parser.
* **Every tile is sniffed.** Each tile passes the codec sniffer of `RASTER-03` (header dimensions equal the edge, no
  trailing payload) before anything is returned. One tile that fails refuses the raster, with a report line naming
  `RASTER-03`, the tile address and the reason; the file is never partly imported.
* **Gaps.** An address the archive does not have is a tile that is not stored, so a sparse archive imports as a
  sparse pyramid. Entries with a run length above one expand to one tile per address, each with the same bytes.
* **Limits.** Counts and sizes are checked before the allocation they protect, against `LIMIT-14` and `LIMIT-15`.
  Internal compression of the directories (gzip) is not supported by this reader and is refused by name.

### Fidelity

The tile bytes and the placement bits are `Exact` in the sense of `docs/FIDELITY_TAXONOMY.md`: the same bytes, compared
by digest, and the same six coefficient bit patterns. There is no `BoundedApproximation` for pixels and none is
offered, since that value carries a bound in drawing units and a pixel has no such bound. A lossy tile codec is the
producer's choice; PVec claims only that it keeps the bytes. An archive built this way makes no claim about where on
Earth it is. It is an interop product, not a conformance input: no comparison accepts one, as for chapter 6.

Interop is checked with libviprs: an archive exported here and extracted by `viprs pmtiles extract` gives the same
tile bytes (recorded in the pull request; the automated version lives in `libviprs-tests`' `pmtiles_interop.rs`).

### Reports

Every report (chapter 7) carries a `rasters` object: `count` (rasters in the artifact), `exported` (rasters the
export carried) and `unsupported` (one entry per raster left out, with its `raster` id and a non-blank `reason`).
`count` equals `exported` plus the length of `unsupported`, and a report that does not reconcile is refused.

### Not done

Georeferencing an archive, re-encoding, GeoPackage tile tables, composing a placement from a `GeoTransform`
(`libviprs/libviprs#1328`), and the MLT and AVIF tile types.

## 9. Displaying a PVec

ADR 0020 decides it, and it is short. To look at a `.pvec` of ours, vectors and rasters together, open it in the
viewer in `viewer/`: a static page over the pvec-wasm module that draws the curves exactly (tessellated by the
module, with the error measured on screen) and the rasters from their own pyramid, in source space. A client that is
not ours, and only needs a picture, takes the PMTiles pair of chapter 8 (or the vector tiles of chapter 5) and loses
what those chapters say it loses. Nothing in this repository is a map library layer, and `libviprs/maplibre-pvec` is
archived.

The page takes `file=` (the URL of a `.pvec`) and `module=` (the URL of `pvec_wasm_bg.wasm`, by default next to the
page), or a local file from its picker, and `view=cx,cy,scale` to start somewhere other than the fitted view. A URL
whose server answers a `HEAD` with `Accept-Ranges: bytes` is read by ranges through the module's -7 loop (issue
#273): only what the window needs is fetched, every range carries the validator from the `HEAD` (or, when the `HEAD` has none, from the first range) as `If-Range` (the
rules of `HttpRangeSource`, READER.md), and a file that changed on the server stops the view with a message instead
of drawing bytes from another file. `ranges=0` fetches the file whole. When the file carries a calibration block (ADR 0019), the page shows the scale with its provenance and
uncertainty next to the manifest unit, the CRS id as it is, a scale bar at the effective scale, and the pointer in source
units and, through the stored affine, in CRS units (issue #274). It never reprojects and has no CRS database; an
uncalibrated file says so and its scale bar is in manifest units. `tools/viewer-test.sh` builds the module into `viewer/` and runs the page's
browser tests in a container; serving `viewer/` with the module beside it is all a deployment needs.
