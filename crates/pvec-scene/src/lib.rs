//! The paint order of a page, worked out in one place (issue 296, `docs/PAINT.md` sections 2.2 and 5.3).

use pvec_capabilities::CapabilityId;
use pvec_components::{
    Band, FillRule, Gradient, ItemKind, PaintOrderStream, PaintRun, Stream, Tiling, TilingPrimitive,
};

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
    /// A tiling needs more copies of its cell than `MAX_TILE_COPIES`, so it is left to the flat colour.
    TilingTooLarge,
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

/// The most texels one gradient row may have, not counting the two guard texels.
pub const MAX_GRADIENT_TEXELS: usize = 65_536;

/// The most copies of one cell that a tiling may expand to.
pub const MAX_TILE_COPIES: u32 = 64;

/// A tile primitive kind: a polygon filled with the non-zero rule.
pub const TILE_POLYGON_NONZERO: u32 = 1;
/// A tile primitive kind: a polygon filled with the even-odd rule.
pub const TILE_POLYGON_EVENODD: u32 = 2;
/// A tile primitive kind: a stroked polyline.
pub const TILE_POLYLINE: u32 = 3;

/// The primitives of one tiling cell and its neighbour copies, as parallel plain vectors.
#[derive(Clone, Debug, PartialEq)]
pub struct TileSample {
    /// Copies of the cell along x and along y, `ceil(cell extent / step)` each and at least one.
    pub copies: [u32; 2],
    /// The kind of each primitive, one of the `TILE_*` words.
    pub kinds: Vec<u32>,
    /// The `0xRRGGBBAA` word of each primitive, straight alpha, passed through untouched.
    pub colours: Vec<u32>,
    /// The line width of each primitive, zero for a polygon.
    pub widths: Vec<f32>,
    /// For primitive `n`, its points are `starts[n]..starts[n + 1]`; one more entry than there are primitives.
    pub starts: Vec<u32>,
    /// The points as x, y pairs in pattern space, copy offsets already added.
    pub points: Vec<f32>,
}

/// A float for the wire: finite whatever the input, since f64 values past `f32::MAX` would become infinite.
fn finite32(v: f64) -> f32 {
    if v.is_nan() { 0.0 } else { v.clamp(-f64::from(f32::MAX), f64::from(f32::MAX)) as f32 }
}

/// Resamples a gradient to a row of RGBA texels for a one-lookup shader.
///
/// The row is `texels + 2` texels of four `f32` each, straight alpha, 0 to 1. Texel 0 and the last texel are
/// guards for a parameter outside 0 to 1: the first stop's colour when `extend_before` (the last stop's when
/// `extend_after`) and fully transparent otherwise. Texels `1..=texels` sample the parameter `n / (texels - 1)`,
/// so the first and last of them are exactly the end values. A parameter before the first stop or after the last
/// one takes the same flag, which folds the extend rule into the data. Between stops the channels are
/// interpolated linearly; at a repeated offset the later stop wins from that offset on, a hard step. `texels` is
/// kept between 2 and `MAX_GRADIENT_TEXELS`. A gradient with no stops is transparent. Alpha is never
/// premultiplied.
pub fn sample_gradient(g: &Gradient, texels: usize) -> Vec<f32> {
    const CLEAR: [f32; 4] = [0.0; 4];
    let texels = texels.clamp(2, MAX_GRADIENT_TEXELS);
    let colour = |rgba: u32| [24, 16, 8, 0].map(|shift| ((rgba >> shift) & 0xFF) as f32 / 255.0);
    let (Some(first), Some(last)) = (g.stops.first(), g.stops.last()) else {
        return vec![0.0; (texels + 2) * 4];
    };
    let before = if g.extend_before { colour(first.rgba) } else { CLEAR };
    let after = if g.extend_after { colour(last.rgba) } else { CLEAR };
    let at = |t: f64| -> [f32; 4] {
        let k = g.stops.partition_point(|s| s.offset <= t);
        if k == 0 {
            return before;
        }
        if k == g.stops.len() {
            return if t <= last.offset { colour(last.rgba) } else { after };
        }
        let (a, b) = (&g.stops[k - 1], &g.stops[k]);
        let span = b.offset - a.offset;
        let u = if span > 0.0 { ((t - a.offset) / span).clamp(0.0, 1.0) } else { 1.0 };
        let (ca, cb) = (colour(a.rgba), colour(b.rgba));
        [0, 1, 2, 3].map(|c| finite32(f64::from(ca[c]) + (f64::from(cb[c]) - f64::from(ca[c])) * u))
    };
    let mut row = Vec::with_capacity((texels + 2) * 4);
    row.extend_from_slice(&before);
    for n in 0..texels {
        row.extend_from_slice(&at(n as f64 / (texels - 1) as f64));
    }
    row.extend_from_slice(&after);
    row
}

/// Expands a tiling to the primitives of one cell and the neighbour copies that reach into it.
///
/// A cell at offset `(i * x_step, j * y_step)` touches the step box at the origin when its extent is wider than
/// the step, so along each axis there are `ceil(extent / step)` copies, `i` from `1 - n` to 0, and at least one.
/// Copies come in reading order (`j` outer, `i` inner), so the cell itself is painted last and every copy keeps
/// the primitive order of the cell. Points are pattern space with the copy offset added; colours, rules and
/// widths pass through untouched. More than `MAX_TILE_COPIES` copies is `Degrade::TilingTooLarge`. Every number
/// is finite: values past `f32::MAX` are clamped to it.
pub fn sample_tiling(t: &Tiling) -> Result<TileSample, Degrade> {
    let n = |extent: f64, step: f64| (extent / step).ceil().max(1.0);
    let (nx, ny) = (n(t.cell[2] - t.cell[0], t.x_step), n(t.cell[3] - t.cell[1], t.y_step));
    if nx * ny > f64::from(MAX_TILE_COPIES) {
        return Err(Degrade::TilingTooLarge);
    }
    let (nx, ny) = (nx as i64, ny as i64);
    let mut s = TileSample {
        copies: [nx as u32, ny as u32],
        kinds: Vec::new(),
        colours: Vec::new(),
        widths: Vec::new(),
        starts: vec![0],
        points: Vec::new(),
    };
    for j in (1 - ny)..=0 {
        for i in (1 - nx)..=0 {
            let (dx, dy) = (i as f64 * t.x_step, j as f64 * t.y_step);
            for p in &t.primitives {
                let (kind, rgba, width, points) = match p {
                    TilingPrimitive::Polygon { rule: FillRule::NonZero, rgba, points } => {
                        (TILE_POLYGON_NONZERO, *rgba, 0.0, points)
                    }
                    TilingPrimitive::Polygon { rule: FillRule::EvenOdd, rgba, points } => {
                        (TILE_POLYGON_EVENODD, *rgba, 0.0, points)
                    }
                    TilingPrimitive::Polyline { rgba, width, points } => {
                        (TILE_POLYLINE, *rgba, *width, points)
                    }
                };
                s.kinds.push(kind);
                s.colours.push(rgba);
                s.widths.push(finite32(width));
                for q in points {
                    s.points.extend([finite32(q[0] + dx), finite32(q[1] + dy)]);
                }
                s.starts.push((s.points.len() / 2) as u32);
            }
        }
    }
    Ok(s)
}
