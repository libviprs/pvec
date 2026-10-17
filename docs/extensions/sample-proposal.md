# Extension proposal: hyperbolic arc (SAMPLE)

This is a SAMPLE. It is a complete proposal for a hypothetical curve, kept so the extension gate has an
example that passes end to end. It is not registered and it proposes nothing for v1.

## Part 42 constructs considered

- `hyperbola`: a hyperbola is an unbounded conic, so a bounded arc of it would need a trim that the construct does not define for this profile
- `trimmed_curve`: a trimmed curve over a hyperbola has the same missing basis, so it cannot be used until the basis exists
- `b_spline_curve_with_knots`: a rational spline can only approximate a hyperbolic arc to a tolerance, which fails the exact locus this extension is meant to give
- `rational_b_spline_curve`: a rational quadratic represents a hyperbola exactly, but only in a form that loses the parameter and the closed form bounds this extension defines

## Mathematical definition

The curve is the branch of the hyperbola x^2/a^2 - y^2/b^2 = 1 with x > 0, parameterised by t in the interval from minus T to T as the point (a cosh t, b sinh t) in a frame given by an axis placement 2D, with a and b strictly positive.

## Bounds and evaluation rules

Bounds are the extremes of cosh and sinh over the interval, taken at the end points and at t = 0, because both functions are monotone away from zero. A point is evaluated in closed form with no iteration, and the derivatives follow from the derivatives of cosh and sinh.

## Tessellation rules

A tessellator receives the exact curve and subdivides by the sagitta bound, choosing the number of segments from the second derivative of the curve over the interval, so the error budget is met without the curve ever being flattened first.

## STEP behaviour

There is no exact representation in the profile; bounded approximation with error reported is written through the exact rational B-spline form of the arc, and the error of that form is stated in the export report.

## GIS behaviour

Densified into line work within the reported error budget, a display-fidelity result. fidelity class: display

## Conformance fixtures

- hyperbolic-arc-synthetic: KNOWN_TRUTH_EXACT
- hyperbolic-arc-dual-converter: DUAL_ORACLE_EQUIVALENT
