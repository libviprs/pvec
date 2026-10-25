//! Cancellation and refinement generations (issue P5.6, pvec#15).
//!
//! Each test is named for the acceptance criterion it holds the barrier or the token to. The barrier
//! tests drive `Scene::apply` the way a reader does: results are tagged when the work starts and arrive
//! later, in whatever order the delays put them.
#![allow(missing_docs)]

use pvec_entity::EntityId;
use pvec_geometry::{Curve, Ellipse, Placement2, Point2, Trimmed, Vector2};
use pvec_stream::{
    Applied, DropReason, Fidelity, Generation, GenerationClock, JobOutcome, MAX_UNITS_AFTER_CANCEL,
    Refinement, RefinementResult, Scene, Tier0Error, Tier0Params, Tier0Plan, run_job,
};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

const N: u32 = 64;

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
}

fn eid(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

fn ellipse_arc(rng: &mut Rng) -> Curve {
    let a = 20.0 + 380.0 * rng.unit();
    let at = Point2::new(9000.0 * rng.unit(), 9000.0 * rng.unit());
    let angle = 3.0 * rng.unit();
    let place = Placement2::new(at, Vector2::new(angle.cos(), angle.sin())).unwrap();
    let e = Ellipse::new(place, a, a * (0.2 + 0.7 * rng.unit())).unwrap();
    let start = 3.0 * rng.unit();
    Curve::Trimmed(Trimmed::new(Curve::Ellipse(e), start, start + 0.5 + 5.0 * rng.unit(), true).unwrap())
}

/// Every entity an ellipse arc, so every entity goes out as a Tier 0 preview.
fn fixture() -> (Tier0Plan, Vec<Curve>) {
    let mut rng = Rng(15);
    let curves: Vec<Curve> = (0..N).map(|_| ellipse_arc(&mut rng)).collect();
    let plan = Tier0Plan::build(
        N,
        curves.iter().enumerate().map(|(i, c)| (eid(i as u32), c.clone())),
        Tier0Params { max_error: 5.0 },
    )
    .unwrap();
    assert_eq!(plan.previews().len(), N as usize, "the fixture is all previews");
    (plan, curves)
}

fn exact(g: Generation, curves: &[Curve], i: u32) -> RefinementResult {
    RefinementResult::exact(g, eid(i), curves[i as usize].clone())
}

/// The entities generation `g` looks at: eight of the 64, sliding as the user pans.
fn viewport(g: u64) -> Vec<u32> {
    (0..8).map(|k| ((g * 7 + k) % u64::from(N)) as u32).collect()
}

// ======================================================= the barrier

#[test]
fn issue_15_a_hundred_viewport_changes_with_delayed_reads_end_in_the_last_viewports_state() {
    let (plan, curves) = fixture();
    let clock = GenerationClock::new();
    let mut scene = Scene::from_plan(&plan);
    // Every viewport change starts a read. The reads are slow, so none has landed when the user stops.
    let mut in_flight = Vec::new();
    for _ in 0..100 {
        let token = clock.advance();
        scene.advance(token.generation());
        in_flight.push((token.generation(), viewport(token.generation().value())));
    }
    assert_eq!(clock.current().value(), 100);
    // The delays reorder them: newest-but-one first, the last one in the middle, the rest scrambled.
    let mut order: Vec<usize> = (0..100).collect();
    let mut rng = Rng(7);
    for i in (1..order.len()).rev() {
        order.swap(i, (rng.next() % (i as u64 + 1)) as usize);
    }
    let mut sent = 0u64;
    for idx in order {
        let (g, ids) = &in_flight[idx];
        for &i in ids {
            let r = scene.apply(exact(*g, &curves, i)).unwrap();
            sent += 1;
            if g.value() != 100 {
                assert_eq!(r, Applied::Dropped(DropReason::StaleGeneration), "generation {}", g.value());
            }
        }
    }
    // What the last viewport alone produces.
    let mut alone = Scene::from_plan(&plan);
    alone.advance(Generation::INITIAL);
    let last = clock.current();
    for &i in &viewport(100) {
        alone.apply(exact(last, &curves, i)).unwrap();
    }
    assert!(scene.same_drawing(&alone), "the scene is the last viewport's scene");
    for i in 0..N {
        let want = viewport(100).contains(&i);
        assert_eq!(scene.fidelity(eid(i)) == Some(Fidelity::Exact), want, "entity {i}");
    }
    assert_eq!(scene.dropped().stale_generation, sent - 8, "every older result was counted");
    assert_eq!(scene.dropped().stale_refinement, 0);
}

#[test]
fn issue_15_results_that_land_between_viewport_changes_never_survive_a_later_generation() {
    let (plan, curves) = fixture();
    let clock = GenerationClock::new();
    let mut scene = Scene::from_plan(&plan);
    let mut pending: Option<(Generation, Vec<u32>)> = None;
    for _ in 0..100 {
        let token = clock.advance();
        scene.advance(token.generation());
        // The read of the previous viewport lands one change late.
        if let Some((g, ids)) = pending.take() {
            for i in ids {
                assert_eq!(
                    scene.apply(exact(g, &curves, i)).unwrap(),
                    Applied::Dropped(DropReason::StaleGeneration)
                );
            }
        }
        pending = Some((token.generation(), viewport(token.generation().value())));
    }
    let (g, ids) = pending.unwrap();
    for i in ids {
        assert_ne!(scene.apply(exact(g, &curves, i)).unwrap(), Applied::Dropped(DropReason::StaleGeneration));
    }
    assert_eq!(scene.dropped().stale_generation, 99 * 8);
}

#[test]
fn issue_15_a_late_tier0_preview_for_an_exact_entity_is_dropped_by_state_and_by_counter() {
    let (plan, curves) = fixture();
    let mut scene = Scene::from_plan(&plan);
    let g = Generation::INITIAL;
    let id = eid(5);
    let preview = plan.previews().iter().find(|p| p.replaces() == id).unwrap().clone();
    assert_eq!(scene.refinement(id), Some(Refinement::TIER0));
    assert_eq!(scene.apply(exact(g, &curves, 5)).unwrap(), Applied::Replaced);
    assert_eq!(scene.refinement(id), Some(Refinement::TIER2));
    let before = scene.clone();
    assert_eq!(scene.dropped().total(), 0);
    // The Tier 0 read of the same viewport lands late.
    let late = scene.apply(RefinementResult::preview(g, preview.clone())).unwrap();
    assert_eq!(late, Applied::Dropped(DropReason::StaleRefinement));
    assert_eq!(scene.fidelity(id), Some(Fidelity::Exact));
    assert!(scene.same_drawing(&before));
    assert_eq!(scene.dropped().stale_refinement, 1);
    assert_eq!(scene.dropped().stale_generation, 0);
    // A newer generation does not make a preview better than exact geometry either.
    let clock = GenerationClock::new();
    let newer = clock.advance().generation();
    let late = scene.apply(RefinementResult::preview(newer, preview)).unwrap();
    assert_eq!(late, Applied::Dropped(DropReason::StaleRefinement));
    assert_eq!(scene.fidelity(id), Some(Fidelity::Exact));
    assert_eq!(scene.dropped().stale_refinement, 2);
}

#[test]
fn issue_15_a_tier0_preview_for_an_entity_that_went_out_exact_is_dropped_too() {
    let plan = {
        let mut rng = Rng(3);
        let line = |rng: &mut Rng| {
            let a = Point2::new(100.0 * rng.unit(), 100.0 * rng.unit());
            let l = pvec_geometry::Line::new(a, Vector2::new(10.0, 0.0)).unwrap();
            Curve::Trimmed(Trimmed::new(Curve::Line(l), 0.0, 10.0, true).unwrap())
        };
        Tier0Plan::build(2, (0..2).map(|i| (eid(i), line(&mut rng))), Tier0Params { max_error: 5.0 }).unwrap()
    };
    let (other, _) = fixture();
    let mut scene = Scene::from_plan(&plan);
    assert_eq!(scene.refinement(eid(0)), Some(Refinement::TIER2), "cheap geometry went out exact");
    let stray = other.previews()[0].clone();
    let r = scene.apply(RefinementResult::preview(Generation::INITIAL, stray)).unwrap();
    assert_eq!(r, Applied::Dropped(DropReason::StaleRefinement));
    assert_eq!(scene.refinement(eid(0)), Some(Refinement::TIER2));
}

#[test]
fn issue_15_a_preview_replaces_a_preview_and_exact_replaces_a_preview_in_any_arrival_order() {
    let (plan, curves) = fixture();
    let g = Generation::INITIAL;
    let p = plan.previews()[3].clone();
    let id = p.replaces();
    let mut a = Scene::from_plan(&plan);
    assert_eq!(a.apply(RefinementResult::preview(g, p.clone())).unwrap(), Applied::Replaced);
    assert_eq!(a.apply(exact(g, &curves, id.index())).unwrap(), Applied::Replaced);
    assert_eq!(a.apply(exact(g, &curves, id.index())).unwrap(), Applied::Unchanged);
    let mut b = Scene::from_plan(&plan);
    b.apply(exact(g, &curves, id.index())).unwrap();
    b.apply(RefinementResult::preview(g, p)).unwrap();
    assert!(a.same_drawing(&b), "exact then preview ends where preview then exact does");
    assert_eq!(a.dropped().total(), 0);
    assert_eq!(b.dropped().stale_refinement, 1);
}

#[test]
fn issue_15_the_scene_generation_only_moves_forward() {
    let (plan, curves) = fixture();
    let clock = GenerationClock::new();
    let mut scene = Scene::from_plan(&plan);
    let gens: Vec<Generation> = (0..5).map(|_| clock.advance().generation()).collect();
    scene.advance(gens[4]);
    scene.advance(gens[2]);
    assert_eq!(scene.generation(), gens[4]);
    // A result of a newer generation than the scene knows raises it.
    let newer = clock.advance().generation();
    assert_ne!(scene.apply(exact(newer, &curves, 0)).unwrap(), Applied::Dropped(DropReason::StaleGeneration));
    assert_eq!(scene.generation(), newer);
    assert_eq!(
        scene.apply(exact(gens[3], &curves, 1)).unwrap(),
        Applied::Dropped(DropReason::StaleGeneration)
    );
    assert_eq!(scene.fidelity(eid(1)), Some(Fidelity::Display));
}

#[test]
fn issue_15_the_barrier_still_reports_unknown_entities_and_conflicting_geometry() {
    let (plan, curves) = fixture();
    let mut scene = Scene::from_plan(&plan);
    let g = Generation::INITIAL;
    let bad = RefinementResult::exact(g, eid(N + 1), curves[0].clone());
    assert!(matches!(scene.apply(bad), Err(Tier0Error::Unknown(_))));
    scene.apply(exact(g, &curves, 0)).unwrap();
    let other = RefinementResult::exact(g, eid(0), curves[1].clone());
    assert!(scene.apply(other).is_err(), "different geometry for a refined entity is refused");
    assert_eq!(scene.dropped().total(), 0, "an error is not a drop");
}

#[test]
fn issue_15_no_result_reaches_the_scene_without_a_generation_by_the_type_of_apply() {
    // The only way in takes a RefinementResult, and every constructor of one takes a Generation.
    let _apply: fn(&mut Scene, RefinementResult) -> Result<Applied, Tier0Error> = Scene::apply;
    let _preview: fn(Generation, pvec_stream::Preview) -> RefinementResult = RefinementResult::preview;
    let _exact: fn(Generation, EntityId, Curve) -> RefinementResult = RefinementResult::exact;
    let (plan, curves) = fixture();
    let r = exact(Generation::INITIAL, &curves, 0);
    assert_eq!(r.generation(), Generation::INITIAL);
    assert_eq!(r.entity(), eid(0));
    assert_eq!(r.refinement(), Refinement::TIER2);
    assert_eq!(
        RefinementResult::preview(Generation::INITIAL, plan.previews()[0].clone()).refinement(),
        Refinement::TIER0
    );
}

// ======================================================= cancellation

#[test]
fn issue_15_advancing_the_clock_cancels_the_superseded_token_and_only_that_one() {
    let clock = GenerationClock::new();
    let first = clock.advance();
    assert_eq!(first.generation().value(), 1);
    assert!(first.checkpoint().is_ok());
    let second = clock.advance();
    assert!(first.is_cancelled());
    assert_eq!(first.checkpoint().unwrap_err().generation, first.generation());
    assert!(!second.is_cancelled());
    assert_eq!(clock.token().generation(), second.generation());
    assert!(!clock.token().is_cancelled());
}

#[test]
fn issue_15_a_cancelled_job_stops_within_the_stated_number_of_blocks_measured() {
    let clock = GenerationClock::new();
    let token = clock.advance();
    let mut started = Vec::new();
    let mut after_cancel = 0u32;
    let out = run_job(&token, 0..1000u32, |b| -> Result<u32, ()> {
        if token.is_cancelled() {
            after_cancel += 1;
        }
        started.push(b);
        if b == 10 {
            clock.advance(); // the user pans while block 10 is being decoded
        }
        Ok(b)
    })
    .unwrap();
    let JobOutcome::Cancelled { done, discarded } = out else { panic!("the job was not cancelled") };
    assert_eq!(done, (0..=9).collect::<Vec<_>>(), "block 10 finished after the cancel and is discarded");
    assert!(discarded <= MAX_UNITS_AFTER_CANCEL, "discarded {discarded}");
    assert_eq!(discarded, 1);
    assert_eq!(started.len(), 11, "no block started after the cancel");
    assert_eq!(after_cancel, 0);
    let m = clock.metrics();
    assert_eq!((m.jobs_cancelled, m.work_discarded), (1, 1));
}

#[test]
fn issue_15_a_job_whose_token_is_already_cancelled_decodes_nothing() {
    let clock = GenerationClock::new();
    let token = clock.advance();
    clock.advance();
    let mut ran = 0;
    let out = run_job(&token, 0..50u32, |b| -> Result<u32, ()> {
        ran += 1;
        Ok(b)
    })
    .unwrap();
    assert_eq!(out, JobOutcome::Cancelled { done: vec![], discarded: 0 });
    assert_eq!(ran, 0);
    assert_eq!(clock.metrics().jobs_cancelled, 1);
    assert_eq!(clock.metrics().work_discarded, 0);
}

#[test]
fn issue_15_a_job_that_is_never_superseded_completes_and_counts_nothing() {
    let clock = GenerationClock::new();
    let token = clock.advance();
    let out = run_job(&token, 0..20u32, |b| -> Result<u32, ()> { Ok(b * 2) }).unwrap();
    assert_eq!(out, JobOutcome::Complete((0..20).map(|b| b * 2).collect()));
    assert_eq!(clock.metrics().jobs_cancelled, 0);
}

#[test]
fn issue_15_a_work_error_surfaces_and_is_not_counted_as_a_cancel() {
    let clock = GenerationClock::new();
    let token = clock.advance();
    let out = run_job(&token, 0..5u32, |b| if b == 2 { Err("bad block") } else { Ok(b) });
    assert_eq!(out, Err("bad block"));
    assert_eq!(clock.metrics().jobs_cancelled, 0);
}

// ======================================================= threads

#[test]
fn issue_15_generations_stay_monotonic_and_unique_across_threads() {
    let clock = Arc::new(GenerationClock::new());
    let threads = 8;
    let per = 500;
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let clock = Arc::clone(&clock);
            std::thread::spawn(move || {
                let mut seen = Vec::new();
                let mut last_current = Generation::INITIAL;
                for _ in 0..per {
                    let t = clock.advance();
                    seen.push(t.generation());
                    let now = clock.current();
                    assert!(now >= last_current, "current went backwards");
                    assert!(now >= t.generation());
                    last_current = now;
                }
                seen
            })
        })
        .collect();
    let mut all = BTreeSet::new();
    for h in handles {
        let seen = h.join().unwrap();
        assert!(seen.windows(2).all(|w| w[0] < w[1]), "one thread saw its generations out of order");
        for g in seen {
            assert!(all.insert(g), "generation {} handed out twice", g.value());
        }
    }
    assert_eq!(all.len(), threads * per);
    assert_eq!(clock.current().value(), (threads * per) as u64);
    assert_eq!(all.iter().next().unwrap().value(), 1);
    assert_eq!(all.iter().last().unwrap().value(), (threads * per) as u64);
}

#[test]
fn issue_15_concurrent_workers_never_apply_out_of_order_or_leave_a_stale_result_in_the_scene() {
    let (plan, curves) = fixture();
    let curves = Arc::new(curves);
    let clock = Arc::new(GenerationClock::new());
    let scene = Arc::new(Mutex::new(Scene::from_plan(&plan)));
    let workers = 6;
    let rounds = 200;
    let handles: Vec<_> = (0..workers)
        .map(|w| {
            let (clock, scene, curves) = (Arc::clone(&clock), Arc::clone(&scene), Arc::clone(&curves));
            std::thread::spawn(move || {
                let mut rng = Rng(100 + w);
                let mut seen = Generation::INITIAL;
                for _ in 0..rounds {
                    let token = clock.advance();
                    {
                        let mut s = scene.lock().unwrap();
                        s.advance(token.generation());
                        assert!(s.generation() >= seen, "the scene generation went backwards");
                        seen = s.generation();
                    }
                    // Another worker may supersede this generation before the result lands.
                    let i = (rng.next() % u64::from(N)) as u32;
                    let mut s = scene.lock().unwrap();
                    let before = s.generation();
                    s.apply(exact(token.generation(), &curves, i)).unwrap();
                    assert!(s.generation() >= before);
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    let s = scene.lock().unwrap();
    assert_eq!(s.generation(), clock.current(), "the scene ends at the newest generation");
    // Every drop was a stale generation (nothing here is a preview over exact).
    assert_eq!(s.dropped().stale_refinement, 0);
    // What is exact is exact for an entity some worker refined, and nothing was torn: every slot is
    // either a plan preview or the exact geometry.
    for i in 0..N {
        match s.fidelity(eid(i)) {
            Some(Fidelity::Display) => {}
            Some(Fidelity::Exact) => assert_eq!(s.refinement(eid(i)), Some(Refinement::TIER2)),
            None => panic!("entity {i} lost its drawing"),
        }
    }
    assert_eq!(s.drawn(), N as usize);
}
