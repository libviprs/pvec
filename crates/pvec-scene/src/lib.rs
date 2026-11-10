//! The paint order of a page, worked out in one place (issue 296, `docs/PAINT.md` sections 2.2 and 5.3).

use pvec_capabilities::CapabilityId;
use pvec_components::{Band, ClipStream, FillRule, ItemKind, PaintOrderStream, PaintRun, Stream};
use pvec_entity::EntityId;
use std::collections::HashMap;

/// The most batches a plan may hold before the legacy plan is returned instead.
pub const DEFAULT_MAX_BATCHES: usize = 4096;

/// What a viewer will pay for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The most batches a plan may hold.
    pub max_batches: usize,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy { max_batches: DEFAULT_MAX_BATCHES }
    }
}

/// How many items of each kind the page has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counts {
    /// Strokes: the entities `0..strokes` are drawn.
    pub strokes: u32,
    /// Records of the hatch stream.
    pub fills: u32,
    /// Records of the text stream.
    pub texts: u32,
    /// The band of each raster, indexed by raster number.
    pub rasters: Vec<Band>,
}

/// Items `first` to `first + count - 1` of one kind, drawn one after the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintBatch {
    /// What the items are.
    pub kind: ItemKind,
    /// The number of the first item.
    pub first: u32,
    /// How many items, at least one.
    pub count: u32,
}

/// Something the plan gave up, reported instead of hidden.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Degrade {
    /// The stream asked for more batches than the policy allows, so the legacy plan was used.
    TooManyBatches,
    /// A stream could not be used, so the legacy plan was used.
    StreamDamaged {
        /// The capability of the stream.
        id: CapabilityId,
        /// The rule that failed, such as `COMP-22`.
        rule: &'static str,
    },
    /// A clip has a ring of fewer than 3 points or with a coordinate that is not finite, or an ancestor that has,
    /// so its items are not drawn (`docs/PAINT.md` 2.5).
    ClipUnbuildable {
        /// The outermost clip of the chain that failed.
        clip: u32,
        /// How many items are not drawn because of it.
        items: u32,
    },
}

/// The batches in paint order, back to front, and what was given up on the way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenePlan {
    /// The batches.
    pub batches: Vec<PaintBatch>,
    /// What was given up.
    pub degrades: Vec<Degrade>,
}

impl ScenePlan {
    /// Works out the plan from the component streams of a page.
    ///
    /// Without a paint order stream (capability 23) the plan is the legacy stage order: underlay rasters, fills,
    /// strokes, text, overlay rasters. With one, the items it does not name come first in that order and the
    /// listed runs follow in file order. A stream that cannot be decoded, or that names an item the page does not
    /// have, is reported as `StreamDamaged` and the legacy plan is used. A plan over `policy.max_batches` is
    /// reported as `TooManyBatches` and the legacy plan is used. This never panics on a damaged payload.
    pub fn build(streams: &[Stream], counts: &Counts, policy: Policy) -> ScenePlan {
        let legacy = |degrade: Option<Degrade>| ScenePlan {
            batches: unlisted_first(counts, &[]),
            degrades: degrade.into_iter().collect(),
        };
        let Some(stream) = streams.iter().find(|s| s.capability == PaintOrderStream::CAPABILITY) else {
            return legacy(None);
        };
        let rasters = u32::try_from(counts.rasters.len()).unwrap_or(u32::MAX);
        let order = PaintOrderStream::decode(&stream.payload).and_then(|o| {
            o.validate(|e| e.index() < counts.strokes, counts.fills, rasters, counts.texts)?;
            Ok(o)
        });
        let order = match order {
            Ok(o) => o,
            Err(e) => return legacy(Some(Degrade::StreamDamaged { id: stream.capability, rule: e.rule() })),
        };
        let mut batches = unlisted_first(counts, &order.runs);
        for r in &order.runs {
            push(&mut batches, PaintBatch { kind: r.kind, first: r.first, count: r.count });
        }
        if batches.len() > policy.max_batches {
            return legacy(Some(Degrade::TooManyBatches));
        }
        ScenePlan { batches, degrades: Vec::new() }
    }
}

/// Appends a batch, joining it to the last one when it continues the same kind without a gap.
fn push(batches: &mut Vec<PaintBatch>, b: PaintBatch) {
    match batches.last_mut() {
        Some(last) if last.kind == b.kind && last.first + last.count == b.first => last.count += b.count,
        _ => batches.push(b),
    }
}

/// The items no run in `listed` names, as batches in the legacy stage order.
fn unlisted_first(counts: &Counts, listed: &[PaintRun]) -> Vec<PaintBatch> {
    let gaps = |kind: ItemKind, limit: u32| -> Vec<(u32, u32)> {
        let mut spans: Vec<(u32, u32)> =
            listed.iter().filter(|r| r.kind == kind).map(|r| (r.first, r.first + r.count)).collect();
        spans.sort_unstable();
        let (mut out, mut at) = (Vec::new(), 0);
        for (first, end) in spans {
            if first > at {
                out.push((at, first.min(limit)));
            }
            at = at.max(end);
        }
        if at < limit {
            out.push((at, limit));
        }
        out
    };
    let rasters = |band: Band| -> Vec<PaintBatch> {
        let mut out = Vec::new();
        for (first, end) in gaps(ItemKind::Raster, counts.rasters.len() as u32) {
            for n in first..end {
                if counts.rasters[n as usize] == band {
                    push(&mut out, PaintBatch { kind: ItemKind::Raster, first: n, count: 1 });
                }
            }
        }
        out
    };
    let mut out = rasters(Band::Underlay);
    for (kind, limit) in
        [(ItemKind::Fill, counts.fills), (ItemKind::Stroke, counts.strokes), (ItemKind::Text, counts.texts)]
    {
        for (first, end) in gaps(kind, limit) {
            push(&mut out, PaintBatch { kind, first, count: end - first });
        }
    }
    for b in rasters(Band::Overlay) {
        push(&mut out, b);
    }
    out
}

/// One clip of the tree, with the stencil bits of its level.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipNode {
    /// The rule that decides what the rings enclose.
    pub rule: FillRule,
    /// The clip this one is intersected with.
    pub parent: Option<u32>,
    /// 1 for a root, one more than the parent otherwise. Equals the length of the chain.
    pub depth: u8,
    /// The stencil bit this level writes: `1 << (depth - 1)`.
    pub write_bit: u8,
    /// The bits of the levels above this one, which an item tests: `(1 << (depth - 1)) - 1`.
    pub chain_mask: u8,
    /// One ring per boundary entity, or `None` when the clip cannot be built. No tessellation: the viewer does it.
    pub rings: Option<Vec<Vec<[f64; 2]>>>,
}

/// Items `first` to `first + count - 1` of batch number `batch`, all inside the same clip (or none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClippedRun {
    /// The index into `ScenePlan::batches`.
    pub batch: usize,
    /// The number of the first item.
    pub first: u32,
    /// How many items, at least one.
    pub count: u32,
    /// The clip the items are drawn inside, `None` for unclipped items.
    pub clip: Option<u32>,
}

/// The clip tree of a page and the runs of each batch that are to be drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipTree {
    /// The clips, parents before children.
    pub nodes: Vec<ClipNode>,
    /// The runs to draw, in batch order. Items of an unbuildable clip are absent.
    pub runs: Vec<ClippedRun>,
    /// What was given up.
    pub degrades: Vec<Degrade>,
}

impl ClipTree {
    /// Works out the clip tree for `plan` from the clip stream (capability 26), if the page has one.
    ///
    /// `ring_of` gives the points of a ring entity. A clip with a ring `ring_of` cannot give, with fewer than 3
    /// points or with a coordinate that is not finite, draws nothing for its items or for those of its descendants,
    /// and is reported once as `ClipUnbuildable` against the outermost clip that failed. A clip stream that does
    /// not decode is reported as `StreamDamaged` and the items are drawn unclipped.
    pub fn build(
        streams: &[Stream],
        plan: &ScenePlan,
        ring_of: impl Fn(EntityId) -> Option<Vec<[f64; 2]>>,
    ) -> ClipTree {
        let whole = |degrades: Vec<Degrade>| ClipTree {
            nodes: Vec::new(),
            runs: plan
                .batches
                .iter()
                .enumerate()
                .map(|(batch, b)| ClippedRun { batch, first: b.first, count: b.count, clip: None })
                .collect(),
            degrades,
        };
        let Some(stream) = streams.iter().find(|s| s.capability == ClipStream::CAPABILITY) else {
            return whole(Vec::new());
        };
        let clips = match ClipStream::decode(&stream.payload) {
            Ok(c) => c,
            Err(e) => return whole(vec![Degrade::StreamDamaged { id: stream.capability, rule: e.rule() }]),
        };
        // `blocked[k]` is the outermost clip of k's chain that cannot be built, if there is one.
        let mut nodes: Vec<ClipNode> = Vec::with_capacity(clips.clips.len());
        let mut blocked: Vec<Option<u32>> = Vec::with_capacity(clips.clips.len());
        for (k, c) in clips.clips.iter().enumerate() {
            let rings = c
                .boundary
                .iter()
                .map(|e| ring_of(*e).filter(|r| r.len() >= 3 && r.iter().flatten().all(|v| v.is_finite())))
                .collect::<Option<Vec<_>>>();
            let parent_at = c.parent.map(|p| p as usize);
            // the decoder guarantees parents come first and the chain is at most MAX_CLIP_DEPTH (8) long
            let depth = parent_at.map_or(1, |p| nodes[p].depth + 1);
            blocked.push(parent_at.and_then(|p| blocked[p]).or(if rings.is_none() {
                Some(k as u32)
            } else {
                None
            }));
            nodes.push(ClipNode {
                rule: c.rule,
                parent: c.parent,
                depth,
                write_bit: 1 << (depth - 1),
                chain_mask: (1 << (depth - 1)) - 1,
                rings,
            });
        }
        let mut spans_of = HashMap::new();
        for kind in [ItemKind::Stroke, ItemKind::Fill, ItemKind::Raster, ItemKind::Text] {
            let mut spans: Vec<(u32, u32, u32)> = clips
                .runs
                .iter()
                .filter(|r| r.kind == kind)
                .map(|r| (r.first, r.first + r.count, r.clip))
                .collect();
            spans.sort_unstable();
            spans_of.insert(kind, spans);
        }
        let mut lost = vec![0u32; nodes.len()];
        let mut runs: Vec<ClippedRun> = Vec::new();
        for (batch, b) in plan.batches.iter().enumerate() {
            let spans = &spans_of[&b.kind];
            let (mut at, end) = (b.first, b.first + b.count);
            while at < end {
                let next = spans.get(spans.partition_point(|s| s.1 <= at)).copied();
                let (stop, clip) = match next {
                    Some((first, last, clip)) if first <= at => (last.min(end), Some(clip)),
                    Some((first, _, _)) => (first.min(end), None),
                    None => (end, None),
                };
                let count = stop - at;
                match clip.and_then(|c| blocked[c as usize]) {
                    Some(root) => lost[root as usize] += count,
                    None => match runs.last_mut() {
                        Some(last)
                            if last.batch == batch && last.clip == clip && last.first + last.count == at =>
                        {
                            last.count += count
                        }
                        _ => runs.push(ClippedRun { batch, first: at, count, clip }),
                    },
                }
                at = stop;
            }
        }
        let degrades = (0..nodes.len())
            .filter(|k| blocked[*k] == Some(*k as u32))
            .map(|k| Degrade::ClipUnbuildable { clip: k as u32, items: lost[k] })
            .collect();
        ClipTree { nodes, runs, degrades }
    }
}
