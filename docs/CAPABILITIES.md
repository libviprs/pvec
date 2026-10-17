# PVec capabilities

This file is generated from the registry in `crates/pvec-capabilities`. I do not edit it by hand: the gate regenerates it and fails if the committed file differs by a single byte. To change it I change the registry and run `PVEC_BLESS_CAPABILITIES=1 cargo test -p pvec-checks --test capabilities_doc`.

The STEP and GIS columns are what the semantic profile declares for each capability. Whether a declared exact form really round-trips to the same locus is measured in phases 2 and 6, not here.

## Registry

| id | name | class | version | ISO 10303-42 construct | profile section | requirement | STEP | GIS | min reader | restriction |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | `cartesian_point` | geometry | 1.0.0 | `cartesian_point` | 1 | required | exact form | exact | 1.0.0 | exactly two finite f64 coordinates; negative zero is stored as positive zero |
| 2 | `direction_vector` | geometry | 1.0.0 | `direction`, `vector` | 2 | required | exact form | not applicable | 1.0.0 | a unit direction, or a direction plus a strictly positive magnitude |
| 3 | `axis_placement_2d` | geometry | 1.0.0 | `axis2_placement_2d` | 3 | required | exact form | not applicable | 1.0.0 | a location and a unit reference direction; always right-handed |
| 4 | `line` | geometry | 1.0.0 | `line` | 4 | required | exact form | not applicable | 1.0.0 | unit direction, so the parameter is arc length; bounded only through a trimmed curve |
| 5 | `polyline` | geometry | 1.0.0 | `polyline` | 5 | required | exact form | exact | 1.0.0 | 2 to 1048576 vertices, vertex i at t = i, no zero-length segment |
| 6 | `circle` | geometry | 1.0.0 | `circle` | 6 | required | exact form | densified, error reported | 1.0.0 | radius strictly positive; canonical domain from 0 to 2 pi |
| 7 | `ellipse` | geometry | 1.0.0 | `ellipse` | 7 | required | exact form | densified, error reported | 1.0.0 | semi-axes are not reordered; axis ratio at most one million |
| 8 | `trimmed_curve` | geometry | 1.0.0 | `trimmed_curve` | 8 | required | exact form | densified, error reported | 1.0.0 | basis is a line, circle, ellipse, Bezier or B-spline; parameter trims only; exact in GIS only over a line |
| 9 | `composite_curve` | geometry | 1.0.0 | `composite_curve` | 9 | required | exact form | densified, error reported | 1.0.0 | segments join end to start; at most 65536 segments; exact in GIS only when every segment is straight |
| 10 | `bezier_curve` | geometry | 1.0.0 | `bezier_curve` | 10 | required | exact form | densified, error reported | 1.0.0 | degree 1 to 16, non-rational, domain 0 to 1 |
| 11 | `b_spline_curve` | geometry | 1.0.0 | `b_spline_curve_with_knots` | 11 | required | exact form | densified, error reported | 1.0.0 | degree 1 to 16, explicit knots, non-rational, no wrapped periodic form |
| 12 | `rational_b_spline_curve` | geometry | 1.0.0 | `rational_b_spline_curve` | 12 | required | exact form | densified, error reported | 1.0.0 | a B-spline plus strictly positive weights; equal weights stay rational |
| 13 | `affine_transform_2d` | geometry | 1.0.0 | `cartesian_transformation_operator_2d` | 13 | required | exact form | not applicable | 1.0.0 | any invertible 2x3 affine map; only the similarity subset has a Part 21 operator |

## Retired ids

None yet. A retired id stays here for good and is never assigned again.
