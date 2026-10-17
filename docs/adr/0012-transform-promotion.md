# ADR 0012: Transforms and promotion

Status: Accepted, 2026-10-16. Decides P3.8 (`Transform2` composition, the promotion table in
`docs/ISO_PROFILE.md`, `Curve::to_rational_b_spline`). Builds on ADR 0001 and ADR 0002.

## Context

The issue's promotion table lists "ellipse under shear" as becoming a rational B-spline. The profile
(ADR 0001) already says a circle under a non-similarity becomes an ellipse, and the P1.3 prototype
measured the transform of every kind under six maps with a worst deviation of 7.3e-16 relative to the
extent. The acceptance criteria are that no path approximates ("a polyline fallback fails the test"), that
singular maps are refused, that trim and transform commute within tolerance, and that composition is
associative within tolerance on huge coordinates with the worst case reported.

## Options

| question | options | evidence |
|---|---|---|
| ellipse under shear | A. promote to a rational B-spline as the issue's table says. B. keep an ellipse, and offer the rational form as an explicit conversion | the affine image of an ellipse is an ellipse, so B is exact and loses nothing, while A gives up the kind (a downstream STEP export would write a B-spline where an `ELLIPSE` is available and exact). A also needs the conversion B offers anyway. Measured in the P3.8 tests: the sheared ellipse's locus and the locus of its rational form both stay within the NP-04 tolerance |
| an image too eccentric for NP-07 | A. refuse. B. fall back to the rational form. C. fall back to a polyline | C is an approximation and is excluded by the issue. B stores the same ill-conditioned minor axis in control points, so it does not make the curve better conditioned. A keeps the rule of NP-07 meaning something |
| where a singular map is caught | A. when applied to a curve. B. when the map is built | with B a degenerate map cannot reach a curve, so no code path can produce a zero-area curve, and the refusal carries the NP-19 rule |
| composing | A. multiply raw matrices unchecked. B. validate the product like any map | B refuses a product that leaves the coordinate range or loses its determinant instead of wrapping or collapsing |

## Decision

B in every row. `Transform2` gets `translate`, `rotate`, `scale`, `mirror_x`, `mirror_y`, `shear`, `then`
and `inverse`, all validated under NP-19. An ellipse stays an ellipse under any invertible map and a circle
becomes an ellipse unless the map is a similarity. `Curve::to_rational_b_spline` converts a circle, an
ellipse or an arc of either into the exact rational quadratic form (segments of at most a quarter turn,
weights `1, cos(phi/2), 1`); the result is the same locus over `[0, 1]`, not the same parameterisation.
The promotion table in `docs/ISO_PROFILE.md` names the target capability of every row by name and id and a
test reads it against the registry.

## Consequences

* The issue's "ellipse | shear | rational B-spline" row is replaced; a reader who relied on it gets an
  `Ellipse` and can ask for the rational form.
* There is no tessellation on the transform path, and `pvec-checks` fails if any geometry-path crate
  mentions it.
* Associativity of composition is not exact in floating point. The test prints the worst deviation over the
  huge-coordinate fixtures and holds it to the NP-04 tolerance; the number is in the PR description.

## Reversal threshold

Reopen the ellipse row if the STEP export (P6.2) finds that a sheared ellipse cannot be written as an
`ELLIPSE` with the orthogonal axes this profile uses, or if a consumer needs the rational form in place of
the conic as the transform output rather than as a conversion.
