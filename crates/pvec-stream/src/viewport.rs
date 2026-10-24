//! Viewport queries and priority range reads (issue P5.5, pvec#14).

use crate::directory::{Aabb, DirectoryError, SpatialDirectory};
use pvec_container::directory::BlockEntry;
use pvec_container::reader::RangeRequest;
use pvec_container::source::{AsyncBlockSource, BlockSource};
use pvec_container::{AsyncContainerReader, BlockId, ContainerReader, ReadError};
use std::collections::BTreeMap;

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

    /// The plan for this viewport against `dir` and the cache state of `reader`. Nothing is fetched,
    /// and nothing about the cache changes: the same viewport, artifact and cache state give the same
    /// plan, request for request.
    ///
    /// `dir` must be the directory of the file `reader` has open. A block is wanted when its own bounds
    /// touch the viewport (visible) or the viewport grown by the margin (prefetch), so the plan holds no
    /// block outside that, though a coalesced request may read the bytes between two blocks it holds
    /// (`bytes_wasted`). Blocks the cache already has are counted and not requested. Visible and
    /// prefetch blocks are coalesced apart from each other, so the cost of the margin is a number of
    /// its own, at the price of a request now and then where one merged range would have served both.
    pub fn plan<R: PlanSource>(
        &self,
        dir: &SpatialDirectory,
        reader: &R,
        config: &ViewportConfig,
    ) -> Result<ReadPlan, ViewportError> {
        if !(self.scale.is_finite() && self.scale > 0.0) {
            return Err(ViewportError::BadScale);
        }
        let px = config.prefetch_margin_px;
        let margin = px / self.scale;
        if !(px.is_finite() && px >= 0.0 && margin.is_finite()) {
            return Err(ViewportError::BadMargin);
        }
        let (lo, hi) = (self.bounds.min(), self.bounds.max());
        let outer = Aabb::new([lo[0] - margin, lo[1] - margin], [hi[0] + margin, hi[1] + margin])?;
        let centre = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0];

        let entries = reader.entries();
        let mut visible: Vec<BlockId> = Vec::new();
        let mut gains: BTreeMap<BlockId, f64> = BTreeMap::new();
        let mut near: Vec<BlockId> = Vec::new();
        let mut distance: BTreeMap<BlockId, f64> = BTreeMap::new();
        let (mut visible_cached, mut prefetch_cached) = (0u32, 0u32);
        for cell in dir.query(&outer) {
            for k in 0..cell.blocks.count {
                let id = BlockId(cell.blocks.first.0 + k);
                let Some(entry) = entries.get(id.0 as usize) else { continue };
                let Some(b) = entry.bounds.and_then(|b| Aabb::new(b.min, b.max).ok()) else { continue };
                let seen = b.intersects(&self.bounds);
                if !seen && !b.intersects(&outer) {
                    continue;
                }
                let cached = reader.is_cached(id);
                match (seen, cached) {
                    (true, true) => visible_cached += 1,
                    (false, true) => prefetch_cached += 1,
                    (true, false) => {
                        visible.push(id);
                        gains.insert(id, f64::from(entry.count) * share(&b, &self.bounds));
                    }
                    (false, false) => {
                        near.push(id);
                        let (bl, bh) = (b.min(), b.max());
                        let (dx, dy) = ((bl[0] + bh[0]) / 2.0 - centre[0], (bl[1] + bh[1]) / 2.0 - centre[1]);
                        distance.insert(id, dx * dx + dy * dy);
                    }
                }
            }
        }

        // One coalescing pass over everything wanted, so a request never re-reads a block that another
        // request of the plan fetches, and the gaps it does read are bytes no block of the plan owns.
        let wanted: Vec<BlockId> = visible.iter().chain(&near).copied().collect();
        let mut first: Vec<PlannedRequest> = Vec::new();
        let mut second: Vec<PlannedRequest> = Vec::new();
        for request in reader.plan(&wanted)? {
            let gain: f64 = request.blocks.iter().filter_map(|b| gains.get(b)).sum();
            if request.blocks.iter().any(|b| gains.contains_key(b)) {
                first.push(PlannedRequest { request, priority: Priority::Visible, gain });
            } else {
                second.push(PlannedRequest { request, priority: Priority::Prefetch, gain: 0.0 });
            }
        }
        // the most visible entities per byte first, the file order among equals
        first.sort_by(|a, b| {
            let (ra, rb) = (a.gain / f64::from(a.request.len), b.gain / f64::from(b.request.len));
            rb.total_cmp(&ra).then(a.request.offset.cmp(&b.request.offset))
        });
        // the margin nearest the viewport first
        let nearest = |p: &PlannedRequest| {
            p.request.blocks.iter().filter_map(|b| distance.get(b)).copied().fold(f64::INFINITY, f64::min)
        };
        second
            .sort_by(|a, b| nearest(a).total_cmp(&nearest(b)).then(a.request.offset.cmp(&b.request.offset)));

        // the budget is for what is visible: the first request always goes, so a query makes progress,
        // and what does not fit waits, prefetch with it
        let mut spent = 0u64;
        let mut cut = first.len();
        for (i, p) in first.iter().enumerate() {
            let len = u64::from(p.request.len);
            if i > 0 && spent.saturating_add(len) > self.tier_budget {
                cut = i;
                break;
            }
            spent += len;
        }
        let mut deferred = first.split_off(cut);
        let mut requests = first;
        if deferred.is_empty() {
            requests.append(&mut second);
        } else {
            deferred.append(&mut second);
        }

        // A request that mixes the two is a visible one, and the margin blocks riding in it are charged
        // to the margin: the two classes add up to the bytes spanned, and the gaps are the request's.
        let mut metrics = PlanMetrics::default();
        for p in &requests {
            let (mut own, mut rider, mut rider_blocks) = (0u64, 0u64, 0u32);
            for b in &p.request.blocks {
                if p.priority == Priority::Visible && distance.contains_key(b) {
                    rider += stored(entries, *b);
                    rider_blocks += 1;
                } else {
                    own += stored(entries, *b);
                }
            }
            let len = u64::from(p.request.len);
            let waste = len - own - rider;
            let class = match p.priority {
                Priority::Visible => &mut metrics.visible,
                Priority::Prefetch => &mut metrics.prefetch,
            };
            class.ranges += 1;
            class.bytes_requested += len - rider;
            class.bytes_wasted += waste;
            class.blocks += p.request.blocks.len() as u32 - rider_blocks;
            if rider_blocks > 0 && p.priority == Priority::Visible {
                metrics.prefetch.bytes_requested += rider;
                metrics.prefetch.blocks += rider_blocks;
            }
        }
        metrics.visible.blocks_cached = visible_cached;
        metrics.prefetch.blocks_cached = prefetch_cached;
        metrics.deferred_ranges = deferred.len() as u32;
        metrics.deferred_bytes = deferred.iter().map(|p| u64::from(p.request.len)).sum();
        let advanced_in_view = dir.advanced_bounds().is_some_and(|b| b.intersects(&self.bounds));
        Ok(ReadPlan { requests, deferred, advanced_in_view, metrics })
    }
}

fn stored(entries: &[BlockEntry], id: BlockId) -> u64 {
    entries.get(id.0 as usize).map_or(0, |e| u64::from(e.stored_length))
}

/// The share of block bounds `b` that lies inside `v`, per axis so that a block of horizontal lines
/// (no height) is not worth nothing. At least a sliver, so a block that only touches still ranks.
fn share(b: &Aabb, v: &Aabb) -> f64 {
    let axis = |i: usize| {
        let (bl, bh, vl, vh) = (b.min()[i], b.max()[i], v.min()[i], v.max()[i]);
        let len = bh - bl;
        if len <= 0.0 { 1.0 } else { ((bh.min(vh) - bl.max(vl)) / len).clamp(0.0, 1.0) }
    };
    (axis(0) * axis(1)).max(1e-9)
}
