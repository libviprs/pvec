//! Pathological cases that exercise the numeric policy itself. Each one names the rule it
//! tests, is built from the V1 constants (never from a literal tolerance), and says what the V1
//! policy must decide. Run under a widened policy, at least one case must change its answer.
#![allow(missing_docs)]

use core::f64::consts::{PI, TAU};
use pvec_numeric::*;

pub struct PolicyCase {
    pub id: &'static str,
    pub rule: &'static str,
    pub what: &'static str,
    /// True when the policy under test makes the decision V1 is required to make.
    pub run: fn(&Policy) -> bool,
}

const V1: Policy = Policy::V1;
const HALF_MAX: f64 = MAX_ABS_COORD / 2.0;

fn rejects(r: Result<impl Sized, NumericError>, e: NumericError) -> bool {
    matches!(r, Err(x) if x == e)
}

pub fn cases() -> Vec<PolicyCase> {
    vec![
        PolicyCase {
            id: "nan",
            rule: "NP-01",
            what: "NaN is refused",
            run: |_| rejects(check_value(f64::NAN), NumericError::NotFinite),
        },
        PolicyCase {
            id: "inf",
            rule: "NP-01",
            what: "infinity is refused",
            run: |_| rejects(check_value(f64::INFINITY), NumericError::NotFinite),
        },
        PolicyCase {
            id: "denormal-parameter",
            rule: "NP-01",
            what: "a subnormal parameter is refused",
            run: |_| rejects(check_value(f64::MIN_POSITIVE / 2.0), NumericError::Subnormal),
        },
        PolicyCase {
            id: "denormal-trim",
            rule: "NP-01",
            what: "a trim with a subnormal parameter is refused",
            run: |p| {
                rejects(
                    p.normalise_trim(f64::MIN_POSITIVE / 2.0, 1.0, true, 0.0, 2.0),
                    NumericError::Subnormal,
                )
            },
        },
        PolicyCase {
            id: "min-normal-ok",
            rule: "NP-01",
            what: "the smallest normal value is accepted",
            run: |_| check_value(f64::MIN_POSITIVE).is_ok(),
        },
        PolicyCase {
            id: "bound-edge",
            rule: "NP-02",
            what: "the coordinate bound is accepted and the next value up is not",
            run: |_| {
                check_value(MAX_ABS_COORD).is_ok()
                    && rejects(check_value(MAX_ABS_COORD * 2.0), NumericError::OutOfRange)
            },
        },
        PolicyCase {
            id: "negative-zero",
            rule: "NP-03",
            what: "negative zero is stored as positive zero",
            run: |_| check_value(-0.0).is_ok_and(|z| z.is_sign_positive()),
        },
        PolicyCase {
            id: "near-degenerate-segment",
            rule: "NP-06",
            what: "a segment of 4x the absolute floor is a real segment, half the floor is not",
            run: |p| !p.degenerate_length(V1.abs * 4.0, 1.0) && p.degenerate_length(V1.abs * 0.5, 1.0),
        },
        PolicyCase {
            id: "huge-coordinate-segment",
            rule: "NP-06",
            what: "at 2^39 a segment of 16 ULPs is real and one of 4 ULPs is lost in rounding",
            run: |p| {
                let u = ulp_of(HALF_MAX);
                !p.degenerate_length(16.0 * u, HALF_MAX) && p.degenerate_length(4.0 * u, HALF_MAX)
            },
        },
        PolicyCase {
            id: "huge-coordinate-near",
            rule: "NP-05",
            what: "at 2^39 values 4 ULPs apart are equal and 50 ULPs apart are not",
            run: |p| {
                let u = ulp_of(HALF_MAX);
                p.near(HALF_MAX, HALF_MAX + 4.0 * u, 1.0) && !p.near(HALF_MAX, HALF_MAX + 50.0 * u, 1.0)
            },
        },
        PolicyCase {
            id: "mixed-magnitude",
            rule: "NP-05",
            what: "a unit feature at 2^39 keeps its own relative scale: 0.5 apart is not equal",
            run: |p| !p.near(HALF_MAX, HALF_MAX + 0.5, 1.0),
        },
        PolicyCase {
            id: "relative-term",
            rule: "NP-04",
            what: "at feature scale 1000 two values 2x the relative term apart are different",
            run: |p| {
                !p.near(0.0, V1.rel * 1000.0 * 2.0, 1000.0) && p.near(0.0, V1.rel * 1000.0 * 0.5, 1000.0)
            },
        },
        PolicyCase {
            id: "tiny-radius",
            rule: "NP-07",
            what: "a radius of 2x the floor is a circle, a radius of half the floor is not",
            run: |p| {
                p.check_radius(V1.abs * 2.0, 0.0).is_ok()
                    && rejects(p.check_radius(V1.abs * 0.5, 0.0), NumericError::DegenerateRadius)
            },
        },
        PolicyCase {
            id: "radius-lost-in-center-rounding",
            rule: "NP-07",
            what: "a radius of 4 ULPs of its own center is degenerate",
            run: |p| {
                rejects(p.check_radius(4.0 * ulp_of(HALF_MAX), HALF_MAX), NumericError::DegenerateRadius)
            },
        },
        PolicyCase {
            id: "ellipse-axis-ratio",
            rule: "NP-07",
            what: "an axis ratio of 2x the conditioning bound is refused, half the bound is not",
            run: |p| {
                rejects(p.check_semi_axes(V1.axis_ratio_max * 2.0, 1.0, 0.0), NumericError::DegenerateRadius)
                    && p.check_semi_axes(V1.axis_ratio_max * 0.5, 1.0, 0.0).is_ok()
            },
        },
        PolicyCase {
            id: "coincident-control-points",
            rule: "NP-08",
            what: "control points that all coincide are a point, not a curve",
            run: |p| rejects(p.check_hull(&[[3.0, 4.0]; 5]), NumericError::DegenerateHull),
        },
        PolicyCase {
            id: "collinear-hull",
            rule: "NP-08",
            what: "a collinear control hull is legal and classified linear",
            run: |p| p.check_hull(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0], [3.0, 3.0]]) == Ok(HullClass::Linear),
        },
        PolicyCase {
            id: "nearly-collinear-hull",
            rule: "NP-08",
            what: "a hull bent by 5x the tolerance off a line is not collinear",
            run: |p| {
                let h = V1.tol(2.0, 2.0) * 5.0;
                p.check_hull(&[[0.0, 0.0], [1.0, 0.0], [2.0, h]]) == Ok(HullClass::Planar)
            },
        },
        PolicyCase {
            id: "orientation-band",
            rule: "NP-09",
            what: "a point 5x the tolerance off a line is on a side; 0.2x is on the line",
            run: |p| {
                let t = V1.tol(2.0, 2.0);
                p.orient2d([0.0, 0.0], [2.0, 0.0], [1.0, t * 5.0]) == Orientation::Ccw
                    && p.orient2d([0.0, 0.0], [2.0, 0.0], [1.0, -t * 5.0]) == Orientation::Cw
                    && p.orient2d([0.0, 0.0], [2.0, 0.0], [1.0, t * 0.2]) == Orientation::Collinear
            },
        },
        PolicyCase {
            id: "zero-direction",
            rule: "NP-10",
            what: "a direction of half the floor is zero, one of 4x the floor is a direction",
            run: |p| {
                rejects(p.normalise_direction(V1.abs * 0.5, 0.0), NumericError::ZeroDirection)
                    && p.normalise_direction(0.0, V1.abs * 4.0).is_ok_and(|d| p.is_unit(d))
            },
        },
        PolicyCase {
            id: "wrapped-arc",
            rule: "NP-11",
            what: "3pi/2 to pi/2 counter-clockwise is a half turn that wraps through zero",
            run: |p| {
                p.normalise_periodic_trim(1.5 * PI, 0.5 * PI, true, TAU)
                    .is_ok_and(|t| p.near(t.span, PI, TAU) && p.near(t.start, 1.5 * PI, TAU) && !t.reversed)
            },
        },
        PolicyCase {
            id: "full-circle",
            rule: "NP-11",
            what: "a trim of exactly one period is the full circle",
            run: |p| {
                p.normalise_periodic_trim(0.25, 0.25 + TAU, true, TAU).is_ok_and(|t| p.near(t.span, TAU, TAU))
            },
        },
        PolicyCase {
            id: "multi-turn",
            rule: "NP-11",
            what: "three periods is refused",
            run: |p| {
                rejects(p.normalise_periodic_trim(0.0, 3.0 * TAU, true, TAU), NumericError::PeriodicSpan)
            },
        },
        PolicyCase {
            id: "backward-full-turn",
            rule: "NP-11",
            what: "a full turn written backwards is ambiguous and refused",
            run: |p| rejects(p.normalise_periodic_trim(TAU, 0.0, true, TAU), NumericError::PeriodicSpan),
        },
        PolicyCase {
            id: "reversed-arc",
            rule: "NP-11",
            what: "sense false keeps the same interval and flags the reversal",
            run: |p| {
                p.normalise_periodic_trim(1.0, 0.5, false, TAU)
                    .is_ok_and(|t| t.reversed && p.near(t.start, 0.5, TAU) && p.near(t.span, 0.5, TAU))
            },
        },
        PolicyCase {
            id: "zero-length-trim",
            rule: "NP-12",
            what: "equal trim parameters collapse",
            run: |p| {
                rejects(p.normalise_periodic_trim(1.0, 1.0, true, TAU), NumericError::CollapsedTrim)
                    && rejects(p.normalise_trim(1.0, 1.0, true, 0.0, 2.0), NumericError::CollapsedTrim)
            },
        },
        PolicyCase {
            id: "near-zero-trim",
            rule: "NP-12",
            what: "a periodic trim of 2x the parameter tolerance is an arc",
            run: |p| {
                let tp = V1.tol(TAU, 1.0);
                p.normalise_periodic_trim(1.0, 1.0 + 2.0 * tp, true, TAU).is_ok()
            },
        },
        PolicyCase {
            id: "inverted-trim",
            rule: "NP-12",
            what: "trim1 after trim2 with sense true on an open basis is inconsistent",
            run: |p| rejects(p.normalise_trim(1.5, 0.5, true, 0.0, 2.0), NumericError::InconsistentSense),
        },
        PolicyCase {
            id: "trim-outside-domain",
            rule: "NP-12",
            what: "a trim beyond the basis domain is refused",
            run: |p| rejects(p.normalise_trim(0.5, 3.0, true, 0.0, 2.0), NumericError::TrimOutsideDomain),
        },
        PolicyCase {
            id: "empty-domain",
            rule: "NP-13",
            what: "a domain narrower than the floor has no width",
            run: |p| {
                rejects(p.unit_param(0.0, 0.0, V1.abs * 0.5), NumericError::EmptyDomain)
                    && p.unit_param(1.0, 0.0, 4.0).is_ok_and(|u| u == 0.25)
            },
        },
        PolicyCase {
            id: "full-multiplicity-interior-knot",
            rule: "NP-14",
            what: "an interior knot of multiplicity = degree is a legal C0 break; degree+1 is refused",
            run: |p| {
                let ok = [0., 0., 0., 0., 0.5, 0.5, 0.5, 1., 1., 1., 1.];
                let bad = [0., 0., 0., 0., 0.5, 0.5, 0.5, 0.5, 1., 1., 1., 1.];
                p.validate_knots(3, 7, &ok) == Ok((0.0, 1.0))
                    && rejects(p.validate_knots(3, 8, &bad), NumericError::BadKnots)
            },
        },
        PolicyCase {
            id: "near-repeated-knots",
            rule: "NP-14",
            what: "two knots 2x the parameter tolerance apart are distinct; 0.5x apart are ambiguous",
            run: |p| {
                let tp = V1.tol(1.0, 1.0);
                let apart = |d: f64| [0., 0., 0., 0., 0.5, 0.5 + d, 1., 1., 1., 1.];
                p.validate_knots(3, 6, &apart(2.0 * tp)).is_ok()
                    && rejects(p.validate_knots(3, 6, &apart(0.5 * tp)), NumericError::AmbiguousKnots)
            },
        },
        PolicyCase {
            id: "decreasing-knots",
            rule: "NP-14",
            what: "a decreasing knot vector is refused",
            run: |p| rejects(p.validate_knots(2, 4, &[0., 0., 0., 1., 0.5, 1., 1.]), NumericError::BadKnots),
        },
        PolicyCase {
            id: "weight-near-zero",
            rule: "NP-15",
            what: "a weight 2x the floor is accepted, 0.5x the floor is refused, zero and negative are refused",
            run: |p| {
                p.validate_weights(&[1.0, V1.weight_floor * 2.0]).is_ok()
                    && rejects(p.validate_weights(&[1.0, V1.weight_floor * 0.5]), NumericError::BadWeight)
                    && rejects(p.validate_weights(&[1.0, 0.0]), NumericError::BadWeight)
                    && rejects(p.validate_weights(&[1.0, -1.0]), NumericError::BadWeight)
            },
        },
        PolicyCase {
            id: "two-sided-derivative-at-full-multiplicity",
            rule: "NP-16",
            what: "the two-sided first derivative at a knot of multiplicity = degree does not exist",
            run: |_| {
                rejects(check_derivative_request(1, 3, 3, Side::Both), NumericError::TwoSidedAtBreak)
                    && check_derivative_request(0, 3, 3, Side::Both).is_ok()
                    && check_derivative_request(1, 3, 3, Side::Left).is_ok()
                    && check_derivative_request(1, 3, 3, Side::Right).is_ok()
                    && check_derivative_request(2, 3, 1, Side::Both).is_ok()
                    && rejects(check_derivative_request(3, 3, 1, Side::Both), NumericError::TwoSidedAtBreak)
            },
        },
        PolicyCase {
            id: "composite-gap",
            rule: "NP-17",
            what: "a joint 0.5x the tolerance apart joins; 5x does not",
            run: |p| {
                let t = V1.tol(1.0, 1.0);
                p.check_join([1.0, 0.0], [1.0 + 0.5 * t, 0.0], 1.0).is_ok()
                    && rejects(p.check_join([1.0, 0.0], [1.0 + 5.0 * t, 0.0], 1.0), NumericError::Gap)
            },
        },
        PolicyCase {
            id: "limits-at-the-edge",
            rule: "NP-18",
            what: "every maximum is accepted and the next count refused",
            run: |_| {
                [
                    MAX_ENTITIES,
                    MAX_CONTROL_POINTS,
                    MAX_KNOTS,
                    MAX_POLYLINE_VERTICES,
                    MAX_COMPOSITE_SEGMENTS,
                    MAX_NESTING_DEPTH,
                    MAX_DEGREE,
                ]
                .iter()
                .all(|&m| check_count(m, m).is_ok() && rejects(check_count(m + 1, m), NumericError::Limit))
            },
        },
        PolicyCase {
            id: "near-singular-transform",
            rule: "NP-19",
            what: "a transform whose determinant is 4x the relative term is invertible; 0.4x is singular",
            run: |p| {
                let ok = [1.0, 1.0, 1.0, 1.0 + 4.0 * V1.rel, 0.0, 0.0];
                let bad = [1.0, 1.0, 1.0, 1.0 + 0.4 * V1.rel, 0.0, 0.0];
                p.validate_affine(ok).is_ok()
                    && rejects(p.validate_affine(bad), NumericError::SingularTransform)
            },
        },
        PolicyCase {
            id: "closed-flag-gap",
            rule: "NP-20",
            what: "a curve flagged closed whose ends are 5x the tolerance apart is refused",
            run: |p| {
                let t = V1.tol(1.0, 1.0);
                p.check_closed_flag(true, [0.0, 0.0], [5.0 * t, 0.0], 1.0)
                    == Err(NumericError::ClosedMismatch)
                    && p.check_closed_flag(true, [0.0, 0.0], [0.5 * t, 0.0], 1.0).is_ok()
            },
        },
    ]
}
