# PVec 2D semantic profile

Status: normative for PVec phase 1. Numeric rule identifiers (`NP-nn`) are defined in
[`NUMERIC_POLICY.md`](NUMERIC_POLICY.md), which is the only place a tolerance is ever defined.

This is the document the other repositories cite instead of inventing their own mapping from CAD
entities to mathematics. I wrote it so that every later claim of exactness is falsifiable: each
capability below names one ISO 10303-42:2025 construct, one concrete Part 21 form, and the exact
list of things a decoder has to refuse.

## Claims

The only claim PVec makes about the standard is this one sentence, and nothing in the repository
may say more than it:

> PVec is aligned to named ISO 10303-42:2025 constructs and validated through STEP interchange.

Consequences I am holding myself to:

- No document, comment, test name or ADR describes PVec as conforming to, compliant with, or
  certified against ISO 10303-42. A check in `crates/pvec-checks` scans the repository for that
  wording and fails the gate when it finds some.
- "Aligned" means the PVec type has the same mathematical definition as the named construct over
  the restricted range given in its table. It does not mean PVec implements the construct's full
  EXPRESS schema, its WHERE rules beyond those restated here, or any other part of Part 42.
- "Validated through STEP interchange" means a PVec value can be written as the Part 21 form in its
  table and read back by an independent STEP implementation to an equivalent locus. The tests that
  do that live in phase 2 and phase 6, not here.
- The text of ISO 10303-42 is not redistributable and is not in this repository. Construct and
  attribute names below are the ones I use for the Part 21 forms. Clause numbers are deliberately
  not given, and the first reviewer with the licensed text is asked to check each table against it.
  Until then the tables are the PVec profile, not a quotation of the standard.

## The profile rule

A distinct DWG (or PDF, or any source) entity class is not a reason for a distinct PVec
mathematical type. An `ARC`, a `CIRCLE`, a polyline bulge and an `ELLIPSE` that happens to be
round all land on the types below. Source-entity mapping belongs to EPIC N in `libviprs/libviprs`,
never to this repository. Style, text and hatch semantics are outside the geometry oracle and
outside this profile.

## Conventions used in every entry

- **Dimension.** Everything is two-dimensional. A coordinate list of any other length is rejected.
  There is no implicit `z = 0` promotion. In Part 21 a 2D item lives in a representation context
  whose `coordinate_space_dimension` is 2, and its coordinate lists have two entries.
- **Numbers.** Coordinates, parameters, radii and weights are `f64`. The finite-value and magnitude
  rules (`NP-01`, `NP-02`) apply to every number in every entry and are not repeated per entry.
- **Angles.** Radians only. A Part 21 context declaring any other plane angle unit is outside the
  profile. The reference export always declares radians.
- **Handedness.** Positive rotation is counter-clockwise, from the first axis towards the second.
  There is no left-handed 2D placement.
- **Parameter domain.** Each entry states its native parameter and its domain. Comparing
  parameters across different curves goes through the unit map of `NP-13`. Part 21 does not carry
  composite or polyline parameter values at all, so for those two the comparable quantity is the
  locus (geometric exactness), and the parameter convention is a PVec definition, not an ISO one.
- **Trim convention.** Only `trimmed_curve` and `composite_curve_segment` trim anything. Every other
  entry is bounded by construction or unbounded and says so.
- **Rejection.** "A decoder rejects" means the whole containing entity is invalid and no partial
  value is produced. A rejected entity is reported, never repaired. Repair is the producer's job
  and is recorded as a fidelity loss by the producer.

## 1. Cartesian point

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `cartesian_point` |
| PVec restriction | exactly two `f64` coordinates; `-0.0` is stored as `+0.0` (`NP-03`) |
| Allowed dimensions | 2 |
| Parameter-domain convention | none (a point has no parameter) |
| Trim convention | none |
| Exact Part 21 form | `CARTESIAN_POINT('',(x,y));` |
| Unsupported Part 42 features | 1 and 3 coordinate forms (not 2D); `point_on_curve`, `point_on_surface`, `point_replica`, `degenerate_pcurve` (they are derived from other geometry, and PVec carries the derived coordinates instead) |

A decoder rejects: a coordinate count other than 2; any coordinate that is NaN, infinite or
subnormal (`NP-01`); any coordinate whose magnitude exceeds the bound in `NP-02`.

## 2. Direction and vector

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `direction`, `vector` |
| PVec restriction | `direction` is stored normalised to unit length; `vector` is a direction plus a strictly positive magnitude |
| Allowed dimensions | 2 |
| Parameter-domain convention | none |
| Trim convention | none |
| Exact Part 21 form | `DIRECTION('',(dx,dy));` and `VECTOR('',#direction,magnitude);` |
| Unsupported Part 42 features | 3D directions; zero-magnitude vectors (Part 42 permits a magnitude of zero in some contexts, PVec does not, because a zero vector has no direction to evaluate) |

A decoder rejects: a direction whose length is zero in the sense of `NP-10`; a stored direction
that is not unit length within the tolerance of `NP-05`; a vector magnitude that is not strictly
positive and finite.

## 3. Axis placement 2D

This is a supporting construct. It carries no locus of its own, but circle, ellipse and the
similarity transform are all defined relative to one.

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `axis2_placement_2d` |
| PVec restriction | a location point and a reference direction; the second axis is the reference direction rotated by +90 degrees, so the frame is always right-handed |
| Allowed dimensions | 2 |
| Parameter-domain convention | none |
| Trim convention | none |
| Exact Part 21 form | `AXIS2_PLACEMENT_2D('',#location,#ref_direction);` with the reference direction always written (PVec never relies on the optional default) |
| Unsupported Part 42 features | `axis1_placement`, `axis2_placement_3d` (3D only); an omitted reference direction on input is read as `(1,0)` only by the Part 21 importer, never by the container decoder |

A decoder rejects: a reference direction that fails section 2; a frame whose axes are not
orthonormal within `NP-05`.

## 4. Line

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `line` |
| PVec restriction | canonical form has a unit direction and magnitude 1, so the parameter is arc length from the origin point. A bounded segment is a `trimmed_curve` over a line (section 8). There is no separate "segment" mathematical type |
| Allowed dimensions | 2 |
| Parameter-domain convention | `P(t) = origin + t * direction`, `t` real and unbounded for the carrier |
| Trim convention | unbounded carrier; bounded only through a trimmed curve |
| Exact Part 21 form | `LINE('',#origin,#vector);` with the `VECTOR` of magnitude 1, and a bounded segment as `TRIMMED_CURVE('',#line,(PARAMETER_VALUE(t0)),(PARAMETER_VALUE(t1)),.T.,.PARAMETER.);` |
| Unsupported Part 42 features | a `vector` of magnitude other than 1 as stored form (an importer rescales the trim parameters; the container never stores it); 3D lines |

A decoder rejects: a segment whose length is degenerate under `NP-06`; a non-unit direction; an
endpoint outside `NP-02`.

## 5. Polyline (polyline-equivalent)

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `polyline` |
| PVec restriction | `n` vertices with `2 <= n <= MAX_POLYLINE_VERTICES`; storage optimisation with the same locus as a composite of `n-1` trimmed lines. A closed polyline repeats its first vertex as its last, because `polyline` has no closed flag |
| Allowed dimensions | 2 |
| Parameter-domain convention | PVec definition: vertex `i` is at `t = i`, linear in between, domain `[0, n-1]`. Part 21 does not carry this parameter, so only the locus is comparable |
| Trim convention | bounded by construction |
| Exact Part 21 form | `POLYLINE('',(#p0,#p1,...,#pn));` |
| Unsupported Part 42 features | bulge or arc segments inside a polyline (those are a composite of lines and trimmed circles, section 9); variable width; per-vertex data |

A decoder rejects: fewer than 2 vertices; more than the count limit of `NP-18`; any consecutive
pair of vertices closer than the degenerate length of `NP-06`.

## 6. Circle

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `circle` (the circular case of the `conic` family) |
| PVec restriction | radius strictly positive; placement per section 3 |
| Allowed dimensions | 2 |
| Parameter-domain convention | `P(t) = c + r (cos t * x_axis + sin t * y_axis)`, `t` in radians, period 2 pi; canonical stored domain `[0, 2 pi)` after `NP-11` |
| Trim convention | the full carrier is periodic; arcs are trimmed curves (section 8) |
| Exact Part 21 form | `CIRCLE('',#placement,radius);` |
| Unsupported Part 42 features | `hyperbola` and `parabola` (the other `conic` members: no v1 source produces them, and they are unbounded, so they would need their own bounds policy; they can arrive through the extension rules of P3.5) |

A decoder rejects: a radius that is not finite, not positive, or degenerate under `NP-07`; a
placement that fails section 3.

## 7. Ellipse

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `ellipse` |
| PVec restriction | `semi_axis_1` along the placement's first axis, `semi_axis_2` along its second, both strictly positive. They are not reordered, so the major axis may be either. A ratio that makes the ellipse a circle is still an ellipse; PVec does not auto-convert |
| Allowed dimensions | 2 |
| Parameter-domain convention | `P(t) = c + a cos t * x_axis + b sin t * y_axis`, `t` in radians, period 2 pi, canonical domain `[0, 2 pi)` after `NP-11`. The parameter is the eccentric anomaly, not the polar angle |
| Trim convention | trimmed curve (section 8) |
| Exact Part 21 form | `ELLIPSE('',#placement,semi_axis_1,semi_axis_2);` |
| Unsupported Part 42 features | the same conic members as section 6 |

A decoder rejects: either semi-axis degenerate under `NP-07`; an axis ratio larger than the
conditioning bound in `NP-07`; a placement that fails section 3.

A producer that holds a centre, a major-axis vector and a minor/major ratio (a DWG `ELLIPSE`) builds the
ellipse with `Ellipse::from_major_axis`: the placement is the vector normalised (`NP-10`), `semi_axis_1`
is the vector's length and `semi_axis_2` is that length times the ratio, so the major axis is first. The
ratio must lie in `(0, 1]` and give an axis ratio inside `NP-07`; the source numbers are passed as they
are, and the one product stays inside PVec.

## 8. Trimmed curve

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `trimmed_curve` |
| PVec restriction | basis is a line, circle, ellipse, Bezier or B-spline (sections 4, 6, 7, 10, 11, 12). A trimmed curve may not be the basis of another trimmed curve, and a polyline or composite curve is never a basis. Both trims are parameter values and the master representation is parameter; a point-form trim is never stored |
| Allowed dimensions | 2 |
| Parameter-domain convention | the basis curve's own parameter. A periodic basis is normalised by `NP-11`; a non-periodic basis requires both trims inside the basis domain |
| Trim convention | `trim1`, `trim2`, `sense_agreement`. With sense true the curve runs from `trim1` to `trim2` in the direction of increasing basis parameter; with sense false it runs the other way. The stored canonical form is an increasing interval `[a, b]` plus a `reversed` flag (`NP-11`, `NP-12`) |
| Exact Part 21 form | `TRIMMED_CURVE('',#basis,(PARAMETER_VALUE(t1)),(PARAMETER_VALUE(t2)),.T.,.PARAMETER.);` (`.F.` for reversed) |
| Unsupported Part 42 features | trims given as `cartesian_point` (the point form needs a projection step that has no single answer near self-approach); `.CARTESIAN.` and `.UNSPECIFIED.` master representation; trimmed curve over a surface curve or `pcurve` |

A decoder rejects: a collapsed trim domain under `NP-12`; a span of more than one period on a
periodic basis; a trim outside the basis domain on a non-periodic basis; a basis outside the list
above; a nesting depth beyond `NP-18`.

## 9. Composite curve

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `composite_curve` with `composite_curve_segment` |
| PVec restriction | each segment's parent is a trimmed curve, a polyline, a Bezier or a B-spline; nested composite curves are flattened in the Part 21 form (the container may store a composite as references to other records, to a depth of `NP-18`, and a Part 21 writer expands them; see ADR 0004, amendment for P4.3). The segments must join end to start within `NP-17` |
| Allowed dimensions | 2 |
| Parameter-domain convention | PVec definition: segment `k` (zero based) occupies `[k, k+1]` of the composite parameter, mapped linearly from that segment's own canonical domain. Part 21 does not carry this parameter, so only the locus is comparable |
| Trim convention | bounded by construction; each segment is already trimmed |
| Exact Part 21 form | `COMPOSITE_CURVE('',(#s0,#s1,...),.F.);` with `COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#parent);` per segment. `.CONT_SAME_GRADIENT.` and `.CONT_SAME_GRADIENT_SAME_CURVATURE.` are written only when the continuity has been verified under `NP-16`; otherwise `.CONTINUOUS.` (position only) |
| Unsupported Part 42 features | `.DISCONTINUOUS.` transitions (a gap is not a curve; the producer emits separate entities); `composite_curve_on_surface`; a segment whose parent is a `pcurve` |

A decoder rejects: a gap between consecutive segments larger than `NP-17`; more segments than
`NP-18`; a `.DISCONTINUOUS.` transition; a segment whose own entry (sections 4 to 8, 10, 11, 12)
would be rejected; a declared closed composite whose last end does not meet its first start.

## 10. Bezier

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `bezier_curve` (a `b_spline_curve` subtype with an implicit single-span knot vector) |
| PVec restriction | "where semantically applicable": a Bezier is the B-spline of section 11 with a clamped single span, kept as its own storage form because it needs no knot array. Degree `1..=MAX_DEGREE`, so `degree + 1` control points, non-rational. A rational Bezier is a rational B-spline (section 12) |
| Allowed dimensions | 2 |
| Parameter-domain convention | `t` in `[0, 1]` |
| Trim convention | bounded by construction; trimmed through a trimmed curve over it, which `NP-13` rewrites onto the unit domain |
| Exact Part 21 form | `B_SPLINE_CURVE_WITH_KNOTS('',degree,(#cp0,...,#cpd),.UNSPECIFIED.,.F.,.U.,(m,m),(0.,1.),.UNSPECIFIED.);` with `m = degree + 1`, the clamped single-span knot vector of section 11. `BEZIER_CURVE` is read by an importer but never written, so the interchange form has one spelling |
| Unsupported Part 42 features | `quasi_uniform_curve`, `uniform_curve`, `b_spline_curve_with_knots` knot-spec forms (`.UNSPECIFIED.` is written, and the explicit knots are the truth) |

A decoder rejects: a control point count other than `degree + 1`; a degree outside the limits of
`NP-18`; a degenerate point-curve under `NP-08`.

## 11. B-spline with knots

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `b_spline_curve_with_knots` |
| PVec restriction | non-rational; degree `1..=MAX_DEGREE`; explicit knot vector with multiplicities; control points as in section 1; no weights. The `curve_form` attribute is written `.UNSPECIFIED.` and ignored on read, so a file that says `.CIRCULAR_ARC.` does not make the curve a circle. `closed_curve` records geometric closure only and is checked, not trusted (`NP-20`). There is no wrapped periodic representation in v1 |
| Allowed dimensions | 2 |
| Parameter-domain convention | native knot values; the evaluated domain is `[knots[degree], knots[n]]` (zero based, `n` the last control point index), the usual clamped or unclamped definition. The unit map of `NP-13` takes it onto `[0, 1]` for comparison |
| Trim convention | bounded by the domain; further trimming through a trimmed curve |
| Exact Part 21 form | `B_SPLINE_CURVE_WITH_KNOTS('',degree,(#cp0,...),.UNSPECIFIED.,.F.,.U.,(m0,m1,...),(k0,k1,...),.UNSPECIFIED.);` The `self_intersect` logical is written `.U.` because PVec does not compute it, and ignored on read |
| Unsupported Part 42 features | the knot-spec shortcut subtypes (`uniform_curve`, `quasi_uniform_curve`, `piecewise_bezier_curve`), which carry no information beyond a particular explicit knot vector; `b_spline_curve` without knots; `rational_b_spline_curve` with non-positive weights; periodic wrap-around; `b_spline_curve` on a surface |

A decoder rejects: a knot vector that is not non-decreasing or whose length is not
`control_points + degree + 1`; a multiplicity rule violation of `NP-14` (including an interior
knot of multiplicity `degree + 1`); two distinct knots that are closer than `NP-14` allows; fewer
than `degree + 1` control points; a degenerate point-curve under `NP-08`; a count above `NP-18`; a
`closed_curve` flag that contradicts the geometry under `NP-20`.

## 12. Rational B-spline (NURBS-equivalent)

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `rational_b_spline_curve` combined with `b_spline_curve_with_knots` (a complex entity instance in Part 21) |
| PVec restriction | section 11 plus one strictly positive weight per control point. Evaluation carries the weights in homogeneous coordinates. A set of all-equal weights is still stored as rational; PVec does not silently demote it |
| Allowed dimensions | 2 |
| Parameter-domain convention | as section 11 |
| Trim convention | as section 11 |
| Exact Part 21 form | `(BOUNDED_CURVE() B_SPLINE_CURVE(degree,(#cp0,...),.UNSPECIFIED.,.F.,.U.) B_SPLINE_CURVE_WITH_KNOTS((m0,...),(k0,...),.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE((w0,...)) REPRESENTATION_ITEM(''));` |
| Unsupported Part 42 features | as section 11; zero or negative weights; weights as a separate homogeneous dimension |

A decoder rejects: everything section 11 rejects; a weight count different from the control point
count; a weight that is not finite and strictly positive; a weight below the conditioning floor of
`NP-15` relative to the largest weight.

## 13. 2D affine transform

| Field | Value |
|---|---|
| ISO 10303-42:2025 construct | `cartesian_transformation_operator_2d` for the similarity subset (translation, rotation, reflection, one uniform scale); `axis2_placement_2d` pairs for rigid placement |
| PVec restriction | any invertible 2x3 affine map `[a b tx; c d ty]` over `f64`, applied exactly to every curve in sections 4 to 12. Anisotropic scale and shear are allowed; a transform that is singular under `NP-19` is not |
| Allowed dimensions | 2 |
| Parameter-domain convention | B-splines, Bezier curves, polylines and composite curves keep their parameter. A line is kept in unit-direction form, so its parameter scales by the length `s` of the image of its direction (`t' = s t`). A circle or ellipse is re-expressed with orthogonal, right-handed axes, major first, and its parameter shifts: `t' = t - t0`, or `t' = t0 - t` when the map reverses handedness, where `t0 = atan2(2 u.v, u.u - v.v) / 2` and `u`, `v` are the images of the two semi-axis vectors. A circle stays a circle only when the map is a similarity, otherwise it becomes an ellipse (the full promotion table is P3.8's). Found by the P1.3 prototype: an earlier draft of this row said conics keep their parameter, which is wrong whenever the image axes are not the images of the old axes |
| Trim convention | trims move with their curve by the parameter map of the row above, so a trim of a transformed line is rescaled and a trim of a transformed conic is shifted, and the arc keeps its end points |
| Exact Part 21 form | similarity: `CARTESIAN_TRANSFORMATION_OPERATOR_2D('',#axis1,#axis2,#origin,scale);`. Non-similarity has no 2D operator in Part 42 that I know of (the operator carries one scale factor), so the reference export bakes the map into the geometry and writes the transformed curves, with circles promoted to ellipses |
| Unsupported Part 42 features | the 3D operators and per-axis scale; `item_defined_transformation` and `representation_map` as an instancing mechanism (instancing is a PVec component concern, not a geometry one); perspective and projective maps |

A decoder rejects: any matrix entry that is not finite or exceeds `NP-02` after composition with
the carried geometry; a matrix that is singular or near-singular under `NP-19`.

### Promotion table

This is what a transform does to each kind, with the exact result and the registry capability it lands
on (`docs/CAPABILITIES.md`; name and id). Every row is exact. Nothing in this table is ever approximated:
a transform never calls a tessellator and never turns a curve into a polyline.

| input | transform | exact result | target capability |
|---|---|---|---|
| line | any non-degenerate affine | a line; the direction is re-normalised and the parameter scales by the length of the image of the direction | `line` (4) |
| circle | similarity (rotation, translation, reflection, one uniform scale) | a circle with the scaled radius | `circle` (6) |
| circle | non-uniform scale, shear, or any non-similarity | an ellipse, axes re-expressed orthogonal and right-handed, major axis first | `ellipse` (7) |
| ellipse | non-uniform scale | an ellipse | `ellipse` (7) |
| ellipse | shear | an ellipse (the affine image of an ellipse is an ellipse), with its exact rational form available on request | `ellipse` (7) |
| ellipse or circle | any map whose image is too eccentric for NP-07 | refused with the NP-07 error | none (refused) |
| arc (trimmed curve over a circle or ellipse) | any non-degenerate affine | the trim parameters mapped analytically, so the arc keeps its end points, over the image of the basis | `trimmed_curve` (8) |
| trimmed curve over a line, Bezier or B-spline | any non-degenerate affine | the trim scaled with the line parameter, or unchanged for a spline, over the image of the basis | `trimmed_curve` (8) |
| polyline | any non-degenerate affine | a polyline with transformed vertices | `polyline` (5) |
| Bezier | any non-degenerate affine | a Bezier curve with transformed control points | `bezier_curve` (10) |
| B-spline | any non-degenerate affine | control points transformed, knots and weights preserved | `b_spline_curve` (11) |
| rational B-spline | any non-degenerate affine | control points transformed, knots and weights preserved | `rational_b_spline_curve` (12) |
| composite curve | any non-degenerate affine | every segment transformed, order and joints kept | `composite_curve` (9) |

A singular or near-singular map (NP-19) is refused when it is built, so it cannot reach a curve and no
zero-area curve comes out. A composition that leaves the coordinate range is refused as well.

One row differs from the first draft of the plan, which listed "ellipse under shear" as a rational
B-spline. The affine image of an ellipse is an ellipse, so the exact and simpler result is an ellipse;
`Curve::to_rational_b_spline` gives the exact rational form of a conic or an arc when a consumer wants
a spline (ADR 0012).

## Unsupported Part 42 constructs, by class

These are outside PVec v1 whichever entry above they might otherwise resemble. A file that carries
one is rejected, or the producer records the translation as lossy in the fidelity report of
phase 6.

| Class | Constructs | Reason |
|---|---|---|
| Surfaces | `plane`, `cylindrical_surface`, `conical_surface`, `spherical_surface`, `toroidal_surface`, `b_spline_surface` and the rest of the `surface` family | PVec is flattened 2D drawing data; a surface has no meaning in it |
| Topology | `vertex_point`, `edge_curve`, `oriented_edge`, `edge_loop`, `face_surface`, `advanced_face`, `closed_shell` and the `topological_representation_item` family | Topology and solid modelling are an explicit non-goal of the epic; adjacency is not stored |
| 3D-only | any 3D coordinate list, `axis1_placement`, `axis2_placement_3d`, 3D transformation operators | The profile is 2D; there is no implicit dimension promotion |
| Swept geometry | `surface_of_revolution`, `surface_of_linear_extrusion`, `swept_surface`, `swept_area_solid` | These are surfaces or solids built from a curve and a direction; no flattened-drawing source emits them |
| Curve-on-surface | `pcurve`, `surface_curve`, `intersection_curve`, `seam_curve`, `bounded_pcurve`, `composite_curve_on_surface` | They only exist relative to a surface |
| Derived curves | `offset_curve_2d`, `curve_replica` | A drawing exports the resulting geometry, not the construction. Carrying the construction would duplicate the geometry model |
| Other conics | `hyperbola`, `parabola` | Unbounded, and no v1 source produces them. The extension rules of P3.5 are the route in |

## What this document does not decide

Tolerances and degeneracy thresholds (all in `NUMERIC_POLICY.md`), the in-memory API (P1.3, P3.1),
the physical encoding (P1.4, P1.5), conformance tests (phase 2), and extension proposals (P3.5).
