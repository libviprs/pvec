//! The first two streams of the paint model (issue 286, `docs/PAINT.md` sections 2.1 to 2.3): capability 23
//! `paint_order`, the order the items of a page were painted in, and capability 24 `stroke_paint`, a colour
//! per entity. Both name things that already exist in other streams or in the geometry and embed no curve.

use crate::error::ComponentError as E;
use crate::frame::Stream;
use crate::payloads::{R, W, into_stream};
use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;

type Res<T> = Result<T, E>;

/// Most runs in a paint order.
pub const MAX_PAINT_RUNS: usize = 1 << 20;
/// Most items a paint order names once its runs are expanded.
pub const MAX_PAINT_ITEMS: u64 = 1 << 24;
/// Most colours in a stroke paint table.
pub const MAX_STROKE_COLOURS: usize = 65_536;
/// Most entity and colour pairs in a stroke paint stream.
pub const MAX_STROKE_ASSIGNMENTS: usize = 1 << 24;

/// The four kinds of thing a viewer draws as a unit (`docs/PAINT.md` section 2.1). The streams that talk about
/// paint items (23 now, 26 later) share this vocabulary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A stroke, numbered by the `EntityId` of a visible entity.
    Stroke,
    /// A fill, numbered by the index of a record of the hatch stream.
    Fill,
    /// A raster, numbered by its `RasterId`.
    Raster,
    /// Text, numbered by the index of a record of the text stream.
    Text,
}

impl ItemKind {
    fn tag(self) -> u32 {
        match self {
            ItemKind::Stroke => 1,
            ItemKind::Fill => 2,
            ItemKind::Raster => 3,
            ItemKind::Text => 4,
        }
    }

    fn from_tag(tag: u32) -> Res<ItemKind> {
        match tag {
            1 => Ok(ItemKind::Stroke),
            2 => Ok(ItemKind::Fill),
            3 => Ok(ItemKind::Raster),
            4 => Ok(ItemKind::Text),
            t => Err(E::BadTag(t)),
        }
    }
}

/// The items `first` to `first + count - 1` of one kind, in ascending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaintRun {
    /// What the items are.
    pub kind: ItemKind,
    /// The number of the first item.
    pub first: u32,
    /// How many items, at least one.
    pub count: u32,
}

/// The order a page was painted in, back to front: the runs in file order, the items of a run ascending.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintOrderStream {
    /// The runs.
    pub runs: Vec<PaintRun>,
}

impl PaintOrderStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::PAINT_ORDER;

    /// The items in paint order, back to front.
    pub fn items(&self) -> impl Iterator<Item = (ItemKind, u32)> + '_ {
        self.runs.iter().flat_map(|r| (r.first..r.first + r.count).map(move |n| (r.kind, n)))
    }

    /// The shape rules, shared by the writer and the reader: COMP-03, 05, 08 and 21.
    fn check(&self) -> Res<()> {
        if self.runs.len() > MAX_PAINT_RUNS {
            return Err(E::TooMany);
        }
        let mut total = 0u64;
        let mut spans: [Vec<(u32, u32)>; 4] = Default::default();
        for r in &self.runs {
            if r.count == 0 {
                return Err(E::BadCount);
            }
            let end = match (r.first.checked_add(r.count), r.kind) {
                (Some(end), _) => end,
                (None, ItemKind::Stroke) => return Err(E::BadEntityId(u32::MAX)),
                (None, _) => return Err(E::TooMany),
            };
            if r.kind == ItemKind::Stroke {
                EntityId::new(end - 1).map_err(|_| E::BadEntityId(end - 1))?;
            }
            total += u64::from(r.count);
            if total > MAX_PAINT_ITEMS {
                return Err(E::TooMany);
            }
            spans[r.kind.tag() as usize - 1].push((r.first, end));
        }
        for kind in &mut spans {
            kind.sort_unstable();
            if kind.windows(2).any(|w| w[1].0 < w[0].1) {
                return Err(E::DuplicateItem);
            }
        }
        Ok(())
    }

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        self.check()?;
        let mut w = W::default();
        w.u32(self.runs.len() as u32);
        for r in &self.runs {
            w.u32(r.kind.tag());
            w.u32(r.first);
            w.u32(r.count);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<PaintOrderStream> {
        let mut r = R::new(bytes);
        let n = r.count(MAX_PAINT_RUNS, 12)?;
        let mut runs = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = ItemKind::from_tag(r.u32()?)?;
            runs.push(PaintRun { kind, first: r.u32()?, count: r.u32()? });
        }
        r.done()?;
        let s = PaintOrderStream { runs };
        s.check()?;
        Ok(s)
    }

    /// COMP-22: every item named exists. `drawn` says whether an entity is in the geometry and visible (a hidden
    /// entity cannot be painted), and the counts are the records of the hatch stream, the rasters and the
    /// records of the text stream.
    pub fn validate(
        &self,
        drawn: impl Fn(EntityId) -> bool,
        fills: u32,
        rasters: u32,
        texts: u32,
    ) -> Res<()> {
        for r in &self.runs {
            let end = r.first.checked_add(r.count).ok_or(E::MissingItem)?;
            let limit = match r.kind {
                ItemKind::Stroke => {
                    for n in r.first..end {
                        if !EntityId::new(n).is_ok_and(&drawn) {
                            return Err(E::MissingItem);
                        }
                    }
                    continue;
                }
                ItemKind::Fill => fills,
                ItemKind::Raster => rasters,
                ItemKind::Text => texts,
            };
            if end > limit {
                return Err(E::MissingItem);
            }
        }
        Ok(())
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

/// A colour per entity: a table of RGBA words and pairs of an entity and a colour index. An entity with no pair
/// has no colour in the file and the viewer's own style applies. Text takes the colour of its anchor entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrokePaintStream {
    /// `0xRRGGBBAA`, straight alpha, sRGB, exactly as in the hatch stream. The alpha byte is the stroke alpha.
    pub colours: Vec<u32>,
    /// Pairs of an entity and an index into `colours`, entities strictly ascending.
    pub assignments: Vec<(EntityId, u32)>,
}

impl StrokePaintStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::STROKE_PAINT;

    /// The colour assigned to `entity`, if any.
    pub fn colour_of(&self, entity: EntityId) -> Option<u32> {
        let at = self.assignments.binary_search_by_key(&entity.index(), |(e, _)| e.index()).ok()?;
        self.colours.get(self.assignments[at].1 as usize).copied()
    }

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.colours.len() > MAX_STROKE_COLOURS || self.assignments.len() > MAX_STROKE_ASSIGNMENTS {
            return Err(E::TooMany);
        }
        if self.colours.is_empty() {
            return Err(E::BadCount);
        }
        let mut w = W::default();
        w.u32(self.colours.len() as u32);
        w.u32(self.assignments.len() as u32);
        for c in &self.colours {
            w.u32(*c);
        }
        let mut last = None;
        for (e, i) in &self.assignments {
            if *i as usize >= self.colours.len() {
                return Err(E::IndexOutOfRange);
            }
            if last.is_some_and(|l| l >= e.index()) {
                return Err(E::NotAscending);
            }
            last = Some(e.index());
            w.u32(e.index());
            w.u32(*i);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<StrokePaintStream> {
        let mut r = R::new(bytes);
        let (colour_n, assign_n) = (r.u32()? as usize, r.u32()? as usize);
        if colour_n > MAX_STROKE_COLOURS || assign_n > MAX_STROKE_ASSIGNMENTS {
            return Err(E::TooMany);
        }
        if colour_n == 0 {
            return Err(E::BadCount);
        }
        if colour_n * 4 + assign_n * 8 > r.left() {
            return Err(E::Truncated);
        }
        let colours = (0..colour_n).map(|_| r.u32()).collect::<Res<Vec<u32>>>()?;
        let mut assignments = Vec::with_capacity(assign_n);
        for _ in 0..assign_n {
            let e = r.entity()?;
            let i = r.u32()?;
            if i as usize >= colour_n {
                return Err(E::IndexOutOfRange);
            }
            if assignments.last().is_some_and(|(l, _): &(EntityId, u32)| l.index() >= e.index()) {
                return Err(E::NotAscending);
            }
            assignments.push((e, i));
        }
        r.done()?;
        Ok(StrokePaintStream { colours, assignments })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}
