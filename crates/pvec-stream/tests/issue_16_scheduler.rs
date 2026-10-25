//! The bounded CPU scheduler (issue P5.7, pvec#16). One test per acceptance criterion, driven by an
//! injected clock so nothing depends on wall-clock time.
//!
//! The million-entity fixture generator under `spikes/fixtures` is a separate workspace with a binary
//! only, so `pvec-stream` cannot reach it. The scheduler never reads entity data, only jobs over entity
//! sets, so these tests build the same scale in memory: 200,000 jobs of 5 entities each.
#![allow(missing_docs)]

use pvec_stream::{
    Clock, Generation, GenerationClock, Interaction, Job, RefusalReason, Scheduler, SchedulerConfig,
    Step, priority,
};
use std::cell::Cell;
use std::time::Duration;

struct Manual(Cell<Duration>);

impl Clock for Manual {
    fn now(&self) -> Duration {
        self.0.get()
    }
}

impl Manual {
    fn new() -> Manual {
        Manual(Cell::new(Duration::ZERO))
    }
    fn advance(&self, d: Duration) {
        self.0.set(self.0.get() + d);
    }
}

fn us(n: u64) -> Duration {
    Duration::from_micros(n)
}

fn job(g: Generation, set: u32, tier: u8, coverage: f64, cost: Duration) -> Job {
    Job {
        generation: g,
        entity_set: set,
        visible: true,
        screen_coverage: coverage,
        estimated_cost: cost,
        error_need: 0.5,
        tier,
    }
}

fn big() -> SchedulerConfig {
    SchedulerConfig { max_in_flight_reads: 8, max_queued_cost: Duration::from_secs(10_000) }
}

fn g0() -> Generation {
    GenerationClock::new().current()
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

#[test]
fn issue_16_forty_percent_beats_a_tenth_of_a_percent() {
    let big_job = job(g0(), 1, 2, 0.40, us(500));
    let small = job(g0(), 2, 2, 0.001, us(500));
    assert!(priority(&big_job, Interaction::Idle) > priority(&small, Interaction::Idle));
}

#[test]
fn issue_16_invisible_work_ranks_below_visible_work() {
    let seen = job(g0(), 1, 2, 0.01, us(500));
    let mut hidden = seen;
    hidden.visible = false;
    hidden.screen_coverage = 0.4;
    assert!(priority(&seen, Interaction::Idle) > priority(&hidden, Interaction::Idle));
}

#[test]
fn issue_16_tier0_completes_before_any_tier2() {
    let mut s = Scheduler::new(big(), g0());
    // Tier 2 jobs are submitted first and are the larger, costlier-looking prize on every other axis.
    for i in 0..50 {
        s.submit(job(g0(), 1000 + i, 2, 0.9, us(100))).unwrap();
    }
    for i in 0..200 {
        s.submit(job(g0(), i, 0, 0.001, us(100))).unwrap();
    }
    let clock = Manual::new();
    let mut order = Vec::new();
    while s.queued() > 0 {
        s.run_slice(us(1000), &clock, |j| {
            order.push(j.tier);
            clock.advance(j.estimated_cost);
            Step::Done
        });
    }
    let first_t2 = order.iter().position(|t| *t == 2).unwrap();
    assert_eq!(first_t2, 200, "all 200 tier 0 jobs come first");
    assert!(order[..first_t2].iter().all(|t| *t == 0));
}

#[test]
fn issue_16_in_flight_reads_never_exceed_the_cap_over_100_viewport_changes() {
    let cap = 4;
    let mut s = Scheduler::new(
        SchedulerConfig { max_in_flight_reads: cap, max_queued_cost: Duration::from_secs(10) },
        g0(),
    );
    let gens = GenerationClock::new();
    let mut rng = Rng(16);
    let mut in_flight = 0u32; // the counting source
    let mut peak = 0u32;
    let mut refused = 0u64;
    for _ in 0..100 {
        s.advance(gens.advance().generation());
        for _ in 0..30 {
            match s.try_begin_read() {
                Ok(()) => {
                    in_flight += 1;
                    peak = peak.max(in_flight);
                }
                Err(r) => {
                    assert_eq!(r, RefusalReason::ReadsInFlight);
                    refused += 1;
                }
            }
            if rng.next() % 3 == 0 && in_flight > 0 {
                in_flight -= 1;
                s.finish_read();
            }
            assert_eq!(s.in_flight_reads(), in_flight);
        }
    }
    assert!(peak <= cap, "peak {peak} over cap {cap}");
    assert_eq!(peak, cap, "the stress reaches the cap");
    assert!(refused > 0);
    assert_eq!(s.metrics().refused_reads, refused);
}

#[test]
fn issue_16_refusal_is_counted_and_hands_the_job_back() {
    let mut s = Scheduler::new(
        SchedulerConfig { max_in_flight_reads: 1, max_queued_cost: us(1000) },
        g0(),
    );
    let mut refused = Vec::new();
    for i in 0..10 {
        if let Err(r) = s.submit(job(g0(), i, 1, 0.1, us(300))) {
            assert_eq!(r.reason, RefusalReason::QueueFull);
            assert_eq!(r.job.entity_set, i, "the refused job comes back intact");
            refused.push(r.job);
        }
    }
    let m = s.metrics();
    assert_eq!(m.admitted, 3);
    assert_eq!(m.refused_queue, 7);
    assert_eq!(refused.len(), 7);
    assert_eq!(s.queued(), 3);
    // Draining frees room, and a refused job can be resubmitted.
    let clock = Manual::new();
    s.run_slice(Duration::from_secs(1), &clock, |_| Step::Done);
    assert_eq!(s.queued(), 0);
    s.submit(refused[0]).unwrap();
}

#[test]
fn issue_16_stale_generation_is_refused_and_queued_stale_work_is_counted() {
    let gens = GenerationClock::new();
    let old = gens.current();
    let mut s = Scheduler::new(big(), old);
    for i in 0..5 {
        s.submit(job(old, i, 1, 0.1, us(100))).unwrap();
    }
    let new = gens.advance().generation();
    s.advance(new);
    assert_eq!(s.queued(), 0);
    assert_eq!(s.metrics().cancelled, 5);
    let r = s.submit(job(old, 9, 1, 0.1, us(100))).unwrap_err();
    assert_eq!(r.reason, RefusalReason::StaleGeneration);
    assert_eq!(s.metrics().refused_stale, 1);
}

#[test]
fn issue_16_refinement_yields_during_a_pan_and_resumes_on_idle() {
    let mut s = Scheduler::new(big(), g0());
    for i in 0..5 {
        s.submit(job(g0(), i, 0, 0.1, us(100))).unwrap();
        s.submit(job(g0(), 100 + i, 2, 0.1, us(100))).unwrap();
    }
    let clock = Manual::new();
    s.set_interaction(Interaction::Panning);
    let mut tiers = Vec::new();
    s.run_slice(Duration::from_secs(1), &clock, |j| {
        tiers.push(j.tier);
        Step::Done
    });
    assert_eq!(tiers, vec![0; 5], "only tier 0 runs while panning");
    assert_eq!(s.queued(), 5, "refinement is held, not dropped");
    s.set_interaction(Interaction::Idle);
    s.run_slice(Duration::from_secs(1), &clock, |j| {
        tiers.push(j.tier);
        Step::Done
    });
    assert_eq!(tiers.iter().filter(|t| **t == 2).count(), 5);
    assert_eq!(s.queued(), 0);
}

#[test]
fn issue_16_a_slice_stops_before_a_job_that_will_not_fit_and_chunks_resume() {
    let mut s = Scheduler::new(big(), g0());
    s.submit(job(g0(), 1, 1, 0.5, us(600))).unwrap();
    s.submit(job(g0(), 2, 1, 0.4, us(600))).unwrap();
    let clock = Manual::new();
    let mut ran = 0;
    s.run_slice(us(1000), &clock, |j| {
        ran += 1;
        clock.advance(j.estimated_cost);
        Step::Done
    });
    assert_eq!(ran, 1, "the second job's estimate exceeds the 400us left");
    assert_eq!(s.queued(), 1);
    // A job that asks to continue stays queued for the next slice.
    let mut chunks = 0;
    let mut s = Scheduler::new(big(), g0());
    s.submit(job(g0(), 3, 1, 0.5, us(100))).unwrap();
    for _ in 0..3 {
        s.run_slice(us(150), &clock, |_| {
            chunks += 1;
            clock.advance(us(100));
            if chunks < 3 { Step::Continue } else { Step::Done }
        });
    }
    assert_eq!(chunks, 3);
    assert_eq!(s.queued(), 0);
    assert_eq!(s.metrics().completed, 1);
}

fn trace(seed: u64) -> (Vec<u32>, pvec_stream::SchedulerMetrics) {
    let mut rng = Rng(seed);
    let mut s = Scheduler::new(big(), g0());
    for i in 0..2000 {
        let r = rng.next();
        let mut j = job(g0(), i, (r % 3) as u8, (r >> 8) as f64 % 100.0 / 100.0, us(50 + (r >> 20) % 400));
        j.error_need = ((r >> 32) % 100) as f64 / 100.0;
        s.submit(j).unwrap();
    }
    let clock = Manual::new();
    let mut order = Vec::new();
    while s.queued() > 0 {
        s.run_slice(us(2000), &clock, |j| {
            order.push(j.entity_set);
            clock.advance(j.estimated_cost);
            Step::Done
        });
    }
    (order, s.metrics())
}

#[test]
fn issue_16_order_and_counters_are_deterministic_for_a_fixed_input() {
    assert_eq!(trace(7), trace(7));
    assert_ne!(trace(7).0, trace(8).0);
}

#[test]
fn issue_16_run_slice_8ms_overshoot_p99_over_10000_slices_on_a_million_entities() {
    const JOBS: u32 = 200_000; // 5 entities each: one million entities
    const OVERSHOOT_P99_MAX: Duration = Duration::from_millis(2);
    let mut rng = Rng(1_000_000);
    let mut s = Scheduler::new(big(), g0());
    for i in 0..JOBS {
        let r = rng.next();
        let cost = us(20 + r % 1980); // 20us to 2ms
        let mut j = job(g0(), i, (r >> 16) as u8 % 3, ((r >> 24) % 1000) as f64 / 1000.0, cost);
        j.error_need = ((r >> 40) % 100) as f64 / 100.0;
        s.submit(j).unwrap();
    }
    let clock = Manual::new();
    let budget = Duration::from_millis(8);
    let mut overshoots = Vec::with_capacity(10_000);
    for _ in 0..10_000 {
        let start = clock.now();
        s.run_slice(budget, &clock, |j| {
            // The estimate is off by up to 25% either way.
            let r = rng.next() % 51;
            clock.advance(j.estimated_cost * (75 + r as u32) / 100);
            Step::Done
        });
        overshoots.push((clock.now() - start).saturating_sub(budget));
    }
    overshoots.sort();
    let p99 = overshoots[9_899];
    println!("run_slice(8ms) overshoot over 10000 slices: p99 = {p99:?}, max = {:?}", overshoots[9_999]);
    assert!(p99 <= OVERSHOOT_P99_MAX, "p99 overshoot {p99:?}");
    assert!(s.queued() > 0, "the slices did not simply drain the queue");
}
