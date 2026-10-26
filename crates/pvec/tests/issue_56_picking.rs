//! Issue #56 (P7.5): spatial query and picking over stable entity ids. Each test is named for the
//! acceptance criterion it holds.
#![allow(missing_docs)]

use pvec::{
    Bounds2, CapabilityId, Curve, EntityId, EntityIdBatch, Manifest, Pick, PickIndex, PickTolerance, Point2,
    PvecReader, PvecWriter, RectMode, Transform2,
};
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve2d, Ellipse, Line, Placement2, Polyline, RationalBSpline,
    Trimmed, Vector2,
};
use pvec_numeric::Policy;

// ------------------------------------------------------------------ fixtures

/// The policy tolerance (NP-04) at a length scale.
fn np_tol(scale: f64) -> f64 {
    Policy::V1.tol(scale, scale)
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}
fn pt(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

/// A valid file of `n` placeholder polylines. The facade reader cannot decode curves yet, so the file
/// supplies the manifest's entity count and the tests hand the index the curves themselves.
fn reader(n: u32) -> PvecReader {
    let mut w = PvecWriter::new(Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [-1000.0, -1000.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-56".into(),
        producer_version: "1".into(),
    })
    .unwrap();
    for i in 0..n {
        let f = f64::from(i);
        let c = Curve::Polyline(Polyline::new(vec![pt(f, 0.0), pt(f, 1.0)]).unwrap());
        w.push(eid(i), &c).unwrap();
    }
    PvecReader::open(w.finish().unwrap()).unwrap()
}

fn segment(a: Point2, b: Point2) -> Curve {
    let len = (b.x - a.x).hypot(b.y - a.y);
    let line = Line::new(a, Vector2::new(b.x - a.x, b.y - a.y)).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, len, true).unwrap())
}

fn placement(x: f64, y: f64) -> Placement2 {
    Placement2::new(pt(x, y), Vector2::new(1.0, 0.0)).unwrap()
}

fn circle(x: f64, y: f64, r: f64) -> Curve {
    Curve::Circle(Circle::new(placement(x, y), r).unwrap())
}

fn arc(x: f64, y: f64, r: f64, t1: f64, t2: f64) -> Curve {
    Curve::Trimmed(Trimmed::new(circle(x, y, r), t1, t2, true).unwrap())
}

fn ellipse(x: f64, y: f64, a: f64, b: f64) -> Curve {
    Curve::Ellipse(Ellipse::new(placement(x, y), a, b).unwrap())
}

fn cubic_bezier() -> Curve {
    Curve::Bezier(Bezier::new(vec![pt(0.0, 0.0), pt(30.0, 80.0), pt(70.0, -60.0), pt(100.0, 10.0)]).unwrap())
}

fn cubic_bspline() -> Curve {
    let cps = vec![pt(0.0, 0.0), pt(20.0, 50.0), pt(50.0, -30.0), pt(80.0, 40.0), pt(120.0, 0.0)];
    Curve::BSpline(BSpline::new(3, cps, vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0]).unwrap())
}

/// A hairpin: a quadratic rational spline whose apex has a radius of curvature far below one pixel.
fn hairpin() -> Curve {
    Curve::RationalBSpline(
        RationalBSpline::new(
            2,
            vec![pt(0.0, 0.0), pt(4000.0, 2.0), pt(0.0, 4.0)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            vec![1.0, 1.5, 1.0],
        )
        .unwrap(),
    )
}

fn composite() -> Curve {
    let segs = vec![
        segment(pt(0.0, 0.0), pt(10.0, 0.0)),
        Curve::Polyline(Polyline::new(vec![pt(10.0, 0.0), pt(10.0, 10.0), pt(0.0, 10.0)]).unwrap()),
        Curve::Bezier(Bezier::new(vec![pt(0.0, 10.0), pt(-8.0, 5.0), pt(0.0, 0.0)]).unwrap()),
    ];
    Curve::Composite(Composite::new(segs).unwrap())
}

fn every_kind() -> Vec<(&'static str, Curve)> {
    vec![
        ("segment", segment(pt(0.0, 0.0), pt(100.0, 30.0))),
        (
            "polyline",
            Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(50.0, 20.0), pt(90.0, -10.0)]).unwrap()),
        ),
        ("circle", circle(10.0, 5.0, 40.0)),
        ("arc", arc(10.0, 5.0, 40.0, 0.4, 2.2)),
        ("ellipse", ellipse(0.0, 0.0, 60.0, 15.0)),
        (
            "trimmed ellipse",
            Curve::Trimmed(Trimmed::new(ellipse(0.0, 0.0, 60.0, 15.0), 0.3, 2.0, true).unwrap()),
        ),
        ("bezier", cubic_bezier()),
        ("bspline", cubic_bspline()),
        ("rational hairpin", hairpin()),
        ("composite", composite()),
    ]
}

fn index_of(curves: &[Curve]) -> PickIndex {
    let r = reader(u32::try_from(curves.len()).unwrap());
    let mut ix = PickIndex::new(&r).unwrap();
    for (i, c) in curves.iter().enumerate() {
        ix.insert_exact(eid(u32::try_from(i).unwrap()), c.clone()).unwrap();
    }
    ix
}

fn scale(s: f64) -> Transform2 {
    Transform2::scale(s, s).unwrap()
}

/// Reference nearest distance: dense sampling, then a ternary search around the best samples. Slow
/// and independent of the code under test.
fn reference_distance(c: &Curve, p: Point2) -> f64 {
    let d = c.domain();
    let n = 20_000;
    let dist = |t: f64| {
        let q = c.eval(t).unwrap();
        (q.x - p.x).hypot(q.y - p.y)
    };
    let step = d.width() / f64::from(n);
    let mut samples: Vec<(f64, f64)> =
        (0..=n).map(|i| (d.lo() + step * f64::from(i), 0.0)).map(|(t, _)| (t, dist(t))).collect();
    samples.sort_by(|a, b| a.1.total_cmp(&b.1));
    samples
        .iter()
        .take(8)
        .map(|&(t, _)| {
            let (mut lo, mut hi) = ((t - step).max(d.lo()), (t + step).min(d.hi()));
            for _ in 0..80 {
                let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
                if dist(m1) < dist(m2) { hi = m2 } else { lo = m1 }
            }
            dist((lo + hi) / 2.0)
        })
        .fold(f64::INFINITY, f64::min)
}

// ------------------------------------------------------------------ tolerance

#[test]
fn issue_56_a_pick_radius_in_pixels_converts_through_the_transform() {
    let t = PickTolerance::new(1.0, &scale(4.0)).unwrap();
    assert!((t.source_radius() - 0.25).abs() < np_tol(1.0));
    assert_eq!(t.px(), 1.0);
    assert!((t.scale() - 4.0).abs() < np_tol(4.0));
    // A rotation and a translation do not change the scale.
    let m = Transform2::rotate(0.7)
        .unwrap()
        .then(&scale(2.0))
        .unwrap()
        .then(&Transform2::translate(9.0, -3.0).unwrap())
        .unwrap();
    assert!((PickTolerance::new(3.0, &m).unwrap().source_radius() - 1.5).abs() < np_tol(1.5));
}

#[test]
fn issue_56_an_anisotropic_transform_uses_the_weakest_axis_so_no_screen_hit_is_lost() {
    let m = Transform2::scale(8.0, 2.0).unwrap();
    assert!((PickTolerance::new(1.0, &m).unwrap().source_radius() - 0.5).abs() < np_tol(0.5));
}

#[test]
fn issue_56_a_bad_radius_or_a_singular_transform_is_refused_with_a_rule() {
    for px in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(matches!(PickTolerance::new(px, &scale(1.0)), Err(pvec::PvecError::Query { .. })));
    }
    let singular = Transform2::new([1.0, 2.0, 2.0, 4.0, 0.0, 0.0]);
    if let Ok(m) = singular {
        assert!(PickTolerance::new(1.0, &m).is_err());
    }
}

// ------------------------------------------------------------------ AC 1: 0.4 px

#[test]
fn issue_56_a_point_0_4_px_from_a_line_hits_at_1_px_and_misses_at_0_25_px() {
    // Zoom 2: 0.2 source units are 0.4 px.
    let ix = index_of(&[segment(pt(0.0, 0.0), pt(100.0, 0.0)), segment(pt(0.0, 50.0), pt(100.0, 50.0))]);
    let zoom = scale(2.0);
    let p = pt(40.0, 0.2);
    let hit = ix.query_point(p, PickTolerance::new(1.0, &zoom).unwrap()).expect("0.4 px is inside 1.0 px");
    assert_eq!(hit.entity, eid(0));
    assert!((hit.distance - 0.2).abs() < np_tol(1.0), "distance {}", hit.distance);
    assert!(
        ix.query_point(p, PickTolerance::new(0.25, &zoom).unwrap()).is_none(),
        "0.4 px is outside 0.25 px"
    );
}

#[test]
fn issue_56_the_same_source_gap_flips_with_zoom_not_with_a_world_guess() {
    let ix = index_of(&[segment(pt(0.0, 0.0), pt(100.0, 0.0))]);
    let p = pt(40.0, 0.2);
    assert!(ix.query_point(p, PickTolerance::new(1.0, &scale(2.0)).unwrap()).is_some());
    assert!(
        ix.query_point(p, PickTolerance::new(1.0, &scale(0.5)).unwrap()).is_some(),
        "zoomed out, a pixel is 2 units"
    );
    assert!(
        ix.query_point(p, PickTolerance::new(1.0, &scale(1024.0)).unwrap()).is_none(),
        "zoomed in, a pixel is 0.001 units"
    );
}

// ------------------------------------------------------------------ analytic hits for every kind

#[test]
fn issue_56_every_curve_kind_is_hit_tested_analytically_against_a_reference() {
    for (name, c) in every_kind() {
        let b = c.bounds().unwrap();
        let (lo, hi) = (b.min(), b.max());
        let (w, h) = (hi.x - lo.x, hi.y - lo.y);
        let ix = index_of(std::slice::from_ref(&c));
        for k in 0..12 {
            // Points on a ring around the curve's box, some inside and some outside.
            let a = f64::from(k) * std::f64::consts::FRAC_PI_6;
            let p = pt(lo.x + w * (0.5 + 0.7 * a.cos()), lo.y + h * (0.5 + 0.7 * a.sin()));
            let want = reference_distance(&c, p);
            let got =
                ix.query_nearest(p, f64::INFINITY).unwrap_or_else(|| panic!("{name}: no nearest at {p:?}"));
            let limit = np_tol(w + h);
            assert!(
                (got.distance - want).abs() <= limit,
                "{name} at {p:?}: {} against reference {want}",
                got.distance
            );
        }
    }
}

#[test]
fn issue_56_known_truth_distances_for_the_conics() {
    let ix = index_of(&[circle(10.0, 5.0, 7.0)]);
    let on_ring = ix.query_nearest(pt(20.0, 5.0), 100.0).unwrap();
    assert_eq!(on_ring.entity, eid(0));
    assert!((on_ring.distance - 3.0).abs() < np_tol(3.0));
    assert!(
        on_ring.param.unwrap().abs() < np_tol(1.0)
            || (on_ring.param.unwrap() - std::f64::consts::TAU).abs() < np_tol(1.0)
    );
    // Below the arc's start: the nearest point of the arc is its end point (10, 0), not the full circle.
    let ix = index_of(&[arc(0.0, 0.0, 10.0, 0.0, std::f64::consts::FRAC_PI_2)]);
    let off_arc = ix.query_nearest(pt(10.0, -4.0), 5.0).unwrap();
    assert_eq!(off_arc.entity, eid(0));
    assert!((off_arc.distance - 4.0).abs() < np_tol(4.0));
    assert!(off_arc.param.unwrap().abs() < np_tol(1.0));
}

#[test]
fn issue_56_pick_carries_source_bounds_and_the_capability_of_the_curve() {
    let ix = index_of(&[segment(pt(0.0, 0.0), pt(100.0, 0.0))]);
    let a = ix.query_nearest(pt(10.0, 0.1), 1.0).unwrap();
    assert_eq!(a.capability, CapabilityId::new(8), "trimmed curve");
    assert_eq!(a.bounds, Bounds2 { min: [0.0, 0.0], max: [100.0, 0.0] });
    let b = hairpin().bounds().unwrap();
    let ix = index_of(&[hairpin()]);
    let h = ix.query_nearest(pt(100.0, 1.0), 1000.0).unwrap();
    assert_eq!(h.entity, eid(0));
    assert_eq!(h.capability, CapabilityId::new(12), "rational B-spline");
    assert_eq!(h.bounds, Bounds2 { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] });
}

// ------------------------------------------------------------------ AC 2: identity across refinement

#[test]
fn issue_56_the_id_of_a_preview_equals_the_id_of_its_exact_replacement() {
    let curve = segment(pt(0.0, 0.0), pt(100.0, 0.0));
    let r = reader(3);
    let mut ix = PickIndex::new(&r).unwrap();
    ix.insert_exact(eid(0), segment(pt(0.0, 20.0), pt(100.0, 20.0))).unwrap();
    let bounds = Bounds2 { min: [0.0, 0.0], max: [100.0, 0.0] };
    let cap = CapabilityId::new(8);
    ix.insert_preview(eid(1), cap, bounds, vec![[0.0, 0.3], [100.0, 0.3]], 0.5).unwrap();
    ix.insert_exact(eid(2), segment(pt(0.0, -20.0), pt(100.0, -20.0))).unwrap();
    let tol = PickTolerance::new(1.0, &scale(1.0)).unwrap();
    let before = ix.query_point(pt(50.0, 0.0), tol).unwrap();
    assert_eq!(before.entity, eid(1));
    assert_eq!(before.param, None, "a preview has no exact parameter");
    let refined = ix.refine(eid(1), curve).unwrap();
    assert_eq!(refined, eid(1));
    let after = ix.query_point(pt(50.0, 0.0), tol).unwrap();
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.bounds, before.bounds, "bounds do not depend on the representation");
    assert_eq!(after.capability, before.capability);
    assert!((after.param.unwrap() - 50.0).abs() < np_tol(50.0));
}

#[test]
fn issue_56_the_id_survives_a_p5_6_refinement_generation_change() {
    use pvec_stream::{GenerationClock, Scene, Tier0Params, Tier0Plan};
    // Entity 0 is a Bezier curve, which Tier 0 sends as a preview; the lines around it go out exact.
    let exact = cubic_bezier();
    let all: Vec<Curve> = std::iter::once(exact.clone())
        .chain((1..4).map(|i| segment(pt(0.0, 100.0 * f64::from(i)), pt(900.0, 100.0 * f64::from(i)))))
        .collect();
    let plan = Tier0Plan::build(
        4,
        all.iter().cloned().enumerate().map(|(i, c)| (eid(i as u32), c)),
        Tier0Params { max_error: 0.5 },
    )
    .unwrap();
    let mut scene = Scene::from_plan(&plan);
    let preview = plan
        .previews()
        .iter()
        .find(|p| p.replaces() == eid(0))
        .expect("a Bezier curve is a preview in Tier 0");
    let r = reader(4);
    let mut ix = PickIndex::new(&r).unwrap();
    for (i, c) in all.iter().enumerate().skip(1) {
        ix.insert_exact(eid(i as u32), c.clone()).unwrap();
    }
    let b = exact.bounds().unwrap();
    let bounds = Bounds2 { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] };
    ix.insert_preview(eid(0), CapabilityId::new(10), bounds, preview.points(), preview.error_bound())
        .unwrap();
    let tol = PickTolerance::new(2.0, &scale(1.0)).unwrap();
    let on = exact.eval(0.4).unwrap();
    let p = pt(on.x, on.y + 0.3);
    let before = ix.query_point(p, tol).expect("the preview is pickable");
    assert_eq!(before.entity, eid(0));
    assert!(before.param.is_none(), "the fixture must start from a preview");
    // The viewport moves (a new generation), and the exact geometry arrives for the same entity.
    let clock = GenerationClock::new();
    scene.advance(clock.advance().generation());
    scene.refine(eid(0), &exact).unwrap();
    ix.refine(eid(0), exact.clone()).unwrap();
    let after = ix.query_point(p, tol).unwrap();
    assert_eq!(after.entity, before.entity);
    assert_eq!(after.bounds, before.bounds);
    let t = after.param.expect("exact geometry reports a parameter");
    assert!((t - 0.4).abs() < 0.05, "param {t}");
}

// ------------------------------------------------------------------ AC 3: zoom independence

#[test]
fn issue_56_query_rect_is_zoom_independent_over_ten_octaves() {
    let curves: Vec<Curve> =
        (0..20).map(|i| segment(pt(f64::from(i) * 10.0, 0.0), pt(f64::from(i) * 10.0 + 8.0, 5.0))).collect();
    let ix = index_of(&curves);
    let rect = Bounds2 { min: [25.0, -1.0], max: [93.0, 6.0] };
    let baseline: Vec<EntityIdBatch> =
        [RectMode::Intersects, RectMode::Contains].iter().map(|m| ix.query_rect(rect, *m)).collect();
    for s in [1.0, 32.0, 1024.0] {
        // Picking at any display scale in between must not change what the rectangle selects.
        let tol = PickTolerance::new(4.0, &scale(s)).unwrap();
        let _ = ix.query_point(pt(30.0, 1.0), tol);
        for (m, base) in [RectMode::Intersects, RectMode::Contains].iter().zip(&baseline) {
            assert_eq!(&ix.query_rect(rect, *m), base, "scale {s}");
        }
    }
    assert_eq!(baseline[0].as_slice(), &[2, 3, 4, 5, 6, 7, 8, 9]);
    assert_eq!(baseline[1].as_slice(), &[3, 4, 5, 6, 7, 8, 9][..6].to_vec()[..]);
}

// ------------------------------------------------------------------ AC 4: param round trip

#[test]
fn issue_56_param_round_trips_to_the_projection_of_the_pick_point() {
    for (name, c) in every_kind() {
        let b = c.bounds().unwrap();
        let scale_len = (b.max().x - b.min().x).hypot(b.max().y - b.min().y);
        let ix = index_of(std::slice::from_ref(&c));
        for k in 0..9 {
            let a = f64::from(k) * 0.7;
            let p = pt(
                b.min().x + scale_len * (0.4 + 0.5 * a.cos()),
                b.min().y + scale_len * (0.2 + 0.4 * a.sin()),
            );
            let got = ix.query_nearest(p, f64::INFINITY).unwrap();
            let t = got.param.expect("exact geometry reports a parameter");
            let d = c.domain();
            assert!(
                t >= d.lo() - np_tol(d.hi()) && t <= d.hi() + np_tol(d.hi()),
                "{name}: param {t} outside {d:?}"
            );
            let q = c.eval(t.clamp(d.lo(), d.hi())).unwrap();
            let on_curve = (q.x - p.x).hypot(q.y - p.y);
            assert!(
                (on_curve - got.distance).abs() <= np_tol(scale_len),
                "{name}: point_at(param) is {on_curve} away, pick said {}",
                got.distance
            );
        }
    }
}

// ------------------------------------------------------------------ batch layout

#[test]
fn issue_56_the_batch_is_a_flat_ascending_u32_array() {
    let curves: Vec<Curve> = (0..6).map(|i| segment(pt(0.0, f64::from(i)), pt(10.0, f64::from(i)))).collect();
    let ix = index_of(&curves);
    let batch = ix.query_rect(Bounds2 { min: [-1.0, 1.5], max: [11.0, 4.5] }, RectMode::Intersects);
    let flat: &[u32] = batch.as_slice();
    assert_eq!(flat, &[2, 3, 4]);
    assert_eq!(batch.len(), 3);
    assert!(!batch.is_empty());
    assert_eq!(std::mem::size_of_val(&flat[0]), 4, "one u32 per entity and no per-entity object");
    assert!(
        ix.query_rect(Bounds2 { min: [500.0, 500.0], max: [600.0, 600.0] }, RectMode::Intersects).is_empty()
    );
}

// ------------------------------------------------------------------ insertion rules

#[test]
fn issue_56_an_id_outside_the_manifest_or_a_repeat_is_refused() {
    let r = reader(2);
    let mut ix = PickIndex::new(&r).unwrap();
    let s = segment(pt(0.0, 0.0), pt(1.0, 0.0));
    assert!(matches!(ix.insert_exact(eid(2), s.clone()), Err(pvec::PvecError::Query { .. })));
    ix.insert_exact(eid(1), s.clone()).unwrap();
    assert!(matches!(ix.insert_exact(eid(1), s.clone()), Err(pvec::PvecError::Query { .. })));
    // A bare line has no bounds, so it cannot be an entity of the index.
    let line = Curve::Line(Line::new(pt(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap());
    assert!(ix.insert_exact(eid(0), line).is_err());
    assert!(matches!(ix.refine(eid(0), s), Err(pvec::PvecError::Query { .. })), "nothing to refine");
}

// ------------------------------------------------------------------ AC 6: mutation check

/// What a tessellated hit test would answer: the distance to a uniform-parameter polyline of `n`
/// segments. This is the mutant that must turn the high-curvature test red.
fn tessellated_distance(c: &Curve, p: Point2, n: usize) -> f64 {
    let d = c.domain();
    let pts: Vec<Point2> =
        (0..=n).map(|i| c.eval(d.lo() + d.width() * i as f64 / n as f64).unwrap()).collect();
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (a.x + t * dx - p.x).hypot(a.y + t * dy - p.y)
        })
        .fold(f64::INFINITY, f64::min)
}

fn hairpin_apex() -> Point2 {
    let c = hairpin();
    // x(t) is unimodal on this curve, so a ternary search finds the apex.
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..200 {
        let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
        if c.eval(m1).unwrap().x < c.eval(m2).unwrap().x { lo = m1 } else { hi = m2 }
    }
    c.eval((lo + hi) / 2.0).unwrap()
}

#[test]
fn issue_56_the_high_curvature_rational_spline_is_picked_at_its_apex() {
    let ix = index_of(&[hairpin()]);
    let apex = hairpin_apex();
    let p = pt(apex.x + 0.4, apex.y);
    let hit = ix.query_point(p, PickTolerance::new(1.0, &scale(1.0)).unwrap()).expect("0.4 px from the apex");
    assert!((hit.distance - 0.4).abs() < Policy::V1.rel.sqrt(), "distance {}", hit.distance);
    assert!(
        ix.query_point(pt(apex.x + 1.5, apex.y), PickTolerance::new(1.0, &scale(1.0)).unwrap()).is_none()
    );
}

#[test]
fn issue_56_mutation_a_tessellated_hit_test_misses_where_the_analytic_one_hits() {
    let apex = hairpin_apex();
    let p = pt(apex.x + 0.4, apex.y);
    let radius = 1.0;
    // The mutant, at a tessellation density a display would use, is further than the radius away, so
    // swapping it in for the analytic test turns `the_high_curvature_rational_spline_is_picked_at_its_apex` red.
    let mutant = tessellated_distance(&hairpin(), p, 31);
    assert!(
        mutant > radius,
        "a 31-segment tessellation would still hit ({mutant}), so the fixture is too gentle"
    );
    let ix = index_of(&[hairpin()]);
    let exact = ix.query_nearest(p, f64::INFINITY).unwrap().distance;
    assert!(exact <= radius);
}

// ------------------------------------------------------------------ nearest and ties

#[test]
fn issue_56_query_nearest_respects_max_dist_and_breaks_ties_by_lowest_id() {
    let ix = index_of(&[
        segment(pt(0.0, 1.0), pt(10.0, 1.0)),
        segment(pt(0.0, -1.0), pt(10.0, -1.0)),
        segment(pt(0.0, 5.0), pt(10.0, 5.0)),
    ]);
    let p = pt(5.0, 0.0);
    assert_eq!(ix.query_nearest(p, 2.0).unwrap().entity, eid(0));
    assert!(ix.query_nearest(p, 0.5).is_none());
    assert_eq!(ix.query_nearest(pt(5.0, 3.5), 10.0).unwrap().entity, eid(2));
    assert!(ix.query_nearest(p, f64::NAN).is_none());
    assert!(ix.query_nearest(p, -1.0).is_none());
}

#[test]
fn issue_56_a_hit_is_a_pick() {
    fn takes(_: Pick) {}
    let ix = index_of(&[segment(pt(0.0, 0.0), pt(1.0, 0.0))]);
    takes(ix.query_nearest(pt(0.5, 0.0), 1.0).unwrap());
}
