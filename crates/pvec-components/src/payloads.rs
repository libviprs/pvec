//! The typed payloads. Every stream refers to entities by [`EntityId`]; none embeds a curve.
//!
//! All integers and floats are little endian and every length is checked against the bytes left before
//! anything is allocated from it. A decoder consumes its payload exactly.

use crate::error::ComponentError as E;
use crate::frame::Stream;
use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;

type Res<T> = Result<T, E>;

/// Most names in a layer table or styles in a line style table.
pub const MAX_TABLE: usize = 65_536;
/// Longest name, in bytes.
pub const MAX_NAME: usize = 255;
/// Most dashes in one line style.
pub const MAX_DASHES: usize = 16;
/// Longest text, in bytes.
pub const MAX_TEXT: usize = 4_096;
/// Most boundary references in one hatch.
pub const MAX_BOUNDARY: usize = 4_096;
/// Longest source handle, in bytes.
pub const MAX_HANDLE: usize = 64;
/// Most records in one stream of records.
pub const MAX_RECORDS: usize = 1 << 20;

struct R<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> R<'a> {
    fn new(b: &'a [u8]) -> Self {
        R { b, p: 0 }
    }
    fn left(&self) -> usize {
        self.b.len() - self.p
    }
    fn take(&mut self, n: usize) -> Res<&'a [u8]> {
        if n > self.left() {
            return Err(E::Truncated);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u32(&mut self) -> Res<u32> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn f64(&mut self) -> Res<f64> {
        let s = self.take(8)?;
        Ok(f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    }
    fn entity(&mut self) -> Res<EntityId> {
        let i = self.u32()?;
        EntityId::new(i).map_err(|_| E::BadEntityId(i))
    }
    fn string(&mut self, max: usize) -> Res<String> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(E::TooLong);
        }
        String::from_utf8(self.take(n)?.to_vec()).map_err(|_| E::BadUtf8)
    }
    fn done(&self) -> Res<()> {
        if self.left() == 0 { Ok(()) } else { Err(E::Trailing) }
    }
    /// A count of records that each take at least `min` bytes: it cannot exceed what is left.
    fn count(&mut self, limit: usize, min: usize) -> Res<usize> {
        let n = self.u32()? as usize;
        if n > limit {
            return Err(E::TooMany);
        }
        if n.checked_mul(min).is_none_or(|need| need > self.left()) {
            return Err(E::Truncated);
        }
        Ok(n)
    }
}

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn f64(&mut self, v: f64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn string(&mut self, s: &str, max: usize) -> Res<()> {
        if s.len() > max {
            return Err(E::TooLong);
        }
        self.u32(s.len() as u32);
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn finite(x: f64) -> Res<f64> {
    if x.is_finite() { Ok(x) } else { Err(E::BadNumber) }
}

fn positive(x: f64) -> Res<f64> {
    if x.is_finite() && x > 0.0 { Ok(x) } else { Err(E::BadNumber) }
}

fn into_stream(registry: &Registry, cap: CapabilityId, payload: Vec<u8>) -> Res<Stream> {
    let version = registry.get(cap).ok_or(E::UnknownCapability(cap))?.version;
    Ok(Stream { capability: cap, version, payload })
}

// ---------------------------------------------------------------- layer

/// Layer names and the layer of each entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerStream {
    /// The layer table.
    pub names: Vec<String>,
    /// Pairs of an entity and an index into `names`.
    pub assignments: Vec<(EntityId, u32)>,
}

impl LayerStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::LAYER;

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.names.len() > MAX_TABLE || self.assignments.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut w = W::default();
        w.u32(self.names.len() as u32);
        w.u32(self.assignments.len() as u32);
        for n in &self.names {
            w.string(n, MAX_NAME)?;
        }
        for (e, i) in &self.assignments {
            if *i as usize >= self.names.len() {
                return Err(E::IndexOutOfRange);
            }
            w.u32(e.index());
            w.u32(*i);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<LayerStream> {
        let mut r = R::new(bytes);
        let names_n = r.count(MAX_TABLE, 4)?;
        let assign_n = r.u32()? as usize;
        if assign_n > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut names = Vec::with_capacity(names_n);
        for _ in 0..names_n {
            names.push(r.string(MAX_NAME)?);
        }
        if assign_n.checked_mul(8).is_none_or(|need| need > r.left()) {
            return Err(E::Truncated);
        }
        let mut assignments = Vec::with_capacity(assign_n);
        for _ in 0..assign_n {
            let e = r.entity()?;
            let i = r.u32()?;
            if i as usize >= names.len() {
                return Err(E::IndexOutOfRange);
            }
            assignments.push((e, i));
        }
        r.done()?;
        Ok(LayerStream { names, assignments })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

// ---------------------------------------------------------------- line style

/// A named dash pattern: alternating dash and gap lengths, empty for a continuous line.
#[derive(Clone, Debug, PartialEq)]
pub struct LineStyleDef {
    /// The style's name.
    pub name: String,
    /// Dash and gap lengths, each finite and not negative, at most 16.
    pub dashes: Vec<f64>,
}

/// Line style definitions and the style of each entity.
#[derive(Clone, Debug, PartialEq)]
pub struct LineStyleStream {
    /// The style table.
    pub styles: Vec<LineStyleDef>,
    /// Pairs of an entity and an index into `styles`.
    pub assignments: Vec<(EntityId, u32)>,
}

impl LineStyleStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::LINE_STYLE;

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.styles.len() > MAX_TABLE || self.assignments.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut w = W::default();
        w.u32(self.styles.len() as u32);
        w.u32(self.assignments.len() as u32);
        for s in &self.styles {
            if s.dashes.len() > MAX_DASHES {
                return Err(E::TooMany);
            }
            w.string(&s.name, MAX_NAME)?;
            w.u32(s.dashes.len() as u32);
            for d in &s.dashes {
                if !d.is_finite() || *d < 0.0 {
                    return Err(E::BadNumber);
                }
                w.f64(*d);
            }
        }
        for (e, i) in &self.assignments {
            if *i as usize >= self.styles.len() {
                return Err(E::IndexOutOfRange);
            }
            w.u32(e.index());
            w.u32(*i);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<LineStyleStream> {
        let mut r = R::new(bytes);
        let n = r.count(MAX_TABLE, 8)?;
        let assign_n = r.u32()? as usize;
        if assign_n > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut styles = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string(MAX_NAME)?;
            let nd = r.u32()? as usize;
            if nd > MAX_DASHES {
                return Err(E::TooMany);
            }
            let mut dashes = Vec::with_capacity(nd);
            for _ in 0..nd {
                let d = r.f64()?;
                if !d.is_finite() || d < 0.0 {
                    return Err(E::BadNumber);
                }
                dashes.push(d);
            }
            styles.push(LineStyleDef { name, dashes });
        }
        if assign_n.checked_mul(8).is_none_or(|need| need > r.left()) {
            return Err(E::Truncated);
        }
        let mut assignments = Vec::with_capacity(assign_n);
        for _ in 0..assign_n {
            let e = r.entity()?;
            let i = r.u32()?;
            if i as usize >= styles.len() {
                return Err(E::IndexOutOfRange);
            }
            assignments.push((e, i));
        }
        r.done()?;
        Ok(LineStyleStream { styles, assignments })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

// ---------------------------------------------------------------- text

/// One piece of text, anchored at a point and tied to an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRecord {
    /// The entity the text belongs to.
    pub entity: EntityId,
    /// Anchor x.
    pub x: f64,
    /// Anchor y.
    pub y: f64,
    /// Text height, strictly positive.
    pub height: f64,
    /// Rotation in radians.
    pub rotation: f64,
    /// The text, at most 4096 bytes.
    pub text: String,
}

/// Text records.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStream {
    /// The records.
    pub records: Vec<TextRecord>,
}

impl TextStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::TEXT;

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.records.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut w = W::default();
        w.u32(self.records.len() as u32);
        for t in &self.records {
            w.u32(t.entity.index());
            w.f64(finite(t.x)?);
            w.f64(finite(t.y)?);
            w.f64(positive(t.height)?);
            w.f64(finite(t.rotation)?);
            w.string(&t.text, MAX_TEXT)?;
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<TextStream> {
        let mut r = R::new(bytes);
        let n = r.count(MAX_RECORDS, 40)?;
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            let entity = r.entity()?;
            let (x, y) = (finite(r.f64()?)?, finite(r.f64()?)?);
            let height = positive(r.f64()?)?;
            let rotation = finite(r.f64()?)?;
            let text = r.string(MAX_TEXT)?;
            records.push(TextRecord { entity, x, y, height, rotation, text });
        }
        r.done()?;
        Ok(TextStream { records })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

// ---------------------------------------------------------------- hatch

/// How a hatch is filled.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// A solid colour, as RGBA.
    Solid {
        /// Red, green, blue and alpha, one byte each.
        rgba: u32,
    },
    /// A named pattern.
    Pattern {
        /// The pattern name.
        name: String,
        /// Pattern angle in radians.
        angle: f64,
        /// Pattern scale, strictly positive.
        scale: f64,
        /// Line colour, as RGBA.
        rgba: u32,
    },
}

/// A hatch: its boundary is entities that already exist, plus its own fill parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct HatchRecord {
    /// The boundary, as references to geometry entities. At least one, at most 4096.
    pub boundary: Vec<EntityId>,
    /// The fill.
    pub fill: Fill,
}

/// Hatch records. A hatch introduces no curve type into the geometry oracle: it points at curves.
#[derive(Clone, Debug, PartialEq)]
pub struct HatchStream {
    /// The records.
    pub records: Vec<HatchRecord>,
}

/// The boundary-kind word of a hatch record: references to entities.
const BOUNDARY_REFERENCES: u32 = 1;
/// The word an inline curve would use. It is not a valid hatch and is refused as embedded geometry.
const BOUNDARY_INLINE_CURVE: u32 = 2;

impl HatchStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::HATCH;

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.records.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut w = W::default();
        w.u32(self.records.len() as u32);
        for h in &self.records {
            if h.boundary.is_empty() {
                return Err(E::BadCount);
            }
            if h.boundary.len() > MAX_BOUNDARY {
                return Err(E::TooMany);
            }
            w.u32(BOUNDARY_REFERENCES);
            w.u32(h.boundary.len() as u32);
            for e in &h.boundary {
                w.u32(e.index());
            }
            match &h.fill {
                Fill::Solid { rgba } => {
                    w.u32(1);
                    w.u32(*rgba);
                }
                Fill::Pattern { name, angle, scale, rgba } => {
                    w.u32(2);
                    w.string(name, MAX_NAME)?;
                    w.f64(finite(*angle)?);
                    w.f64(positive(*scale)?);
                    w.u32(*rgba);
                }
            }
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<HatchStream> {
        let mut r = R::new(bytes);
        let n = r.count(MAX_RECORDS, 20)?;
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            match r.u32()? {
                BOUNDARY_REFERENCES => {}
                BOUNDARY_INLINE_CURVE => return Err(E::EmbeddedGeometry),
                other => return Err(E::BadTag(other)),
            }
            let nb = r.u32()? as usize;
            if nb == 0 {
                return Err(E::BadCount);
            }
            if nb > MAX_BOUNDARY {
                return Err(E::TooMany);
            }
            if nb.checked_mul(4).is_none_or(|need| need > r.left()) {
                return Err(E::Truncated);
            }
            let mut boundary = Vec::with_capacity(nb);
            for _ in 0..nb {
                boundary.push(r.entity()?);
            }
            let fill = match r.u32()? {
                1 => Fill::Solid { rgba: r.u32()? },
                2 => {
                    let name = r.string(MAX_NAME)?;
                    let angle = finite(r.f64()?)?;
                    let scale = positive(r.f64()?)?;
                    Fill::Pattern { name, angle, scale, rgba: r.u32()? }
                }
                other => return Err(E::BadTag(other)),
            };
            records.push(HatchRecord { boundary, fill });
        }
        r.done()?;
        Ok(HatchStream { records })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

// ---------------------------------------------------------------- provenance

/// Where an entity came from in the source file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceRecord {
    /// The entity.
    pub entity: EntityId,
    /// What kind of source identity the handle is, as the producer defines it.
    pub source_kind: u16,
    /// The source handle, at most 64 bytes. Its content is the producer's business.
    pub handle: Vec<u8>,
}

/// Source provenance. This is the one place a source handle lives; it is never inside an `EntityId`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceStream {
    /// The records.
    pub records: Vec<ProvenanceRecord>,
}

impl ProvenanceStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::SOURCE_PROVENANCE;

    /// The payload bytes.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.records.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut w = W::default();
        w.u32(self.records.len() as u32);
        for p in &self.records {
            if p.handle.len() > MAX_HANDLE {
                return Err(E::TooLong);
            }
            w.u32(p.entity.index());
            w.u32(u32::from(p.source_kind));
            w.u32(p.handle.len() as u32);
            w.0.extend_from_slice(&p.handle);
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly.
    pub fn decode(bytes: &[u8]) -> Res<ProvenanceStream> {
        let mut r = R::new(bytes);
        let n = r.count(MAX_RECORDS, 12)?;
        let mut records = Vec::with_capacity(n);
        for _ in 0..n {
            let entity = r.entity()?;
            let kind = r.u32()?;
            let source_kind = u16::try_from(kind).map_err(|_| E::ReservedNotZero)?;
            let len = r.u32()? as usize;
            if len > MAX_HANDLE {
                return Err(E::TooLong);
            }
            let handle = r.take(len)?.to_vec();
            records.push(ProvenanceRecord { entity, source_kind, handle });
        }
        r.done()?;
        Ok(ProvenanceStream { records })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}

// ---------------------------------------------------------------- reserved

/// The reserved space for future DWG semantics. Its payload is opaque: this crate never interprets it,
/// and a reader that does not know it skips it like any other stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservedStream {
    /// The opaque payload.
    pub payload: Vec<u8>,
}

impl ReservedStream {
    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::DWG_SEMANTICS;

    /// The payload bytes, unchanged.
    pub fn encode(&self) -> Res<Vec<u8>> {
        Ok(self.payload.clone())
    }

    /// The payload bytes, unchanged.
    pub fn decode(bytes: &[u8]) -> Res<ReservedStream> {
        Ok(ReservedStream { payload: bytes.to_vec() })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }
}
