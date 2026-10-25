//! Cancellation and refinement generations (issue P5.6, pvec#15).
//!
//! While a read is in flight the user keeps panning, so results arrive for views that no longer exist.
//! Every unit of work therefore carries the [`Generation`] it was started for, the apply path
//! (`Scene::apply`) refuses anything older than what it already holds, and a superseded job stops at its
//! next checkpoint. The design is in `docs/STREAMING.md` section 4.8.

use crate::tiers::Preview;
use pvec_entity::EntityId;
use pvec_geometry::Curve;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The most units of work (blocks) a job may finish after it was cancelled: the one already in flight.
/// A job checks its token before it starts each unit, and drops the output of a unit that finished after
/// the cancel.
pub const MAX_UNITS_AFTER_CANCEL: u32 = 1;

/// The number of a viewport: monotonic, incremented on every viewport change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(u64);

impl Generation {
    /// The generation before any viewport change.
    pub const INITIAL: Generation = Generation(0);
    /// The number.
    pub fn value(self) -> u64 {
        self.0
    }
}

/// How refined the thing standing for an entity is. Higher replaces lower, never the other way round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Refinement(u8);

impl Refinement {
    /// A Tier 0 preview.
    pub const TIER0: Refinement = Refinement(0);
    /// Tier 1.
    pub const TIER1: Refinement = Refinement(1);
    /// Tier 2: the exact geometry.
    pub const TIER2: Refinement = Refinement(2);
    /// The tier number.
    pub fn tier(self) -> u8 {
        self.0
    }
}

/// A job stopped at a checkpoint because its generation was superseded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled {
    /// The generation the job belonged to.
    pub generation: Generation,
}

impl core::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "the job of generation {} was superseded", self.generation.0)
    }
}

impl std::error::Error for Cancelled {}

/// What cancellation has cost and saved so far.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CancelMetrics {
    /// Jobs that stopped at a checkpoint because they were superseded.
    pub jobs_cancelled: u64,
    /// Units of work that finished after their job was cancelled, and whose output was thrown away.
    pub work_discarded: u64,
}

#[derive(Debug, Default)]
struct Counters {
    jobs_cancelled: AtomicU64,
    work_discarded: AtomicU64,
}

/// The cancellation token of one generation, checked at every checkpoint of a job.
#[derive(Clone, Debug)]
pub struct CancelToken {
    generation: Generation,
    flag: Arc<AtomicBool>,
    counters: Arc<Counters>,
}

impl CancelToken {
    /// The generation this token belongs to.
    pub fn generation(&self) -> Generation {
        self.generation
    }
    /// Whether the generation was superseded.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
    /// A checkpoint: `Err` when the generation was superseded, and the job must stop.
    pub fn checkpoint(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() { Err(Cancelled { generation: self.generation }) } else { Ok(()) }
    }
}

/// Hands out generations and the token of the current one.
#[derive(Debug)]
pub struct GenerationClock {
    state: Mutex<CancelToken>,
    counters: Arc<Counters>,
}

impl Default for GenerationClock {
    fn default() -> Self {
        GenerationClock::new()
    }
}

impl GenerationClock {
    /// A clock at [`Generation::INITIAL`].
    pub fn new() -> GenerationClock {
        let counters = Arc::new(Counters::default());
        let token = CancelToken {
            generation: Generation::INITIAL,
            flag: Arc::new(AtomicBool::new(false)),
            counters: Arc::clone(&counters),
        };
        GenerationClock { state: Mutex::new(token), counters }
    }
    /// The current generation.
    pub fn current(&self) -> Generation {
        self.token().generation
    }
    /// The token of the current generation.
    pub fn token(&self) -> CancelToken {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// A viewport change: the next generation, and its token. The token of the one it replaces is
    /// cancelled.
    pub fn advance(&self) -> CancelToken {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.flag.store(true, Ordering::Release);
        let next = CancelToken {
            generation: Generation(state.generation.0 + 1),
            flag: Arc::new(AtomicBool::new(false)),
            counters: Arc::clone(&self.counters),
        };
        *state = next.clone();
        next
    }
    /// The counters.
    pub fn metrics(&self) -> CancelMetrics {
        CancelMetrics {
            jobs_cancelled: self.counters.jobs_cancelled.load(Ordering::Relaxed),
            work_discarded: self.counters.work_discarded.load(Ordering::Relaxed),
        }
    }
}

/// How a job over a list of units ended.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutcome<T> {
    /// Every unit ran, and the token was never cancelled.
    Complete(Vec<T>),
    /// The token was cancelled. `done` holds the outputs of the units that finished before the cancel;
    /// `discarded` counts the units that finished after it, whose outputs were thrown away.
    Cancelled {
        /// Outputs finished before the cancel.
        done: Vec<T>,
        /// Units finished after the cancel and discarded.
        discarded: u32,
    },
}

/// Runs `work` over `units` in order, with a checkpoint before each unit and one after it, so a job
/// finishes at most [`MAX_UNITS_AFTER_CANCEL`] unit after its token is cancelled and throws that unit's
/// output away. An error from `work` ends the job and is not counted as a cancel.
pub fn run_job<U, T, E>(
    token: &CancelToken,
    units: impl IntoIterator<Item = U>,
    mut work: impl FnMut(U) -> Result<T, E>,
) -> Result<JobOutcome<T>, E> {
    let mut done = Vec::new();
    for u in units {
        if token.is_cancelled() {
            token.counters.jobs_cancelled.fetch_add(1, Ordering::Relaxed);
            return Ok(JobOutcome::Cancelled { done, discarded: 0 });
        }
        let out = work(u)?;
        if token.is_cancelled() {
            // The unit that was in flight when the cancel came: its output is for a view that is gone.
            token.counters.jobs_cancelled.fetch_add(1, Ordering::Relaxed);
            token.counters.work_discarded.fetch_add(1, Ordering::Relaxed);
            return Ok(JobOutcome::Cancelled { done, discarded: 1 });
        }
        done.push(out);
    }
    Ok(JobOutcome::Complete(done))
}

/// A result of a read, a decode or a refinement, tagged with the generation it was started for. The
/// fields are private and the only constructors take a generation, so a result cannot reach
/// `Scene::apply` without one.
#[derive(Clone, Debug, PartialEq)]
pub struct RefinementResult {
    generation: Generation,
    update: Update,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Update {
    Preview(Preview),
    Exact { entity: EntityId, curve: Curve },
}

impl RefinementResult {
    /// A Tier 0 preview for the entity it replaces.
    pub fn preview(generation: Generation, preview: Preview) -> RefinementResult {
        RefinementResult { generation, update: Update::Preview(preview) }
    }
    /// The exact geometry of an entity.
    pub fn exact(generation: Generation, entity: EntityId, curve: Curve) -> RefinementResult {
        RefinementResult { generation, update: Update::Exact { entity, curve } }
    }
    /// The generation the work was started for.
    pub fn generation(&self) -> Generation {
        self.generation
    }
    /// The entity the result is for.
    pub fn entity(&self) -> EntityId {
        match &self.update {
            Update::Preview(p) => p.replaces(),
            Update::Exact { entity, .. } => *entity,
        }
    }
    /// How refined the result is.
    pub fn refinement(&self) -> Refinement {
        match &self.update {
            Update::Preview(_) => Refinement::TIER0,
            Update::Exact { .. } => Refinement::TIER2,
        }
    }
    pub(crate) fn update(&self) -> &Update {
        &self.update
    }
}

/// Why the apply barrier dropped a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    /// The result belongs to a generation older than the scene's.
    StaleGeneration,
    /// The entity already has something more refined than the result.
    StaleRefinement,
}

/// What `Scene::apply` did with a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The result replaced what stood for the entity.
    Replaced,
    /// The result was already what stands for the entity.
    Unchanged,
    /// The barrier refused the result. It is counted, and the scene is as it was.
    Dropped(DropReason),
}

/// How many results the barrier has dropped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropCounts {
    /// Dropped for an older generation.
    pub stale_generation: u64,
    /// Dropped for an older refinement.
    pub stale_refinement: u64,
}

impl DropCounts {
    /// Both kinds together.
    pub fn total(&self) -> u64 {
        self.stale_generation + self.stale_refinement
    }
}
