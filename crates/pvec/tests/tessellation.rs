//! Issue #53: one exact-evaluation entry point and one adaptive tessellation entry point, with the
//! whole transform chain and an explicit error budget, and a result that says it is for display.

use pvec::{
    Bounds2, Conformance, Curve, CurveEval, PvecError, Side, Tessellate, Tessellation, TessellationContext,
    Transform2,
};
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline, Trimmed,
    Vector2,
};

fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn frame(x: f64, y: f64) -> Placement2 {
    Placement2::new(p(x, y), Vector2::new(1.0, 0.0)).unwrap()
}

fn bspline(points: &[(f64, f64)], degree: usize) -> BSpline {
    let n = points.len();
    let inner = n - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..=inner).map(|i| i as f64 / (inner + 1) as f64));
    knots.extend(vec![1.0; degree + 1]);
    BSpline::new(degree, points.iter().map(|(x, y)| p(*x, *y)).collect(), knots).unwrap()
}

/// One curve of each of the nine kinds the profile has.
fn profile() -> Vec<(&'static str, Curve)> {
    let circle = Curve::Circle(Circle::new(frame(5.0, 5.0), 5.0).unwrap());
    let ellipse = Curve::Ellipse(Ellipse::new(frame(5.0, 5.0), 6.0, 2.0).unwrap());
    let line = Line::new(p(0.0, 0.0), Vector2::new(3.0, 1.0)).unwrap();
    vec![
        ("line", Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, 10.0, true).unwrap())),
        (
            "polyline",
            Curve::Polyline(
                Polyline::new(vec![p(0.0, 0.0), p(3.0, 4.0), p(6.0, 0.5), p(9.0, 7.0)]).unwrap(),
            ),
        ),
        ("circle", circle.clone()),
        ("ellipse", ellipse),
        ("trimmed", Curve::trimmed_conic(circle, 0.3, 4.0, true).unwrap()),
        (
            "composite",
            Curve::Composite(
                Composite::new(vec![
                    Curve::Bezier(Bezier::new(vec![p(0.0, 0.0), p(1.0, 3.0), p(2.0, 3.0), p(3.0, 0.0)]).unwrap()),
                    Curve::BSpline(bspline(&[(3.0, 0.0), (4.0, -3.0), (6.0, -2.0), (8.0, 1.0), (9.0, 0.0)], 3)),
                ])
                .unwrap(),
            ),
        ),
        (
            "bezier",
            Curve::Bezier(Bezier::new(vec![p(0.0, 0.0), p(2.0, 9.0), p(7.0, -4.0), p(10.0, 5.0)]).unwrap()),
        ),
        (
            "bspline",
            Curve::BSpline(bspline(&[(0.0, 0.0), (1.0, 5.0), (3.0, 7.0), (5.0, 1.0), (8.0, 6.0), (10.0, 0.0)], 3)),
        ),
        (
            "rational",
            Curve::RationalBSpline(
                RationalBSpline::new(
                    2,
                    vec![p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)],
                    vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                    vec![1.0, std::f64::consts::FRAC_1_SQRT_2, 1.0],
                )
                .unwrap(),
            ),
        ),
    ]
}

fn scale(s: f64) -> Transform2 {
    Transform2::scale(s, s).unwrap()
}

fn context(zoom: f64) -> TessellationContext {
    TessellationContext {
        source_to_map: Transform2::IDENTITY,
        map_to_screen: scale(zoom),
        pixel_ratio: 1.0,
        max_screen_error_px: 0.25,
        max_world_error: f64::INFINITY,
        max_segments: 200_000,
    }
}

/// Where a source point is on screen, in device pixels, after the whole chain.
fn on_screen(ctx: &TessellationContext, q: [f64; 2]) -> [f64; 2] {
    let m = ctx.source_to_map.apply_point(p(q[0], q[1]));
    let s = ctx.map_to_screen.apply_point(m);
    [s.x * ctx.pixel_ratio, s.y * ctx.pixel_ratio]
}

fn distance_to_segment(q: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 { 0.0 } else { (((q[0] - a[0]) * dx + (q[1] - a[1]) * dy) / len2).clamp(0.0, 1.0) };
    ((q[0] - a[0] - t * dx).hypot(q[1] - a[1] - t * dy)).abs()
}

/// The largest distance, on screen, from the exact curve to the polyline, found by a dense sweep that is
/// independent of the tessellator's own samples.
fn deviation_px(curve: &Curve, ctx: &TessellationContext, t: &Tessellation) -> f64 {
    let screen: Vec<[f64; 2]> = t.vertices.iter().map(|v| on_screen(ctx, *v)).collect();
    let d = curve.domain();
    let n = 4000;
    (0..=n)
        .map(|i| {
            let u = d.lo() + d.width() * i as f64 / n as f64;
            let q = on_screen(ctx, curve.point_at(u).unwrap());
            screen.windows(2).map(|w| distance_to_segment(q, w[0], w[1])).fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
}

#[test]
fn issue_53_every_profile_kind_stays_inside_the_screen_budget_at_three_zoom_levels() {
    for (name, curve) in profile() {
        for zoom in [1.0, 32.0, 1024.0] {
            let ctx = context(zoom);
            let t = curve.tessellate(&ctx).unwrap_or_else(|e| panic!("{name} at {zoom}: {e}"));
            let worst = deviation_px(&curve, &ctx, &t);
            assert!(
                worst <= ctx.max_screen_error_px,
                "{name} at zoom {zoom}: deviation {worst} px, budget {}",
                ctx.max_screen_error_px
            );
            assert!(t.achieved_screen_error_px <= ctx.max_screen_error_px, "{name} at {zoom}: claim");
            assert!(t.vertices.len() >= 2, "{name} at {zoom}");
            assert!(t.vertices.len() - 1 <= ctx.max_segments);
        }
    }
}

#[test]
fn issue_53_the_pixel_ratio_scales_the_budget_to_device_pixels() {
    let (_, curve) = profile().remove(2);
    let ctx = TessellationContext { pixel_ratio: 2.0, ..context(32.0) };
    let t = curve.tessellate(&ctx).unwrap();
    assert!(deviation_px(&curve, &ctx, &t) <= ctx.max_screen_error_px);
    let coarse = curve.tessellate(&TessellationContext { pixel_ratio: 1.0, ..ctx }).unwrap();
    assert!(t.vertices.len() > coarse.vertices.len(), "twice the device pixels need more segments");
}

#[test]
fn issue_53_a_world_budget_holds_when_the_screen_budget_is_loose() {
    let (_, curve) = profile().remove(2);
    let ctx = TessellationContext { max_screen_error_px: 1000.0, max_world_error: 0.001, ..context(1.0) };
    let t = curve.tessellate(&ctx).unwrap();
    assert!(t.achieved_world_error <= 0.001);
    let world = TessellationContext { map_to_screen: Transform2::IDENTITY, pixel_ratio: 1.0, ..ctx };
    assert!(deviation_px(&curve, &world, &t) <= 0.001);
}

#[test]
fn issue_53_the_conformance_tag_admits_only_display_fidelity() {
    let (_, curve) = profile().remove(2);
    let t = curve.tessellate(&context(1.0)).unwrap();
    // An exhaustive match with one arm compiles only while the enum has exactly one value.
    match t.conformance {
        Conformance::DisplayFidelity => {}
    }
    let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/public-api.txt")).unwrap();
    let variants: Vec<&str> = text.lines().filter(|l| l.contains("Conformance::")).collect();
    assert_eq!(variants.len(), 1, "{variants:?}");
    assert!(variants[0].contains("DisplayFidelity"));
}

#[test]
fn issue_53_four_segments_for_a_200_knot_spline_is_refused_not_coarsened() {
    let points: Vec<(f64, f64)> = (0..204).map(|i| (i as f64, if i % 2 == 0 { 0.0 } else { 3.0 })).collect();
    let spline = Curve::BSpline(bspline(&points, 3));
    let ctx = TessellationContext { max_segments: 4, ..context(1.0) };
    match spline.tessellate(&ctx) {
        Err(PvecError::ErrorBudgetUnmet { achieved, requested }) => {
            assert_eq!(requested, 0.25);
            assert!(achieved > requested, "achieved {achieved}");
        }
        other => panic!("expected ErrorBudgetUnmet, got {other:?}"),
    }
}

#[test]
fn issue_53_evaluation_is_bit_identical_before_and_after_tessellation() {
    for (name, curve) in profile() {
        let d = curve.domain();
        let ts: Vec<f64> = (0..=50).map(|i| d.lo() + d.width() * i as f64 / 50.0).collect();
        let at = |c: &Curve| -> Vec<[u64; 2]> {
            ts.iter().map(|t| c.point_at(*t).unwrap().map(f64::to_bits)).collect()
        };
        let before = at(&curve);
        curve.tessellate(&context(32.0)).unwrap();
        curve.tessellate(&context(1024.0)).unwrap();
        assert_eq!(before, at(&curve), "{name}");
    }
}

#[test]
fn issue_53_a_million_unit_offset_in_the_source_is_measured_after_the_chain() {
    let spline = bspline(&[(1e6, 1e6), (1e6 + 1.0, 1e6 + 5.0), (1e6 + 3.0, 1e6 + 7.0), (1e6 + 5.0, 1e6 + 1.0), (1e6 + 8.0, 1e6 + 6.0)], 3);
    let curve = Curve::BSpline(spline);
    let ctx = TessellationContext {
        source_to_map: Transform2::translate(-1e6, -1e6).unwrap(),
        map_to_screen: scale(256.0),
        ..context(1.0)
    };
    let t = curve.tessellate(&ctx).unwrap();
    assert!(deviation_px(&curve, &ctx, &t) <= ctx.max_screen_error_px);
    // The same budget measured in source space would be met by a handful of segments at this zoom.
    assert!(t.vertices.len() > 8);
}

#[test]
fn issue_53_the_breaks_of_a_curve_are_vertices() {
    let poly = Polyline::new(vec![p(0.0, 0.0), p(4.0, 3.0), p(8.0, 0.0)]).unwrap();
    let t = Curve::Polyline(poly).tessellate(&context(1.0)).unwrap();
    assert_eq!(t.vertices, vec![[0.0, 0.0], [4.0, 3.0], [8.0, 0.0]]);
    assert_eq!(t.achieved_screen_error_px, 0.0);
}

#[test]
fn issue_53_a_context_that_cannot_be_used_is_refused() {
    let (_, curve) = profile().remove(2);
    for bad in [
        TessellationContext { max_screen_error_px: 0.0, ..context(1.0) },
        TessellationContext { max_screen_error_px: f64::NAN, ..context(1.0) },
        TessellationContext { max_world_error: -1.0, ..context(1.0) },
        TessellationContext { pixel_ratio: 0.0, ..context(1.0) },
        TessellationContext { max_segments: 0, ..context(1.0) },
    ] {
        assert!(matches!(curve.tessellate(&bad), Err(PvecError::BadContext { .. })), "{bad:?}");
    }
}

#[test]
fn issue_53_an_unbounded_line_has_no_tessellation_and_no_bounds() {
    let line = Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    let unbounded = |e: Result<_, PvecError>| matches!(e, Err::<(), _>(PvecError::Geometry { rule }) if rule == "PROFILE-4");
    assert!(unbounded(line.tessellate(&context(1.0)).map(|_| ())));
    assert!(unbounded(line.bounds().map(|_| ())));
}

#[test]
fn issue_53_exact_evaluation_answers_points_derivatives_bounds_closure_and_images() {
    let circle = Curve::Circle(Circle::new(frame(5.0, 5.0), 5.0).unwrap());
    let d = circle.domain();
    assert_eq!((d.lo(), d.hi()), (0.0, std::f64::consts::TAU));
    assert_eq!(circle.point_at(0.0).unwrap(), [10.0, 5.0]);
    let v = circle.derivative(0.0, 1).unwrap();
    assert_eq!(v[0], 0.0);
    assert_eq!(v[1], 5.0);
    assert!(circle.is_closed());
    let b: Bounds2 = circle.bounds().unwrap();
    assert!(b.min[0] <= 0.0 && b.max[0] >= 10.0);
    let moved = circle.transformed(&Transform2::translate(1.0, 2.0).unwrap()).unwrap();
    assert_eq!(moved.point_at(0.0).unwrap(), [11.0, 7.0]);

    let open = &profile()[7].1;
    assert!(!open.is_closed());
    let poly = Curve::Polyline(Polyline::new(vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.0)]).unwrap());
    assert!(matches!(poly.derivative(1.0, 1), Err(PvecError::Geometry { .. })), "a corner has no two-sided derivative");
    assert_eq!(poly.derivative_from(1.0, 1, Side::Right).unwrap(), [1.0, -1.0]);
    assert!(matches!(circle.point_at(100.0), Err(PvecError::Geometry { .. })));
}
