//! The paint order of a page, worked out in one place (issue 296, `docs/PAINT.md` sections 2.2 and 5.3).

use pvec_capabilities::CapabilityId;
use pvec_components::{Band, ItemKind, PaintOrderStream, PaintRun, Stream};

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
