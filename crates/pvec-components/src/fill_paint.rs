//! Capability 25, `fill_paint` (issue 286, `docs/PAINT.md` section 2.4): the fill rule of a hatch record, and a
//! gradient or a tiling pattern in place of its flat colour. The flat colour stays in the hatch record as the
//! fallback of a reader that does not know 25. A gradient is sampled stops and never a PDF function; a tiling
//! cell is a handful of primitives in pattern space, bounded hard (COMP-27).

use crate::error::ComponentError as E;
use crate::frame::Stream;
use crate::payloads::{R, W, finite, into_stream};
use pvec_capabilities::{CapabilityId, Registry, ids};

type Res<T> = Result<T, E>;

/// Most gradients in one stream.
pub const MAX_GRADIENTS: usize = 4_096;
/// Most tilings in one stream.
pub const MAX_TILINGS: usize = 1_024;
/// Most hatch assignments in one stream.
pub const MAX_FILL_ASSIGNMENTS: usize = 1 << 20;
/// Most stops in one gradient.
pub const MAX_STOPS: usize = 256;
/// Most primitives in one tiling.
pub const MAX_TILING_PRIMITIVES: usize = 256;
/// Most points in one primitive.
pub const MAX_PRIMITIVE_POINTS: usize = 4_096;
/// Most points in all the primitives of one tiling (COMP-27).
pub const MAX_TILING_POINTS: usize = 8_192;

/// How the winding number of a ring set decides what is inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillRule {
    /// Inside where the winding number is not zero.
    NonZero,
    /// Inside where the winding number is odd.
    EvenOdd,
}

impl FillRule {
    pub(crate) fn tag(self) -> u8 {
        u8::from(self == FillRule::EvenOdd)
    }

    pub(crate) fn from_tag(tag: u8) -> Res<FillRule> {
        match tag {
            0 => Ok(FillRule::NonZero),
            1 => Ok(FillRule::EvenOdd),
            t => Err(E::BadTag(u32::from(t))),
        }
    }
}

/// What a hatch record is filled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillSource {
    /// The flat colour of the hatch record itself.
    Own,
    /// The gradient with this index.
    Gradient(u32),
    /// The tiling with this index.
    Tiling(u32),
}

/// The rule and the paint of one hatch record. A record with no assignment keeps what a viewer did before this
/// stream existed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FillAssignment {
    /// The index of the record in the hatch stream.
    pub hatch: u32,
    /// The fill rule, applied to all the boundary rings of the record together.
    pub rule: FillRule,
    /// What the area is filled with.
    pub paint: FillSource,
}

/// A colour at a point along a gradient.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stop {
    /// 0 to 1, non-decreasing along the gradient. A repeated offset is a hard step.
    pub offset: f64,
    /// `0xRRGGBBAA`, straight alpha, sRGB.
    pub rgba: u32,
}

/// The geometry of a gradient in gradient space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientCoords {
    /// The parameter is the projection on the segment from `(x0, y0)` to `(x1, y1)`.
    Axial {
        /// Start x.
        x0: f64,
        /// Start y.
        y0: f64,
        /// End x.
        x1: f64,
        /// End y.
        y1: f64,
    },
    /// The circle family from `(x0, y0, r0)` to `(x1, y1, r1)`, the PDF type 3 rule.
    Radial {
        /// Start centre x.
        x0: f64,
        /// Start centre y.
        y0: f64,
        /// Start radius.
        r0: f64,
        /// End centre x.
        x1: f64,
        /// End centre y.
        y1: f64,
        /// End radius.
        r1: f64,
    },
}

/// An axial or radial gradient as sampled stops.
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    /// Where the colours run.
    pub coords: GradientCoords,
    /// Paint before the first stop.
    pub extend_before: bool,
    /// Paint after the last stop.
    pub extend_after: bool,
    /// `[a b c d tx ty]`, from gradient space to source space. Not singular.
    pub transform: [f64; 6],
    /// Two to 256 stops.
    pub stops: Vec<Stop>,
}

/// One piece of a tiling cell, in pattern space.
#[derive(Clone, Debug, PartialEq)]
pub enum TilingPrimitive {
    /// A filled polygon of 3 to 4,096 points.
    Polygon {
        /// The fill rule.
        rule: FillRule,
        /// The colour, ignored in an uncoloured tiling.
        rgba: u32,
        /// The points.
        points: Vec<[f64; 2]>,
    },
    /// A stroked polyline of 2 to 4,096 points.
    Polyline {
        /// The colour, ignored in an uncoloured tiling.
        rgba: u32,
        /// The line width, finite and not negative.
        width: f64,
        /// The points.
        points: Vec<[f64; 2]>,
    },
}

/// A tiling pattern: a cell of primitives repeated at a step.
#[derive(Clone, Debug, PartialEq)]
pub struct Tiling {
    /// Paint type 1 (coloured) when true, 2 (uncoloured, painted in the colour of the hatch record) when false.
    pub coloured: bool,
    /// The horizontal step, finite and positive.
    pub x_step: f64,
    /// The vertical step, finite and positive.
    pub y_step: f64,
    /// `x0 y0 x1 y1` of the cell, non-empty.
    pub cell: [f64; 4],
    /// `[a b c d tx ty]`, from pattern space to source space. Not singular.
    pub transform: [f64; 6],
    /// One to 256 primitives, at most 8,192 points in all.
    pub primitives: Vec<TilingPrimitive>,
}

/// Fill rules, gradients and tilings, keyed by hatch record.
#[derive(Clone, Debug, PartialEq)]
pub struct FillPaintStream {
    /// The gradients, at most 4,096.
    pub gradients: Vec<Gradient>,
    /// The tilings, at most 1,024.
    pub tilings: Vec<Tiling>,
    /// The assignments, hatch records strictly ascending, at most 2^20.
    pub assignments: Vec<FillAssignment>,
}

fn check_transform(t: &[f64; 6]) -> Res<()> {
    t.iter().try_for_each(|v| finite(*v).map(drop))?;
    let det = t[0] * t[3] - t[1] * t[2];
    if det == 0.0 || !det.is_finite() { Err(E::DegeneratePaint) } else { Ok(()) }
}

fn check_gradient(g: &Gradient) -> Res<()> {
    if g.stops.len() > MAX_STOPS {
        return Err(E::TooMany);
    }
    if g.stops.len() < 2 {
        return Err(E::BadCount);
    }
    let coords: &[f64] = match &g.coords {
        GradientCoords::Axial { x0, y0, x1, y1 } => &[*x0, *y0, *x1, *y1],
        GradientCoords::Radial { x0, y0, r0, x1, y1, r1 } => &[*x0, *y0, *r0, *x1, *y1, *r1],
    };
    coords.iter().try_for_each(|v| finite(*v).map(drop))?;
    g.stops.iter().try_for_each(|s| finite(s.offset).map(drop))?;
    check_transform(&g.transform)?;
    let degenerate = match g.coords {
        GradientCoords::Axial { x0, y0, x1, y1 } => x0 == x1 && y0 == y1,
        GradientCoords::Radial { x0, y0, r0, x1, y1, r1 } => {
            r0 < 0.0 || r1 < 0.0 || (x0 == x1 && y0 == y1 && r0 == r1)
        }
    };
    if degenerate {
        return Err(E::DegeneratePaint);
    }
    let mut last = 0.0;
    for s in &g.stops {
        if !(0.0..=1.0).contains(&s.offset) || s.offset < last {
            return Err(E::BadStops);
        }
        last = s.offset;
    }
    Ok(())
}

fn check_tiling(t: &Tiling) -> Res<()> {
    if t.primitives.len() > MAX_TILING_PRIMITIVES {
        return Err(E::TooMany);
    }
    if t.primitives.is_empty() {
        return Err(E::BadCount);
    }
    for v in [t.x_step, t.y_step] {
        if !v.is_finite() || v <= 0.0 {
            return Err(E::BadNumber);
        }
    }
    t.cell.iter().try_for_each(|v| finite(*v).map(drop))?;
    if t.cell[2] <= t.cell[0] || t.cell[3] <= t.cell[1] {
        return Err(E::BadNumber);
    }
    check_transform(&t.transform)?;
    let mut total = 0usize;
    for p in &t.primitives {
        let (points, least) = match p {
            TilingPrimitive::Polygon { points, .. } => (points, 3),
            TilingPrimitive::Polyline { width, points, .. } => {
                if !width.is_finite() || *width < 0.0 {
                    return Err(E::BadNumber);
                }
                (points, 2)
            }
        };
        if points.len() > MAX_PRIMITIVE_POINTS {
            return Err(E::TooMany);
        }
        if points.len() < least {
            return Err(E::BadCount);
        }
        points.iter().try_for_each(|p| p.iter().try_for_each(|v| finite(*v).map(drop)))?;
        total += points.len();
        if total > MAX_TILING_POINTS {
            return Err(E::TilingTooLarge);
        }
    }
    Ok(())
}

fn write_points(w: &mut W, points: &[[f64; 2]]) {
    for p in points {
        w.f64(p[0]);
        w.f64(p[1]);
    }
}

fn read_f64s<const N: usize>(r: &mut R) -> Res<[f64; N]> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = r.f64()?;
    }
    Ok(out)
}

fn read_points(r: &mut R, n: usize) -> Res<Vec<[f64; 2]>> {
    if n * 16 > r.left() {
        return Err(E::Truncated);
    }
    (0..n).map(|_| read_f64s::<2>(r)).collect()
}

fn read_u16(r: &mut R) -> Res<usize> {
    let b = r.take(2)?;
    Ok(usize::from(u16::from_le_bytes([b[0], b[1]])))
}

impl FillPaintStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::FILL_PAINT;

    /// The assignment of a hatch record, if it has one.
    pub fn assignment_of(&self, hatch: u32) -> Option<&FillAssignment> {
        self.assignments.binary_search_by_key(&hatch, |a| a.hatch).ok().map(|i| &self.assignments[i])
    }

    fn check(&self) -> Res<()> {
        if self.gradients.len() > MAX_GRADIENTS
            || self.tilings.len() > MAX_TILINGS
            || self.assignments.len() > MAX_FILL_ASSIGNMENTS
        {
            return Err(E::TooMany);
        }
        self.gradients.iter().try_for_each(check_gradient)?;
        self.tilings.iter().try_for_each(check_tiling)?;
        let mut last = None;
        for a in &self.assignments {
            if last.is_some_and(|l| l >= a.hatch) {
                return Err(E::NotAscending);
            }
            last = Some(a.hatch);
            let (index, limit) = match a.paint {
                FillSource::Own => continue,
                FillSource::Gradient(i) => (i, self.gradients.len()),
                FillSource::Tiling(i) => (i, self.tilings.len()),
            };
            if index as usize >= limit {
                return Err(E::IndexOutOfRange);
            }
        }
        Ok(())
    }

    /// COMP-22: every assignment names a record of the hatch stream, which has `hatch_records` of them.
    pub fn validate(&self, hatch_records: u32) -> Res<()> {
        if self.assignments.iter().any(|a| a.hatch >= hatch_records) { Err(E::MissingItem) } else { Ok(()) }
    }

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        self.check()?;
        let mut w = W::default();
        w.u32(self.gradients.len() as u32);
        w.u32(self.tilings.len() as u32);
        w.u32(self.assignments.len() as u32);
        for g in &self.gradients {
            let (kind, coords): (u8, &[f64]) = match &g.coords {
                GradientCoords::Axial { x0, y0, x1, y1 } => (1, &[*x0, *y0, *x1, *y1]),
                GradientCoords::Radial { x0, y0, r0, x1, y1, r1 } => (2, &[*x0, *y0, *r0, *x1, *y1, *r1]),
            };
            w.u8(kind);
            w.u8(u8::from(g.extend_before) | u8::from(g.extend_after) << 1);
            w.0.extend_from_slice(&(g.stops.len() as u16).to_le_bytes());
            g.transform.iter().chain(coords).for_each(|v| w.f64(*v));
            for s in &g.stops {
                w.f64(s.offset);
                w.u32(s.rgba);
            }
        }
        for t in &self.tilings {
            w.u8(if t.coloured { 1 } else { 2 });
            w.u8(0);
            w.0.extend_from_slice(&(t.primitives.len() as u16).to_le_bytes());
            [t.x_step, t.y_step].iter().chain(&t.cell).chain(&t.transform).for_each(|v| w.f64(*v));
            for p in &t.primitives {
                match p {
                    TilingPrimitive::Polygon { rule, rgba, points } => {
                        w.u8(1);
                        w.u8(rule.tag());
                        w.0.extend_from_slice(&(points.len() as u16).to_le_bytes());
                        w.u32(*rgba);
                        w.f64(0.0);
                        write_points(&mut w, points);
                    }
                    TilingPrimitive::Polyline { rgba, width, points } => {
                        w.u8(2);
                        w.u8(0);
                        w.0.extend_from_slice(&(points.len() as u16).to_le_bytes());
                        w.u32(*rgba);
                        w.f64(*width);
                        write_points(&mut w, points);
                    }
                }
            }
        }
        for a in &self.assignments {
            w.u32(a.hatch);
            w.u8(a.rule.tag());
            let (paint, index) = match a.paint {
                FillSource::Own => (0, 0),
                FillSource::Gradient(i) => (1, i),
                FillSource::Tiling(i) => (2, i),
            };
            w.u8(paint);
            w.0.extend_from_slice(&[0, 0]);
            w.u32(index);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<FillPaintStream> {
        // The smallest a gradient is on the wire: head, transform, an axial's coordinates and two stops.
        const MIN_GRADIENT: usize = 4 + 48 + 32 + 24;
        // The smallest a tiling is on the wire: its head and one two point polyline.
        const MIN_TILING: usize = 100 + 16 + 32;
        let mut r = R::new(bytes);
        let (gn, tn, an) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
        if gn > MAX_GRADIENTS || tn > MAX_TILINGS || an > MAX_FILL_ASSIGNMENTS {
            return Err(E::TooMany);
        }
        if gn * MIN_GRADIENT + tn * MIN_TILING + an * 12 > r.left() {
            return Err(E::Truncated);
        }
        let mut gradients = Vec::with_capacity(gn);
        for _ in 0..gn {
            let kind = r.u8()?;
            let extend = r.u8()?;
            let stop_n = read_u16(&mut r)?;
            if !matches!(kind, 1 | 2) {
                return Err(E::BadTag(u32::from(kind)));
            }
            if extend & !3 != 0 {
                return Err(E::ReservedNotZero);
            }
            if stop_n > MAX_STOPS {
                return Err(E::TooMany);
            }
            let transform = read_f64s::<6>(&mut r)?;
            let coords = if kind == 1 {
                let [x0, y0, x1, y1] = read_f64s::<4>(&mut r)?;
                GradientCoords::Axial { x0, y0, x1, y1 }
            } else {
                let [x0, y0, r0, x1, y1, r1] = read_f64s::<6>(&mut r)?;
                GradientCoords::Radial { x0, y0, r0, x1, y1, r1 }
            };
            if stop_n * 12 > r.left() {
                return Err(E::Truncated);
            }
            let stops = (0..stop_n)
                .map(|_| Ok(Stop { offset: r.f64()?, rgba: r.u32()? }))
                .collect::<Res<Vec<Stop>>>()?;
            let g = Gradient {
                coords,
                extend_before: extend & 1 != 0,
                extend_after: extend & 2 != 0,
                transform,
                stops,
            };
            check_gradient(&g)?;
            gradients.push(g);
        }
        let mut tilings = Vec::with_capacity(tn);
        for _ in 0..tn {
            let paint_type = r.u8()?;
            if !matches!(paint_type, 1 | 2) {
                return Err(E::BadTag(u32::from(paint_type)));
            }
            if r.u8()? != 0 {
                return Err(E::ReservedNotZero);
            }
            let prim_n = read_u16(&mut r)?;
            if prim_n > MAX_TILING_PRIMITIVES {
                return Err(E::TooMany);
            }
            let [x_step, y_step] = read_f64s::<2>(&mut r)?;
            let cell = read_f64s::<4>(&mut r)?;
            let transform = read_f64s::<6>(&mut r)?;
            let mut primitives = Vec::with_capacity(prim_n);
            for _ in 0..prim_n {
                let kind = r.u8()?;
                let rule = r.u8()?;
                let n = read_u16(&mut r)?;
                let rgba = r.u32()?;
                let width = r.f64()?;
                primitives.push(match kind {
                    1 => {
                        let rule = FillRule::from_tag(rule)?;
                        if width.to_bits() != 0 {
                            return Err(E::ReservedNotZero);
                        }
                        TilingPrimitive::Polygon { rule, rgba, points: read_points(&mut r, n)? }
                    }
                    2 => {
                        if rule != 0 {
                            return Err(E::ReservedNotZero);
                        }
                        TilingPrimitive::Polyline { rgba, width, points: read_points(&mut r, n)? }
                    }
                    k => return Err(E::BadTag(u32::from(k))),
                });
            }
            let t = Tiling { coloured: paint_type == 1, x_step, y_step, cell, transform, primitives };
            check_tiling(&t)?;
            tilings.push(t);
        }
        let mut assignments = Vec::with_capacity(an);
        for _ in 0..an {
            let hatch = r.u32()?;
            let rule = FillRule::from_tag(r.u8()?)?;
            let paint = r.u8()?;
            if r.take(2)? != [0, 0] {
                return Err(E::ReservedNotZero);
            }
            let index = r.u32()?;
            let paint = match paint {
                0 if index == 0 => FillSource::Own,
                0 => return Err(E::ReservedNotZero),
                1 => FillSource::Gradient(index),
                2 => FillSource::Tiling(index),
                p => return Err(E::BadTag(u32::from(p))),
            };
            assignments.push(FillAssignment { hatch, rule, paint });
        }
        r.done()?;
        let s = FillPaintStream { gradients, tilings, assignments };
        s.check()?;
        Ok(s)
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}
