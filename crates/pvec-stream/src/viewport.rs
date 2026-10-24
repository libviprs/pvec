//! Viewport queries and priority range reads (issue P5.5, pvec#14).

use crate::directory::{Aabb, DirectoryError, SpatialDirectory};
use pvec_container::directory::BlockEntry;
use pvec_container::reader::RangeRequest;
use pvec_container::source::{AsyncBlockSource, BlockSource};
use pvec_container::{AsyncContainerReader, BlockId, ContainerReader, ReadError};

/// The default margin around the viewport that is prefetched, in display pixels.
pub const DEFAULT_PREFETCH_MARGIN_PX: f64 = 128.0;

/// What a reader must answer for a plan to be built against it.
pub trait PlanSource {
    /// The block directory, in file order.
    fn entries(&self) -> &[BlockEntry];
    /// The coalesced requests that would fetch `ids`.
    fn plan(&self, ids: &[BlockId]) -> Result<Vec<RangeRequest>, ReadError>;
    /// Whether the block is in the decoded-block cache.
    fn is_cached(&self, id: BlockId) -> bool;
}

impl<S: BlockSource> PlanSource for ContainerReader<S> {
    fn entries(&self) -> &[BlockEntry] {
        ContainerReader::entries(self)
    }
    fn plan(&self, ids: &[BlockId]) -> Result<Vec<RangeRequest>, ReadError> {
        ContainerReader::plan(self, ids)
    }
    fn is_cached(&self, id: BlockId) -> bool {
        ContainerReader::is_cached(self, id)
    }
}

impl<S: AsyncBlockSource> PlanSource for AsyncContainerReader<S> {
    fn entries(&self) -> &[BlockEntry] {
        AsyncContainerReader::entries(self)
    }
    fn plan(&self, ids: &[BlockId]) -> Result<Vec<RangeRequest>, ReadError> {
        AsyncContainerReader::plan(self, ids)
    }
    fn is_cached(&self, id: BlockId) -> bool {
        AsyncContainerReader::is_cached(self, id)
    }
}

/// What a viewport asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportQuery {
    /// The visible rectangle, in source space.
    pub bounds: Aabb,
    /// Display pixels per source unit. Finite and above zero.
    pub scale: f64,
    /// The most bytes of visible requests one plan may ask for. Requests past it are deferred.
    pub tier_budget: u64,
}

/// How a plan is built.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportConfig {
    /// The prefetch margin around the viewport, in display pixels. Zero turns prefetch off.
    pub prefetch_margin_px: f64,
}

impl Default for ViewportConfig {
    fn default() -> ViewportConfig {
        ViewportConfig { prefetch_margin_px: DEFAULT_PREFETCH_MARGIN_PX }
    }
}

/// Why a request is in a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
    /// It holds blocks that touch the viewport.
    Visible,
    /// It holds blocks that touch only the margin around the viewport.
    Prefetch,
}

/// One request of a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct PlannedRequest {
    /// The range and the blocks in it.
    pub request: RangeRequest,
    /// Why it is there.
    pub priority: Priority,
    /// For a visible request, the entities it brings into the viewport (a block that only partly
    /// overlaps counts for its overlapping share). Zero for a prefetch request.
    pub gain: f64,
}

/// What one class of requests costs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClassMetrics {
    /// Range requests.
    pub ranges: u32,
    /// Bytes the requests span.
    pub bytes_requested: u64,
    /// Bytes of those that belong to no wanted block (the gaps coalescing read).
    pub bytes_wasted: u64,
    /// Blocks the requests fetch.
    pub blocks: u32,
    /// Blocks the viewport wants that the cache already holds.
    pub blocks_cached: u32,
}

/// The cost of a plan.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlanMetrics {
    /// The requests for what is visible.
    pub visible: ClassMetrics,
    /// The requests for the margin, kept apart so its cost stays visible.
    pub prefetch: ClassMetrics,
    /// Requests the budget pushed out of this plan.
    pub deferred_ranges: u32,
    /// Bytes of those.
    pub deferred_bytes: u64,
}

/// An ordered, coalesced read plan.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadPlan {
    /// The requests to make, in the order to make them: visible first, best gain per byte first.
    pub requests: Vec<PlannedRequest>,
    /// Requests the budget left out, in the order they would have come.
    pub deferred: Vec<PlannedRequest>,
    /// Whether the advanced curve set, which no cell holds, touches the viewport (P5.2 splits it).
    pub advanced_in_view: bool,
    /// What the plan costs.
    pub metrics: PlanMetrics,
}

impl ReadPlan {
    /// The blocks of every request, in plan order.
    pub fn block_ids(&self) -> Vec<BlockId> {
        self.requests.iter().flat_map(|r| r.request.blocks.iter().copied()).collect()
    }

    /// The blocks of the visible requests only.
    pub fn visible_block_ids(&self) -> Vec<BlockId> {
        self.requests
            .iter()
            .filter(|r| r.priority == Priority::Visible)
            .flat_map(|r| r.request.blocks.iter().copied())
            .collect()
    }
}

/// Why a plan could not be built.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ViewportError {
    /// The scale is not finite and above zero (VP-01).
    BadScale,
    /// The margin is not finite and at least zero, or pushes the viewport out of range (VP-02).
    BadMargin,
    /// The reader refused the plan (VP-03, carrying its own rule).
    Read(ReadError),
}

impl ViewportError {
    /// The rule it breaks.
    pub fn rule(&self) -> &'static str {
        match self {
            ViewportError::BadScale => "VP-01",
            ViewportError::BadMargin => "VP-02",
            ViewportError::Read(_) => "VP-03",
        }
    }
}

impl core::fmt::Display for ViewportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let rule = self.rule();
        match self {
            ViewportError::BadScale => write!(f, "{rule}: the scale is not finite and above zero"),
            ViewportError::BadMargin => write!(f, "{rule}: the prefetch margin is not usable"),
            ViewportError::Read(e) => write!(f, "{rule}: {e}"),
        }
    }
}

impl std::error::Error for ViewportError {}

impl From<ReadError> for ViewportError {
    fn from(e: ReadError) -> ViewportError {
        ViewportError::Read(e)
    }
}

impl From<DirectoryError> for ViewportError {
    fn from(_: DirectoryError) -> ViewportError {
        ViewportError::BadMargin
    }
}

/// The rules of this module, by id.
pub const VIEWPORT_RULES: &[(&str, &str)] = &[
    ("VP-01", "the scale is not finite and above zero"),
    ("VP-02", "the prefetch margin is not finite, not at least zero, or leaves the numeric range"),
    ("VP-03", "the reader refused the plan"),
];

impl ViewportQuery {
    /// A query for `bounds` at `scale` display pixels per source unit, with no budget.
    pub fn new(bounds: Aabb, scale: f64) -> ViewportQuery {
        ViewportQuery { bounds, scale, tier_budget: u64::MAX }
    }

    /// The plan for this viewport against `dir` and the cache state of `reader`. Nothing is fetched.
    pub fn plan<R: PlanSource>(
        &self,
        dir: &SpatialDirectory,
        reader: &R,
        config: &ViewportConfig,
    ) -> Result<ReadPlan, ViewportError> {
        let _ = (dir, reader, config);
        todo!("P5.5")
    }
}
