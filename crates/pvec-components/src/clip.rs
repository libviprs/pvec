//! Capability 26, `clip` (issue 286, `docs/PAINT.md` section 2.5): clip regions made of rings of entities that
//! already exist, a rule, a parent, and the paint items each region applies to. The region of a clip is the area
//! its rings enclose under its rule, intersected with its parent's. The rings are ordinary entities (written
//! hidden by the line style stream), so the geometry stays whole and the clip is a statement about display.

use crate::error::ComponentError as E;
use crate::fill_paint::FillRule;
use crate::frame::Stream;
use crate::paint::{ItemKind, check_spans, items_exist};
use crate::payloads::{R, W, into_stream};
use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;

type Res<T> = Result<T, E>;

/// Most clips in one stream.
pub const MAX_CLIPS: usize = 65_536;
/// Most runs in one stream.
pub const MAX_CLIP_RUNS: usize = 1 << 20;
/// Most entities in the boundary of one clip.
pub const MAX_CLIP_BOUNDARY: usize = 4_096;
/// The deepest chain of nested clips, the number of bits a stencil buffer gives a renderer to intersect with.
pub const MAX_CLIP_DEPTH: usize = 8;

const NO_PARENT: u32 = u32::MAX;

/// One clip region.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipRecord {
    /// The rule that decides what the rings enclose.
    pub rule: FillRule,
    /// The clip this one is intersected with, an earlier one.
    pub parent: Option<u32>,
    /// The entities of the rings, one to 4,096, by reference.
    pub boundary: Vec<EntityId>,
}

/// The items `first` to `first + count - 1` of one kind, drawn only inside a clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipRun {
    /// What the items are.
    pub kind: ItemKind,
    /// The number of the first item.
    pub first: u32,
    /// How many items, at least one.
    pub count: u32,
    /// The index of the clip.
    pub clip: u32,
}

/// Clip regions and the items they apply to. An item no run names is not clipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipStream {
    /// The clips, parents before children.
    pub clips: Vec<ClipRecord>,
    /// The runs.
    pub runs: Vec<ClipRun>,
}

impl ClipStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::CLIP;

    /// The clip a paint item is drawn inside, if any.
    pub fn clip_of(&self, kind: ItemKind, item: u32) -> Option<u32> {
        self.runs
            .iter()
            .find(|r| r.kind == kind && item >= r.first && item - r.first < r.count)
            .map(|r| r.clip)
    }

    /// The chain of a clip from the outermost to itself: the regions whose intersection is the clip's region.
    pub fn chain_of(&self, clip: u32) -> Vec<u32> {
        let mut chain = vec![clip];
        let mut at = clip;
        while let Some(parent) = self.clips.get(at as usize).and_then(|c| c.parent) {
            if chain.len() > self.clips.len() {
                break;
            }
            chain.push(parent);
            at = parent;
        }
        chain.reverse();
        chain
    }

    /// COMP-25 and COMP-26 (and the shape rules of the clips and the runs), shared by the writer and the reader.
    fn check(&self) -> Res<()> {
        if self.clips.len() > MAX_CLIPS || self.runs.len() > MAX_CLIP_RUNS {
            return Err(E::TooMany);
        }
        let mut depth = Vec::with_capacity(self.clips.len());
        for (k, c) in self.clips.iter().enumerate() {
            if c.boundary.len() > MAX_CLIP_BOUNDARY {
                return Err(E::TooMany);
            }
            if c.boundary.is_empty() {
                return Err(E::BadCount);
            }
            let d = match c.parent {
                None => 1,
                Some(p) if (p as usize) < k => depth[p as usize] + 1,
                Some(_) => return Err(E::ClipCycle),
            };
            if d > MAX_CLIP_DEPTH {
                return Err(E::ClipTooDeep);
            }
            depth.push(d);
        }
        if self.runs.iter().any(|r| r.clip as usize >= self.clips.len()) {
            return Err(E::IndexOutOfRange);
        }
        check_spans(self.runs.iter().map(|r| (r.kind, r.first, r.count)))
    }

    /// COMP-22: the entities of the rings exist (`exists`, a hidden entity does), and so does every item the runs
    /// name (`drawn`: a stroke must be visible; the counts are the hatch records, the rasters and the text
    /// records).
    pub fn validate(
        &self,
        exists: impl Fn(EntityId) -> bool,
        drawn: impl Fn(EntityId) -> bool,
        fills: u32,
        rasters: u32,
        texts: u32,
    ) -> Res<()> {
        if self.clips.iter().flat_map(|c| &c.boundary).any(|e| !exists(*e)) {
            return Err(E::MissingItem);
        }
        items_exist(self.runs.iter().map(|r| (r.kind, r.first, r.count)), drawn, [fills, rasters, texts])
    }

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        self.check()?;
        let mut w = W::default();
        w.u32(self.clips.len() as u32);
        w.u32(self.runs.len() as u32);
        for c in &self.clips {
            w.u8(c.rule.tag());
            w.0.extend_from_slice(&[0; 3]);
            w.u32(c.parent.unwrap_or(NO_PARENT));
            w.u32(1);
            w.u32(c.boundary.len() as u32);
            c.boundary.iter().for_each(|e| w.u32(e.index()));
        }
        for r in &self.runs {
            w.u32(r.kind.tag());
            w.u32(r.first);
            w.u32(r.count);
            w.u32(r.clip);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<ClipStream> {
        let mut r = R::new(bytes);
        let (clip_n, run_n) = (r.u32()? as usize, r.u32()? as usize);
        if clip_n > MAX_CLIPS || run_n > MAX_CLIP_RUNS {
            return Err(E::TooMany);
        }
        // a clip is at least 16 bytes and one entity, a run is 16
        if clip_n * 20 + run_n * 16 > r.left() {
            return Err(E::Truncated);
        }
        let mut clips = Vec::with_capacity(clip_n);
        for _ in 0..clip_n {
            let rule = r.u8()?;
            if r.take(3)? != [0, 0, 0] {
                return Err(E::ReservedNotZero);
            }
            let rule = FillRule::from_tag(rule)?;
            let parent = match r.u32()? {
                NO_PARENT => None,
                p => Some(p),
            };
            if r.u32()? != 1 {
                return Err(E::EmbeddedGeometry);
            }
            let n = r.u32()? as usize;
            if n > MAX_CLIP_BOUNDARY {
                return Err(E::TooMany);
            }
            if n * 4 > r.left() {
                return Err(E::Truncated);
            }
            let boundary = (0..n).map(|_| r.entity()).collect::<Res<Vec<EntityId>>>()?;
            clips.push(ClipRecord { rule, parent, boundary });
        }
        let mut runs = Vec::with_capacity(run_n);
        for _ in 0..run_n {
            let kind = ItemKind::from_tag(r.u32()?)?;
            runs.push(ClipRun { kind, first: r.u32()?, count: r.u32()?, clip: r.u32()? });
        }
        r.done()?;
        let s = ClipStream { clips, runs };
        s.check()?;
        Ok(s)
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}
