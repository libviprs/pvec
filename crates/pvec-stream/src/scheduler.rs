//! Bounded CPU scheduler (issue P5.7, pvec#16). Stub: the types and signatures only, bodies land with the fix.

use crate::Generation;
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
    /// Most estimated decode cost queued at once.
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

/// The priority of a job: higher runs sooner. Zero means held under this interaction state.
pub fn priority(job: &Job, interaction: Interaction) -> f64 {
    let _ = (job, interaction);
    unimplemented!("P5.7")
}

/// The scheduler.
#[derive(Debug)]
pub struct Scheduler {
    _config: SchedulerConfig,
}

impl Scheduler {
    /// A scheduler with the given caps, at the given generation.
    pub fn new(config: SchedulerConfig, generation: Generation) -> Scheduler {
        let _ = generation;
        Scheduler { _config: config }
    }
    /// Begin a newer generation: queued older jobs are dropped and counted.
    pub fn advance(&mut self, generation: Generation) {
        let _ = generation;
        unimplemented!("P5.7")
    }
    /// Set the interaction state.
    pub fn set_interaction(&mut self, interaction: Interaction) {
        let _ = interaction;
        unimplemented!("P5.7")
    }
    /// Admit a job or hand it back.
    pub fn submit(&mut self, job: Job) -> Result<(), Refused> {
        let _ = job;
        unimplemented!("P5.7")
    }
    /// Claim a read slot.
    pub fn try_begin_read(&mut self) -> Result<(), RefusalReason> {
        unimplemented!("P5.7")
    }
    /// Release a read slot.
    pub fn finish_read(&mut self) {
        unimplemented!("P5.7")
    }
    /// Run jobs until the budget is spent, starting no job whose estimate exceeds what is left.
    pub fn run_slice(&mut self, budget: Duration, clock: &impl Clock, run: impl FnMut(&Job) -> Step) {
        let _ = (budget, clock, run);
        unimplemented!("P5.7")
    }
    /// Jobs still queued, held ones included.
    pub fn queued(&self) -> usize {
        unimplemented!("P5.7")
    }
    /// Reads in flight.
    pub fn in_flight_reads(&self) -> u32 {
        unimplemented!("P5.7")
    }
    /// The counters.
    pub fn metrics(&self) -> SchedulerMetrics {
        unimplemented!("P5.7")
    }
}
