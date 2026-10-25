//! Bounded CPU scheduler (issue P5.7, pvec#16).
//!
//! Work is ordered by tier first (complete Tier 0 coverage before any deeper refinement), then by a score
//! of what the job buys per unit of cost: `(W_COVERAGE * coverage + W_ERROR * error_need) / (cost +
//! COST_FLOOR)`, scaled down by `INVISIBLE_FACTOR` when the set is off screen. While the camera moves,
//! jobs deeper than `INTERACTIVE_MAX_TIER` are held, not dropped. `run_slice` performs whole jobs (or
//! resumable chunks) and starts none whose estimate exceeds the time left, so it overshoots only by the
//! estimate error of the last job. Admission is capped on queued cost and on in-flight reads, and every
//! refusal is counted and hands the job back. In the simulated-clock spec (25% estimate error, 200,000 jobs)
//! the run_slice(8ms) overshoot p99 over 10,000 slices is 218us, and the spec asserts a 500us ceiling. All
//! weights are placeholders until P8.6 tunes them.

use crate::Generation;
use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::time::Duration;

/// A monotonic clock the host injects, so tests never depend on wall-clock time.
pub trait Clock {
    /// Time since an arbitrary fixed origin.
    fn now(&self) -> Duration;
}

/// What the viewer is doing right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    /// The camera is panning.
    Panning,
    /// The camera is zooming.
    Zooming,
    /// Nothing is moving.
    Idle,
}

/// Weight of screen coverage in the priority score (placeholder, tuned in P8.6).
pub const W_COVERAGE: f64 = 1.0;
/// Weight of remaining error need in the priority score (placeholder, tuned in P8.6).
pub const W_ERROR: f64 = 1.0;
/// Seconds added to a job's cost before dividing, so a near-free job cannot dominate (placeholder).
pub const COST_FLOOR: f64 = 0.000_1;
/// Factor applied to jobs that are not visible (placeholder).
pub const INVISIBLE_FACTOR: f64 = 0.01;
/// Error need is clamped to this before weighting (placeholder).
pub const ERROR_NEED_CAP: f64 = 1.0;
/// Highest tier that still runs while the camera moves; deeper refinement waits for idle.
pub const INTERACTIVE_MAX_TIER: u8 = 0;

/// One unit of schedulable work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Job {
    /// The generation that owns the job.
    pub generation: Generation,
    /// Which entity set the job decodes or refines.
    pub entity_set: u32,
    /// Whether the set intersects the viewport.
    pub visible: bool,
    /// Fraction of the screen the set covers, 0 to 1.
    pub screen_coverage: f64,
    /// Estimated decode and evaluation cost.
    pub estimated_cost: Duration,
    /// Remaining error need: current preview bound over the target screen-space error, clamped.
    pub error_need: f64,
    /// Tier the job produces.
    pub tier: u8,
}

/// What a job reports after one chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// The job is finished.
    Done,
    /// The job did one chunk and wants to run again.
    Continue,
}

/// The caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedulerConfig {
    /// Most reads in flight at once.
    pub max_in_flight_reads: u32,
    /// Most estimated decode cost queued at once. Each job counts at least `COST_FLOOR`, so this also caps the job count.
    pub max_queued_cost: Duration,
}

/// Why work was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefusalReason {
    /// The queued-cost cap is reached.
    QueueFull,
    /// The in-flight read cap is reached.
    ReadsInFlight,
    /// The job belongs to an older generation.
    StaleGeneration,
}

/// A refused job, handed back so the caller can retry it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Refused {
    /// The job that was not admitted.
    pub job: Job,
    /// Why.
    pub reason: RefusalReason,
}

/// Counters that make every outcome observable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SchedulerMetrics {
    /// Jobs admitted.
    pub admitted: u64,
    /// Jobs refused because the queue was full.
    pub refused_queue: u64,
    /// Reads refused because the cap was reached.
    pub refused_reads: u64,
    /// Jobs refused as stale at submission.
    pub refused_stale: u64,
    /// Queued jobs dropped because a newer generation began.
    pub cancelled: u64,
    /// Jobs that finished.
    pub completed: u64,
}

/// The priority of a job: higher runs sooner. Whether a job is held is decided by tier and interaction in
/// `run_slice`, never by the score, because a legitimate job can score zero.
pub fn priority(job: &Job) -> f64 {
    let need = W_COVERAGE * job.screen_coverage + W_ERROR * job.error_need.min(ERROR_NEED_CAP);
    let visible = if job.visible { 1.0 } else { INVISIBLE_FACTOR };
    visible * need / (job.estimated_cost.as_secs_f64() + COST_FLOOR)
}

/// What a job counts against the queue cap: its estimate, but never less than `COST_FLOOR`, so the cap
/// also bounds how many jobs are queued and zero-cost jobs cannot be admitted without limit.
fn charged(job: &Job) -> Duration {
    job.estimated_cost.max(Duration::from_secs_f64(COST_FLOOR))
}

#[derive(Debug)]
struct Entry {
    job: Job,
    score: f64,
    seq: u64,
}

impl Ord for Entry {
    fn cmp(&self, other: &Entry) -> Ordering {
        other
            .job
            .tier
            .cmp(&self.job.tier)
            .then(self.score.total_cmp(&other.score))
            .then(other.seq.cmp(&self.seq))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Entry) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Entry) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

/// The scheduler.
#[derive(Debug)]
pub struct Scheduler {
    config: SchedulerConfig,
    generation: Generation,
    interaction: Interaction,
    queue: BinaryHeap<Entry>,
    queued_cost: Duration,
    reads: u32,
    seq: u64,
    metrics: SchedulerMetrics,
}

impl Scheduler {
    /// A scheduler with the given caps, at the given generation.
    pub fn new(config: SchedulerConfig, generation: Generation) -> Scheduler {
        Scheduler {
            config,
            generation,
            interaction: Interaction::Idle,
            queue: BinaryHeap::new(),
            queued_cost: Duration::ZERO,
            reads: 0,
            seq: 0,
            metrics: SchedulerMetrics::default(),
        }
    }
    /// Begin a newer generation: queued older jobs are dropped and counted.
    pub fn advance(&mut self, generation: Generation) {
        if generation <= self.generation {
            return;
        }
        self.generation = generation;
        let all = std::mem::take(&mut self.queue).into_vec();
        for e in all {
            if e.job.generation < generation {
                self.metrics.cancelled += 1;
                self.queued_cost -= charged(&e.job);
            } else {
                self.queue.push(e);
            }
        }
    }
    /// Set the interaction state.
    pub fn set_interaction(&mut self, interaction: Interaction) {
        self.interaction = interaction;
    }
    /// Admit a job or hand it back.
    pub fn submit(&mut self, job: Job) -> Result<(), Refused> {
        if job.generation < self.generation {
            self.metrics.refused_stale += 1;
            return Err(Refused { job, reason: RefusalReason::StaleGeneration });
        }
        if self.queued_cost + charged(&job) > self.config.max_queued_cost {
            self.metrics.refused_queue += 1;
            return Err(Refused { job, reason: RefusalReason::QueueFull });
        }
        self.queued_cost += charged(&job);
        self.metrics.admitted += 1;
        self.seq += 1;
        self.queue.push(Entry { score: priority(&job), job, seq: self.seq });
        Ok(())
    }
    /// Claim a read slot.
    pub fn try_begin_read(&mut self) -> Result<(), RefusalReason> {
        if self.reads >= self.config.max_in_flight_reads {
            self.metrics.refused_reads += 1;
            return Err(RefusalReason::ReadsInFlight);
        }
        self.reads += 1;
        Ok(())
    }
    /// Release a read slot.
    pub fn finish_read(&mut self) {
        self.reads = self.reads.saturating_sub(1);
    }
    /// Run jobs until the budget is spent, starting no job whose estimate exceeds what is left.
    ///
    /// The first job of a slice always runs, so a job costlier than the budget cannot starve.
    pub fn run_slice(&mut self, budget: Duration, clock: &impl Clock, mut run: impl FnMut(&Job) -> Step) {
        let start = clock.now();
        let mut ran = false;
        loop {
            let elapsed = clock.now().saturating_sub(start);
            if elapsed >= budget {
                break;
            }
            let Some(top) = self.queue.peek() else { break };
            // Tier is the primary key, so if the best job is held every other job is held too.
            if self.interaction != Interaction::Idle && top.job.tier > INTERACTIVE_MAX_TIER {
                break;
            }
            if ran && top.job.estimated_cost > budget - elapsed {
                break;
            }
            let Some(entry) = self.queue.pop() else { break };
            ran = true;
            match run(&entry.job) {
                Step::Done => {
                    self.queued_cost -= charged(&entry.job);
                    self.metrics.completed += 1;
                }
                Step::Continue => self.queue.push(entry),
            }
        }
    }
    /// Jobs still queued, held ones included.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    /// Reads in flight.
    pub fn in_flight_reads(&self) -> u32 {
        self.reads
    }
    /// The counters.
    pub fn metrics(&self) -> SchedulerMetrics {
        self.metrics
    }
}
