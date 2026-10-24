//! The semantic class stream (issue P3.9): what a drawn thing is in the building.
//!
//! Two kinds of subject share one stream. An **element class** (`Wall`, `Column`, or `Other` so that
//! absence is explicit) sits on a geometry entity that already exists. A **region** (`Room`, `Corridor`)
//! is not drawn linework, so it is a record that points at an ordered closed boundary by `EntityId`: an
//! outer ring and optional holes. It embeds no curve parameters, the rule the hatch stream already has.
//! Every entry says where it came from ([`SourceTag`]) and an inferred one may carry a confidence.
//!
//! Like the other streams this one is optional, framed by the registry id and version, and skipped by a
//! reader that does not know it. The crate cannot reach a curve, so closure and existence are checked
//! through [`EntityGeometry`], which the caller implements over whatever geometry it has decoded. The
//! byte layout is in `docs/COMPONENT_STREAMS.md`.

use crate::error::ComponentError as E;
use crate::frame::{Opened, Stream};
use crate::payloads::{MAX_BOUNDARY, MAX_RECORDS, R, W, finite, into_stream};
use pvec_capabilities::{CapabilityId, Registry, ids};
use pvec_entity::EntityId;
use std::collections::HashSet;
use std::fmt;

type Res<T> = Result<T, E>;

/// Most holes in one region.
pub const MAX_HOLES: usize = 256;

/// The ring-kind word of a region: a ring made of references to entities.
const RING_REFERENCES: u32 = 1;
/// The word an inline curve would use. It is refused as embedded geometry.
const RING_INLINE_CURVE: u32 = 2;

/// Bytes of one element entry.
const ELEMENT_BYTES: usize = 24;
/// Fewest bytes of one region record (an outer ring with no entity, no holes).
const REGION_MIN_BYTES: usize = 32;

/// What a drawn element is. Closed for this stream version: a new value bumps the stream version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElementClass {
    /// Not classified. Present so that absence is stated and not implied.
    Other,
    /// A wall.
    Wall,
    /// A column.
    Column,
}

impl ElementClass {
    fn word(self) -> u32 {
        match self {
            ElementClass::Other => 0,
            ElementClass::Wall => 1,
            ElementClass::Column => 2,
        }
    }
    fn from_word(w: u32) -> Res<ElementClass> {
        match w {
            0 => Ok(ElementClass::Other),
            1 => Ok(ElementClass::Wall),
            2 => Ok(ElementClass::Column),
            other => Err(E::BadTag(other)),
        }
    }
}

/// What a region is. Closed for this stream version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// A room.
    Room,
    /// A corridor.
    Corridor,
}

impl RegionKind {
    fn word(self) -> u32 {
        match self {
            RegionKind::Room => 1,
            RegionKind::Corridor => 2,
        }
    }
    fn from_word(w: u32) -> Res<RegionKind> {
        match w {
            1 => Ok(RegionKind::Room),
            2 => Ok(RegionKind::Corridor),
            other => Err(E::BadTag(other)),
        }
    }
}

/// How an entry came to be, so a reader can tell a drafted wall from a guessed room.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTag {
    /// A rule over layer names.
    LayerRule,
    /// A rule over block names.
    BlockRule,
    /// Inferred from the geometry.
    Inferred,
    /// Stated explicitly by the source or the author.
    Explicit,
}

impl SourceTag {
    fn word(self) -> u32 {
        match self {
            SourceTag::LayerRule => 1,
            SourceTag::BlockRule => 2,
            SourceTag::Inferred => 3,
            SourceTag::Explicit => 4,
        }
    }
    fn from_word(w: u32) -> Res<SourceTag> {
        match w {
            1 => Ok(SourceTag::LayerRule),
            2 => Ok(SourceTag::BlockRule),
            3 => Ok(SourceTag::Inferred),
            4 => Ok(SourceTag::Explicit),
            other => Err(E::BadTag(other)),
        }
    }
}

/// The source tag of an entry and, for an inferred one only, an optional confidence from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    /// Where the entry came from.
    pub source: SourceTag,
    /// How sure the inference is, finite and in 0 to 1. Only an `Inferred` entry may carry one.
    pub confidence: Option<f64>,
}

impl Origin {
    fn check(&self) -> Res<()> {
        match self.confidence {
            None => Ok(()),
            Some(_) if self.source != SourceTag::Inferred => Err(E::ConfidenceNotAllowed),
            Some(c) if c.is_finite() && (0.0..=1.0).contains(&c) => Ok(()),
            Some(_) => Err(E::BadNumber),
        }
    }

    fn write(&self, w: &mut W) -> Res<()> {
        self.check()?;
        w.u32(self.source.word());
        match self.confidence {
            Some(c) => {
                w.u32(1);
                w.f64(finite(c)?);
            }
            None => {
                w.u32(0);
                w.f64(0.0);
            }
        }
        Ok(())
    }

    fn read(r: &mut R<'_>) -> Res<Origin> {
        let source = SourceTag::from_word(r.u32()?)?;
        let present = r.u32()?;
        let bits = r.take(8)?;
        let value =
            f64::from_le_bytes([bits[0], bits[1], bits[2], bits[3], bits[4], bits[5], bits[6], bits[7]]);
        let confidence = match present {
            0 if bits.iter().all(|b| *b == 0) => None,
            0 => return Err(E::ReservedNotZero),
            1 => Some(value),
            other => return Err(E::BadTag(other)),
        };
        let o = Origin { source, confidence };
        o.check()?;
        Ok(o)
    }
}

/// A class on an entity that already exists in the geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementEntry {
    /// The entity. An entity is classified at most once.
    pub entity: EntityId,
    /// Its class.
    pub class: ElementClass,
    /// Where the class came from.
    pub origin: Origin,
}

/// A room or a corridor: a region bounded by references to entities, never by curve parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionRecord {
    /// Room or corridor.
    pub kind: RegionKind,
    /// Where the region came from.
    pub origin: Origin,
    /// The ordered outer ring, one to 4096 entities. Consecutive entities share an endpoint (in either
    /// direction) and the last returns to the first: [`SemanticClassStream::validate`] checks it.
    pub outer: Vec<EntityId>,
    /// Holes, at most 256, each a ring of the same kind.
    pub holes: Vec<Vec<EntityId>>,
}

/// What this crate may ask of the geometry a stream points at. The crate depends on no geometry, so the
/// caller answers from whatever it has decoded.
pub trait EntityGeometry {
    /// The start and end point of the entity (equal for a closed curve), or `None` when no such entity
    /// exists.
    fn endpoints(&self, entity: EntityId) -> Option<([f64; 2], [f64; 2])>;
}

/// Why a stream failed validation against the geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RegionError {
    /// A reference to an entity that does not exist. `region` is the index of the region that holds the
    /// reference, or `None` when it is an element entry.
    MissingEntity {
        /// The entity that is not there.
        entity: EntityId,
        /// Index of the region in the stream, `None` for an element entry.
        region: Option<usize>,
    },
    /// A ring of this region does not close.
    NotClosed {
        /// Index of the region in the stream.
        region: usize,
    },
    /// An entity appears twice in the rings of one region, within a ring or across the outer ring and
    /// the holes.
    DuplicateEntity {
        /// Index of the region in the stream.
        region: usize,
        /// The entity that appears twice.
        entity: EntityId,
    },
    /// The closure tolerance is NaN, infinite or negative.
    BadTolerance,
}

impl RegionError {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        match self {
            RegionError::MissingEntity { .. } => "COMP-16",
            RegionError::NotClosed { .. } => "COMP-17",
            RegionError::DuplicateEntity { .. } => "COMP-18",
            RegionError::BadTolerance => "COMP-19",
        }
    }
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = self.rule();
        match self {
            RegionError::MissingEntity { entity, region: Some(region) } => {
                write!(f, "entity {} in region {region} does not exist ({r})", entity.index())
            }
            RegionError::MissingEntity { entity, region: None } => {
                write!(f, "entity {} does not exist ({r})", entity.index())
            }
            RegionError::DuplicateEntity { region, entity } => {
                write!(f, "entity {} appears twice in region {region} ({r})", entity.index())
            }
            RegionError::BadTolerance => {
                write!(f, "the closure tolerance is not finite and non-negative ({r})")
            }
            RegionError::NotClosed { region } => {
                write!(f, "the boundary of region {region} is not closed ({r})")
            }
        }
    }
}

impl std::error::Error for RegionError {}

/// What a reader gets from the semantic class stream of an opened set. A damaged payload fails this
/// stream only: it is reported here, and nothing about it reaches the geometry or the other streams.
#[derive(Clone, Debug, PartialEq)]
pub enum SemanticRead {
    /// The set holds no semantic class stream this reader opened (absent, unknown, or skipped).
    Absent,
    /// The stream decoded.
    Present(SemanticClassStream),
    /// The stream was opened at a compatible version but its payload is damaged, and is dropped.
    Damaged(E),
}

/// The semantic class stream.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticClassStream {
    /// Classes on entities.
    pub elements: Vec<ElementEntry>,
    /// Rooms and corridors.
    pub regions: Vec<RegionRecord>,
}

fn write_ring(w: &mut W, ring: &[EntityId], limit: usize) -> Res<()> {
    if ring.is_empty() {
        return Err(E::BadCount);
    }
    if ring.len() > limit {
        return Err(E::TooMany);
    }
    w.u32(RING_REFERENCES);
    w.u32(ring.len() as u32);
    for e in ring {
        w.u32(e.index());
    }
    Ok(())
}

fn read_ring(r: &mut R<'_>) -> Res<Vec<EntityId>> {
    match r.u32()? {
        RING_REFERENCES => {}
        RING_INLINE_CURVE => return Err(E::EmbeddedGeometry),
        other => return Err(E::BadTag(other)),
    }
    let n = r.u32()? as usize;
    if n == 0 {
        return Err(E::BadCount);
    }
    if n > MAX_BOUNDARY {
        return Err(E::TooMany);
    }
    if n.checked_mul(4).is_none_or(|need| need > r.left()) {
        return Err(E::Truncated);
    }
    (0..n).map(|_| r.entity()).collect()
}

fn near(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
    (a[0] - b[0]).abs() <= tol && (a[1] - b[1]).abs() <= tol
}

/// Whether the entities, in order and each in either direction, form one closed loop.
fn ring_closes(ends: &[([f64; 2], [f64; 2])], tol: f64) -> bool {
    let start_with = |first_reversed: bool| {
        let (a, b) = ends[0];
        let (start, mut cur) = if first_reversed { (b, a) } else { (a, b) };
        for &(p, q) in &ends[1..] {
            if near(cur, p, tol) {
                cur = q;
            } else if near(cur, q, tol) {
                cur = p;
            } else {
                return false;
            }
        }
        near(cur, start, tol)
    };
    start_with(false) || start_with(true)
}

impl SemanticClassStream {
    /// Decodes the semantic class stream of an opened set, if it holds one.
    pub fn from_opened(_opened: &Opened) -> SemanticRead {
        SemanticRead::Absent
    }

    /// The registry capability.
    pub const CAPABILITY: CapabilityId = ids::SEMANTIC_CLASS;

    /// The payload bytes. Refuses what a reader would refuse: an unmatched confidence, a duplicate entity,
    /// an empty or oversized ring.
    pub fn encode(&self) -> Res<Vec<u8>> {
        if self.elements.len() > MAX_RECORDS || self.regions.len() > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let mut seen = HashSet::new();
        let mut w = W::default();
        w.u32(self.elements.len() as u32);
        w.u32(self.regions.len() as u32);
        for e in &self.elements {
            if !seen.insert(e.entity.index()) {
                return Err(E::Duplicate(e.entity));
            }
            w.u32(e.entity.index());
            w.u32(e.class.word());
            e.origin.write(&mut w)?;
        }
        for g in &self.regions {
            if g.holes.len() > MAX_HOLES {
                return Err(E::TooMany);
            }
            w.u32(g.kind.word());
            g.origin.write(&mut w)?;
            write_ring(&mut w, &g.outer, MAX_BOUNDARY)?;
            w.u32(g.holes.len() as u32);
            for h in &g.holes {
                write_ring(&mut w, h, MAX_BOUNDARY)?;
            }
        }
        Ok(w.0)
    }

    /// Reads a payload, which must be consumed exactly. A region that carries curve parameters, an
    /// undefined tag and a value outside the closed enums are refused.
    pub fn decode(bytes: &[u8]) -> Res<SemanticClassStream> {
        let mut r = R::new(bytes);
        let elements_n = r.u32()? as usize;
        let regions_n = r.u32()? as usize;
        if elements_n > MAX_RECORDS || regions_n > MAX_RECORDS {
            return Err(E::TooMany);
        }
        let need = elements_n * ELEMENT_BYTES + regions_n * REGION_MIN_BYTES; // both at most 2^20 * 32
        if need > r.left() {
            return Err(E::Truncated);
        }
        let mut seen = HashSet::new();
        let mut elements = Vec::with_capacity(elements_n);
        for _ in 0..elements_n {
            let entity = r.entity()?;
            let class = ElementClass::from_word(r.u32()?)?;
            let origin = Origin::read(&mut r)?;
            if !seen.insert(entity.index()) {
                return Err(E::Duplicate(entity));
            }
            elements.push(ElementEntry { entity, class, origin });
        }
        let mut regions = Vec::with_capacity(regions_n);
        for _ in 0..regions_n {
            let kind = RegionKind::from_word(r.u32()?)?;
            let origin = Origin::read(&mut r)?;
            let outer = read_ring(&mut r)?;
            let holes_n = r.u32()? as usize;
            if holes_n > MAX_HOLES {
                return Err(E::TooMany);
            }
            if holes_n * 8 > r.left() {
                return Err(E::Truncated);
            }
            let holes = (0..holes_n).map(|_| read_ring(&mut r)).collect::<Res<Vec<_>>>()?;
            regions.push(RegionRecord { kind, origin, outer, holes });
        }
        r.done()?;
        Ok(SemanticClassStream { elements, regions })
    }

    /// The stream, with the version the registry gives this capability.
    pub fn into_stream(&self, registry: &Registry) -> Res<Stream> {
        into_stream(registry, Self::CAPABILITY, self.encode()?)
    }

    /// The entities with this class, in stream order.
    pub fn entities_of(&self, class: ElementClass) -> Vec<EntityId> {
        self.elements.iter().filter(|e| e.class == class).map(|e| e.entity).collect()
    }

    /// The regions of this kind, in stream order.
    pub fn regions_of(&self, kind: RegionKind) -> Vec<&RegionRecord> {
        self.regions.iter().filter(|g| g.kind == kind).collect()
    }

    /// Checks the stream against the geometry it points at: every referenced entity exists, and every ring
    /// (outer and holes) is a closed loop, with endpoints that meet within `tolerance`.
    pub fn validate(&self, geometry: &dyn EntityGeometry, tolerance: f64) -> Result<(), RegionError> {
        for e in &self.elements {
            geometry
                .endpoints(e.entity)
                .ok_or(RegionError::MissingEntity { entity: e.entity, region: None })?;
        }
        for (region, g) in self.regions.iter().enumerate() {
            for ring in std::iter::once(&g.outer).chain(g.holes.iter()) {
                let ends = ring
                    .iter()
                    .map(|e| {
                        geometry.endpoints(*e).ok_or(RegionError::MissingEntity { entity: *e, region: None })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if ends.is_empty() || !ring_closes(&ends, tolerance) {
                    return Err(RegionError::NotClosed { region });
                }
            }
        }
        Ok(())
    }
}
