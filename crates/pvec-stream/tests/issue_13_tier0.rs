//! The Tier 0 complete-coverage stream (issue P5.4, pvec#13).
//!
//! Each test is named for the acceptance criterion it holds the stream to. The deviation checks do not
//! use the writer's measurement: they sample the exact curve through the P3.1 evaluator on their own and
//! compare with the bound the plan stored.
#![allow(missing_docs)]

use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Curve2d, Ellipse, Line, Placement2, Point2, Polyline, Trimmed,
    Vector2,
};
use pvec_stream::{
    Fidelity, Preview, PreviewForm, Refined, Scene, Tier0Builder, Tier0Entry, Tier0Error, Tier0Params,
    Tier0Plan, build_preview, is_cheap_exact, measure_deviation, MAX_PREVIEW_VERTICES,
};
use std::time::Instant;

// ---------------------------------------------------------------- budgets

/// The error budget of every fixture below: world units.
const MAX_ERROR: f64 = 5.0;
/// The stated byte budget of the Tier 0 plan stream of the million-entity fixture.
const MILLION_MAX_BYTES: usize = 8 * 1024 * 1024;
/// The stated decode-time budget of the same stream (a debug build, any host).
const MILLION_MAX_DECODE_MS: u128 = 2_000;

// ---------------------------------------------------------------- fixtures

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}
fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}
fn place(x: f64, y: f64, angle: f64) -> Placement2 {
    Placement2::new(pt(x, y), Vector2::new(angle.cos(), angle.sin())).unwrap()
}

fn segment(a: Point2, b: Point2) -> Curve {
    let len = a.distance(b);
    let line = Line::new(a, Vector2::new(b.x - a.x, b.y - a.y)).unwrap();
    Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, len, true).unwrap())
}

fn arc(rng: &mut Rng) -> Curve {
    let c = Circle::new(place(rng.range(0.0, 9000.0), rng.range(0.0, 9000.0), 0.0), rng.range(2.0, 300.0)).unwrap();
    let start = rng.range(0.0, 3.0);
    Curve::Trimmed(Trimmed::new(Curve::Circle(c), start, start + rng.range(0.3, 4.0), true).unwrap())
}

fn ellipse_arc(rng: &mut Rng) -> Curve {
    let a = rng.range(20.0, 400.0);
    let e = Ellipse::new(
        place(rng.range(0.0, 9000.0), rng.range(0.0, 9000.0), rng.range(0.0, 3.0)),
        a,
        a * rng.range(0.2, 0.9),
    )
    .unwrap();
    let start = rng.range(0.0, 3.0);
    Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), start, start + rng.range(0.5, 5.5), true).unwrap())
}

fn spline(rng: &mut Rng, n: usize, scale: f64) -> Curve {
    let p = 3usize;
    let (x0, y0) = (rng.range(0.0, 9000.0), rng.range(0.0, 9000.0));
    let cps: Vec<Point2> = (0..n)
        .map(|i| pt(x0 + scale * i as f64 + rng.range(-scale, scale), y0 + rng.range(-scale * 2.0, scale * 2.0)))
        .collect();
    let spans = n - p;
    let mut knots = vec![0.0; p + 1];
    knots.extend((1..spans).map(|k| k as f64 / spans as f64));
    knots.extend(vec![1.0; p + 1]);
    Curve::BSpline(BSpline::new(p, cps, knots).unwrap())
}

fn bezier(rng: &mut Rng) -> Curve {
    let (x0, y0) = (rng.range(0.0, 9000.0), rng.range(0.0, 9000.0));
    let cps = (0..4).map(|i| pt(x0 + 60.0 * i as f64, y0 + rng.range(-120.0, 120.0))).collect();
    Curve::Bezier(Bezier::new(cps).unwrap())
}

fn composite(rng: &mut Rng) -> Curve {
    let c = pt(rng.range(100.0, 9000.0), rng.range(100.0, 9000.0));
    let r = rng.range(20.0, 200.0);
    let circle = Circle::new(place(c.x, c.y, 0.0), r).unwrap();
    let a = Curve::Trimmed(Trimmed::new(Curve::Circle(circle), 0.0, 2.0, true).unwrap());
    let end_of_arc = a.eval(a.domain().hi()).unwrap();
    let tail = pt(end_of_arc.x + 90.0, end_of_arc.y - 40.0);
    Curve::Composite(Composite::new(vec![a, segment(end_of_arc, tail)]).unwrap())
}

fn polyline(rng: &mut Rng, n: usize) -> Curve {
    let (x0, y0) = (rng.range(0.0, 9000.0), rng.range(0.0, 9000.0));
    let pts = (0..n).map(|i| pt(x0 + 7.0 * i as f64, y0 + rng.range(-30.0, 30.0))).collect();
    Curve::Polyline(Polyline::new(pts).unwrap())
}

/// A plan-like drawing: about nine entities in ten are cheap, the rest are deferred, of every kind.
fn mixed(seed: u64, n: u32) -> Vec<(EntityId, Curve)> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|i| {
            let extra = (rng.next() % 64) as usize;
            let curve = match rng.next() % 40 {
                0..=27 => {
                    let a = pt(rng.range(0.0, 9000.0), rng.range(0.0, 9000.0));
                    segment(a, pt(a.x + rng.range(5.0, 400.0), a.y + rng.range(-200.0, 200.0)))
                }
                28..=31 => arc(&mut rng),
                32..=33 => polyline(&mut rng, 3 + extra % 30),
                34 => ellipse_arc(&mut rng),
                35 => spline(&mut rng, 5 + extra % 12, 25.0),
                36 => bezier(&mut rng),
                37 => composite(&mut rng),
                38 => polyline(&mut rng, 70 + extra),
                _ => ellipse_arc(&mut rng),
            };
            (eid(i), curve)
        })
        .collect()
}

fn params() -> Tier0Params {
    Tier0Params { max_error: MAX_ERROR }
}

fn plan_of(seed: u64, n: u32) -> (Vec<(EntityId, Curve)>, Tier0Plan) {
    let fx = mixed(seed, n);
    let plan = Tier0Plan::build(n, fx.clone(), params()).unwrap();
    (fx, plan)
}

// ------------------------------------------------- independent deviation

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 == 0.0 { 0.0 } else { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

fn dist_to(p: [f64; 2], poly: &[[f64; 2]]) -> f64 {
    if poly.len() == 1 {
        return (p[0] - poly[0][0]).hypot(p[1] - poly[0][1]);
    }
    poly.windows(2).map(|w| seg_dist(p, w[0], w[1])).fold(f64::INFINITY, f64::min)
}

/// Two-sided deviation by 3,000 samples of the exact curve, through P3.1.
fn sampled_deviation(curve: &Curve, preview: &Preview) -> f64 {
    let dom = curve.domain();
    let dense: Vec<[f64; 2]> = (0..=3000)
        .map(|i| {
            let p = curve.eval(dom.lo() + dom.width() * f64::from(i) / 3000.0).unwrap();
            [p.x, p.y]
        })
        .collect();
    let poly = preview.points();
    let mut worst = dense.iter().map(|&p| dist_to(p, &poly)).fold(0.0, f64::max);
    for w in poly.windows(2) {
        for k in 0..=8 {
            let t = f64::from(k) / 8.0;
            let q = [w[0][0] + t * (w[1][0] - w[0][0]), w[0][1] + t * (w[1][1] - w[0][1])];
            worst = worst.max(dist_to(q, &dense));
        }
    }
    worst
}

// =========================================================== coverage

#[test]
fn issue_13_every_fixture_has_tier0_entities_equal_to_artifact_entities() {
    for (seed, n) in [(1u64, 1_000u32), (2, 3_000), (3, 777)] {
        let (fx, plan) = plan_of(seed, n);
        assert_eq!(plan.artifact_entities(), n as usize);
        assert_eq!(plan.tier0_entities(), plan.artifact_entities(), "seed {seed}");
        assert!(plan.infeasible().is_empty(), "seed {seed}: nothing may be infeasible on this fixture");
        let ids: Vec<EntityId> = fx.iter().map(|(id, _)| *id).collect();
        plan.check_coverage(&ids).unwrap();
        for id in ids {
            assert!(plan.entry(id).is_some(), "{id} is not reachable from Tier 0");
        }
        assert_eq!(plan.exact_ids().len() + plan.previews().len(), n as usize);
        assert!(!plan.previews().is_empty() && !plan.exact_ids().is_empty(), "the fixture must exercise both");
    }
}

#[test]
fn issue_13_cheap_geometry_is_exact_and_everything_else_is_previewed() {
    let (fx, plan) = plan_of(4, 2_000);
    for (id, curve) in &fx {
        match plan.entry(*id).unwrap() {
            Tier0Entry::Exact { .. } => assert!(is_cheap_exact(curve), "{id} was shipped exact but is not cheap"),
            Tier0Entry::Preview(p) => {
                assert!(!is_cheap_exact(curve), "{id} is cheap but was previewed");
                assert_eq!(p.replaces(), *id);
            }
        }
    }
}

#[test]
fn issue_13_a_fixture_with_one_entity_absent_from_tier0_fails_the_writer() {
    let fx = mixed(5, 500);
    let absent = eid(137);
    let curves: Vec<_> = fx.iter().filter(|(id, _)| *id != absent).cloned().collect();
    let err = Tier0Plan::build(500, curves, params()).unwrap_err();
    assert_eq!(err, Tier0Error::Gap(absent));
    assert_eq!(err.rule(), "TIER0-01");

    let mut b = Tier0Builder::new(500, params()).unwrap();
    for (id, curve) in &fx {
        if *id != absent {
            b.add_curve(*id, curve).unwrap();
        }
    }
    assert_eq!(b.finish().unwrap_err(), Tier0Error::Gap(absent));
}

#[test]
fn issue_13_a_plan_checked_against_a_larger_artifact_reports_the_missing_entity() {
    let (fx, plan) = plan_of(6, 300);
    let mut ids: Vec<EntityId> = fx.iter().map(|(id, _)| *id).collect();
    ids.push(eid(300));
    assert_eq!(plan.check_coverage(&ids).unwrap_err(), Tier0Error::Gap(eid(300)));
    ids.pop();
    ids.pop();
    assert!(plan.check_coverage(&ids).is_err(), "an entity of Tier 0 that is not in the artifact is an error too");
}

#[test]
fn issue_13_an_entity_twice_or_out_of_range_is_a_writer_error() {
    let mut b = Tier0Builder::new(10, params()).unwrap();
    b.add_exact(eid(3)).unwrap();
    assert_eq!(b.add_exact(eid(3)).unwrap_err(), Tier0Error::Duplicate(eid(3)));
    assert_eq!(b.add_exact(eid(10)).unwrap_err(), Tier0Error::OutOfRange(eid(10)));
    assert_eq!(Tier0Error::Duplicate(eid(3)).rule(), "TIER0-02");
    assert_eq!(Tier0Error::OutOfRange(eid(10)).rule(), "TIER0-03");
}

// ============================================================ bounds

#[test]
fn issue_13_every_preview_stores_a_world_space_bound_that_holds_against_the_exact_curve() {
    let (fx, plan) = plan_of(7, 3_000);
    let mut checked = 0;
    for p in plan.previews() {
        let curve = &fx[p.replaces().index() as usize].1;
        assert!(p.error_bound() > 0.0 && p.error_bound().is_finite(), "{}: bound is {}", p.replaces(), p.error_bound());
        assert!(p.error_bound() <= MAX_ERROR, "{}: stored bound {} is over the budget", p.replaces(), p.error_bound());
        let dev = sampled_deviation(curve, p);
        assert!(
            dev <= p.error_bound(),
            "{} ({:?}): sampled deviation {dev} is over the stored bound {}",
            p.replaces(),
            p.form(),
            p.error_bound()
        );
        checked += 1;
    }
    assert!(checked > 200, "only {checked} previews were checked");
}

#[test]
fn issue_13_the_writer_measurement_agrees_with_the_bound_it_stores() {
    let (fx, plan) = plan_of(8, 800);
    for p in plan.previews() {
        let measured = measure_deviation(&fx[p.replaces().index() as usize].1, p).unwrap();
        assert!(measured <= p.error_bound(), "{}: measured {measured} over stored {}", p.replaces(), p.error_bound());
    }
}

#[test]
fn issue_13_the_form_follows_the_measurement_and_a_tighter_budget_changes_it() {
    let mut rng = Rng(9);
    let s = spline(&mut rng, 8, 25.0);
    let loose = build_preview(eid(0), &s, &Tier0Params { max_error: 200.0 }).unwrap().unwrap();
    assert_eq!(loose.form(), PreviewForm::ControlPolygon, "a spline small against the budget keeps its control polygon");
    assert_eq!(loose.offsets().len(), 8);
    let tight = build_preview(eid(0), &s, &Tier0Params { max_error: 0.05 }).unwrap().unwrap();
    assert_eq!(tight.form(), PreviewForm::ChordPolyline, "the polygon does not hold 0.05, so the writer falls back");
    assert!(tight.error_bound() <= 0.05);
    assert!(sampled_deviation(&s, &tight) <= tight.error_bound());
    let e = ellipse_arc(&mut rng);
    let p = build_preview(eid(1), &e, &params()).unwrap().unwrap();
    assert!(p.error_bound() <= MAX_ERROR && sampled_deviation(&e, &p) <= p.error_bound());
}

#[test]
fn issue_13_a_preview_whose_bound_is_over_the_budget_is_refused_by_the_writer() {
    let mut rng = Rng(10);
    let s = spline(&mut rng, 8, 25.0);
    let p = build_preview(eid(2), &s, &Tier0Params { max_error: 200.0 }).unwrap().unwrap();
    let mut b = Tier0Builder::new(3, params()).unwrap();
    assert!(p.error_bound() > 0.0);
    let hollow = Preview::new(eid(2), p.form(), 50.0, p.origin(), p.offsets().to_vec()).unwrap();
    assert_eq!(b.add_preview(hollow).unwrap_err(), Tier0Error::BadBound);
    assert_eq!(Preview::new(eid(0), PreviewForm::ChordPolyline, 0.0, [0.0; 2], vec![[0.0; 2]]).unwrap_err(), Tier0Error::BadBound);
    assert_eq!(Preview::new(eid(0), PreviewForm::ChordPolyline, f32::NAN, [0.0; 2], vec![[0.0; 2]]).unwrap_err(), Tier0Error::BadBound);
    assert!(Preview::new(eid(0), PreviewForm::ChordPolyline, 1.0, [0.0; 2], vec![]).is_err());
    assert!(Preview::new(eid(0), PreviewForm::ChordPolyline, 1.0, [0.0; 2], vec![[0.0; 2]; MAX_PREVIEW_VERTICES + 1]).is_err());
}

// ========================================================== fidelity

#[test]
fn issue_13_a_preview_is_display_fidelity_and_conformance_says_so() {
    let (_, plan) = plan_of(11, 1_500);
    for p in plan.previews() {
        assert!(!p.is_exact());
        assert_eq!(p.fidelity(), Fidelity::Display);
        assert_eq!(Tier0Entry::Preview(p).fidelity(), Fidelity::Display);
    }
    for id in plan.exact_ids() {
        assert_eq!(plan.entry(*id).unwrap().fidelity(), Fidelity::Exact);
    }
    let c = plan.conformance();
    assert_eq!(c.exact, plan.exact_ids().len());
    assert_eq!(c.display_fidelity, plan.previews().len());
    assert_eq!(c.infeasible, 0);
    assert_eq!(Fidelity::Display.label(), "display-fidelity");
    assert_eq!(Fidelity::Exact.label(), "exact");
    assert!(!Fidelity::Display.label().contains("exact"));
}

// ======================================================== replacement

fn scene_and_curve() -> (Tier0Plan, Vec<(EntityId, Curve)>, Scene) {
    let (fx, plan) = plan_of(12, 600);
    let scene = Scene::from_plan(&plan);
    (plan, fx, scene)
}

#[test]
fn issue_13_replacing_a_preview_keeps_the_identity_and_does_not_overdraw() {
    let (plan, fx, mut scene) = scene_and_curve();
    assert_eq!(scene.drawn(), plan.artifact_entities());
    let id = plan.previews()[0].replaces();
    assert_eq!(scene.fidelity(id), Some(Fidelity::Display));
    assert_eq!(scene.refine(id, &fx[id.index() as usize].1).unwrap(), Refined::Replaced);
    assert_eq!(scene.fidelity(id), Some(Fidelity::Exact));
    assert_eq!(scene.drawn(), plan.artifact_entities(), "refinement replaces, it does not add");
}

#[test]
fn issue_13_applying_refinement_twice_gives_the_same_scene_as_once() {
    let (plan, fx, mut once) = scene_and_curve();
    let mut twice = once.clone();
    for p in plan.previews() {
        let c = &fx[p.replaces().index() as usize].1;
        assert_eq!(once.refine(p.replaces(), c).unwrap(), Refined::Replaced);
        assert_eq!(twice.refine(p.replaces(), c).unwrap(), Refined::Replaced);
    }
    let after_once = once.clone();
    for p in plan.previews() {
        let c = &fx[p.replaces().index() as usize].1;
        assert_eq!(twice.refine(p.replaces(), c).unwrap(), Refined::Unchanged);
    }
    assert_eq!(twice, after_once);
    assert_eq!(once, twice);
}

#[test]
fn issue_13_refinement_in_any_order_gives_the_same_scene() {
    let (plan, fx, mut fwd) = scene_and_curve();
    let mut rev = fwd.clone();
    for p in plan.previews() {
        fwd.refine(p.replaces(), &fx[p.replaces().index() as usize].1).unwrap();
    }
    for p in plan.previews().iter().rev() {
        rev.refine(p.replaces(), &fx[p.replaces().index() as usize].1).unwrap();
    }
    assert_eq!(fwd, rev);
}

#[test]
fn issue_13_geometry_that_does_not_match_the_preview_is_refused() {
    let (plan, fx, mut scene) = scene_and_curve();
    let id = plan.previews()[0].replaces();
    let other = plan.previews()[1].replaces();
    assert_eq!(scene.refine(id, &fx[other.index() as usize].1).unwrap_err(), Tier0Error::Mismatch(id));
    assert_eq!(scene.fidelity(id), Some(Fidelity::Display), "a refused refinement changes nothing");
    scene.refine(id, &fx[id.index() as usize].1).unwrap();
    assert_eq!(scene.refine(id, &fx[other.index() as usize].1).unwrap_err(), Tier0Error::Conflict(id));
    assert_eq!(scene.refine(eid(9_999), &fx[0].1).unwrap_err(), Tier0Error::Unknown(eid(9_999)));
}

#[test]
fn issue_13_refining_an_entity_that_went_out_exact_changes_nothing() {
    let (plan, fx, mut scene) = scene_and_curve();
    let id = plan.exact_ids()[0];
    let before = scene.clone();
    assert_eq!(scene.refine(id, &fx[id.index() as usize].1).unwrap(), Refined::Unchanged);
    assert_eq!(scene, before);
}

// =========================================================== infeasible

#[test]
fn issue_13_an_entity_with_no_bounded_preview_is_declared_not_dropped() {
    // An ellipse arc a million units wide at a bound of a thousandth needs far more than the cap.
    let e = Ellipse::new(place(0.0, 0.0, 0.0), 1.0e6, 5.0e5).unwrap();
    let huge = Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), 0.0, 5.0, true).unwrap());
    let tight = Tier0Params { max_error: 1.0e-3 };
    let r = build_preview(eid(0), &huge, &tight).unwrap();
    assert_eq!(r.unwrap_err(), pvec_stream::InfeasibleReason::VertexCap);

    let mut b = Tier0Builder::new(2, tight).unwrap();
    b.add_curve(eid(0), &huge).unwrap();
    b.add_exact(eid(1)).unwrap();
    let plan = b.finish().unwrap();
    assert_eq!(plan.infeasible().len(), 1);
    assert_eq!(plan.infeasible()[0].id, eid(0));
    assert_eq!(plan.tier0_entities(), 1, "only reachable entities count");
    assert_eq!(plan.conformance().infeasible, 1);
    plan.check_coverage(&[eid(0), eid(1)]).unwrap();
    let back = Tier0Plan::decode(&plan.encode()).unwrap();
    assert_eq!(back.infeasible(), plan.infeasible());
}

#[test]
fn issue_13_chapter_4_of_the_streaming_doc_has_the_infeasibility_list() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/STREAMING.md")).unwrap();
    assert!(doc.contains("\n## 4. Tier 0"), "STREAMING.md has no chapter 4");
    let ch4 = doc.split("\n## 4. Tier 0").nth(1).unwrap().split("\n## 5.").next().unwrap();
    assert!(ch4.contains("### 4.6 Infeasibility"), "chapter 4 has no infeasibility section");
    assert!(ch4.contains("VertexCap") && ch4.contains("EvaluatorRefused"), "the reasons are not named");
    assert!(ch4.contains("million"), "the million-entity measurement is not recorded");
    assert!(ch4.contains("display fidelity"), "the doc must say previews are display fidelity");
}

// ============================================================= stream

#[test]
fn issue_13_the_stream_round_trips() {
    let (_, plan) = plan_of(13, 2_000);
    let bytes = plan.encode();
    assert_eq!(&bytes[..4], b"PVT0");
    let back = Tier0Plan::decode(&bytes).unwrap();
    assert_eq!(back, plan);
    assert_eq!(back.encode(), bytes, "the encoding is deterministic");
    assert_eq!(back.tier0_entities(), back.artifact_entities());
}

#[test]
fn issue_13_hostile_streams_are_refused_without_a_panic_or_an_allocation() {
    let (_, plan) = plan_of(14, 120);
    let bytes = plan.encode();
    for cut in 0..bytes.len() {
        assert!(Tier0Plan::decode(&bytes[..cut]).is_err(), "a stream cut at {cut} was accepted");
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(Tier0Plan::decode(&trailing).is_err());
    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 0xFF;
    assert!(Tier0Plan::decode(&bad_magic).is_err());
    // Header: magic(4) version(1) pad(3) entities(4) exact_runs(4) previews(4) infeasible(4).
    for at in [8usize, 12, 16, 20] {
        let mut lying = bytes.clone();
        lying[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(Tier0Plan::decode(&lying).is_err(), "a count of u32::MAX at {at} was accepted");
    }
    for i in 0..bytes.len() {
        let mut flipped = bytes.clone();
        flipped[i] ^= 0xA5;
        let _ = Tier0Plan::decode(&flipped);
    }
    // A stream that leaves an entity out is a gap, whatever else it says.
    let mut short = bytes.clone();
    short[8..12].copy_from_slice(&121u32.to_le_bytes());
    assert!(Tier0Plan::decode(&short).is_err());
}

#[test]
fn issue_13_the_million_entity_tier0_stays_within_its_byte_and_decode_budget() {
    // 1,000,000 entities, about 8% deferred (spike P1.7 measured 8.03% on `million`). The deferred ones
    // are previewed from a few hundred verified shapes, re-identified, because verifying 80,000
    // previews against the evaluator is a writer-side cost this test does not need to repeat.
    const N: u32 = 1_000_000;
    let shapes: Vec<Preview> = mixed(15, 600)
        .into_iter()
        .filter(|(_, c)| !is_cheap_exact(c))
        .map(|(id, c)| build_preview(id, &c, &params()).unwrap().unwrap())
        .collect();
    assert!(shapes.len() > 20);
    let mut rng = Rng(16);
    let mut b = Tier0Builder::new(N, params()).unwrap();
    let mut deferred = 0usize;
    for i in 0..N {
        if rng.next() % 100 < 8 {
            let s = &shapes[(rng.next() % shapes.len() as u64) as usize];
            b.add_preview(Preview::new(eid(i), s.form(), s.error_bound() as f32, s.origin(), s.offsets().to_vec()).unwrap())
                .unwrap();
            deferred += 1;
        } else {
            b.add_exact(eid(i)).unwrap();
        }
    }
    let plan = b.finish().unwrap();
    assert_eq!(plan.tier0_entities(), N as usize);
    let bytes = plan.encode();
    let t = Instant::now();
    let back = Tier0Plan::decode(&bytes).unwrap();
    let ms = t.elapsed().as_millis();
    eprintln!(
        "million: {deferred} previews, {} bytes ({:.1} MiB), decode {ms} ms (budget {MILLION_MAX_BYTES} bytes, {MILLION_MAX_DECODE_MS} ms)",
        bytes.len(),
        bytes.len() as f64 / 1_048_576.0
    );
    assert_eq!(back.tier0_entities(), N as usize);
    assert!(bytes.len() <= MILLION_MAX_BYTES, "{} bytes is over the budget", bytes.len());
    assert!(ms <= MILLION_MAX_DECODE_MS, "decode took {ms} ms");
}
