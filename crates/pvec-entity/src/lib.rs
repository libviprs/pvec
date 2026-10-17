//! The file-local stable entity identity (issue P3.6).
//!
//! An [`EntityId`] is a fixed-width index into one artifact: dense from zero, assigned in the writer's
//! canonical order, and the same for a preview and for the exact geometry that replaces it. It is
//! deliberately only a number. Source handles, original DWG identity and provenance live in the
//! provenance component stream and are never encoded inside an id.
//!
//! The stability rule is in `docs/ENTITY_ID.md`: ids are stable within one artifact and are not
//! promised across regenerations from a changed source.
//!
//! [`EntityTable`] is the in-memory shape of the rule. It holds each entity's kind and bounds in
//! arrays of their own, so an id resolves to its bounds without decoding any geometry payload, which
//! is what picking (P7.5) and viewport queries (P5.5) rely on.

use core::fmt;
use pvec_geometry::{Bounds, CurveKind};
use pvec_numeric::MAX_ENTITIES;

/// A compact file-local entity index.
///
/// Four bytes, no niche, no string, handle or pointer: the size assertion below fails the build if
/// anything else is ever put in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct EntityId(u32);

const _: () = assert!(core::mem::size_of::<EntityId>() == 4);
const _: () = assert!(core::mem::align_of::<EntityId>() == 4);
const _: () = assert!(MAX_ENTITIES <= u32::MAX as usize);

impl EntityId {
    /// An id from its index, which must be below the entity limit of NP-18.
    pub fn new(index: u32) -> Result<EntityId, EntityError> {
        if (index as usize) < MAX_ENTITIES { Ok(EntityId(index)) } else { Err(EntityError::TooMany) }
    }

    /// The index.
    pub fn index(self) -> u32 {
        self.0
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Why an id or a table operation was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum EntityError {
    /// More entities than NP-18 allows in one file.
    TooMany,
    /// The ids are not dense: this id is missing.
    Gap(EntityId),
    /// The same id appears twice.
    Duplicate(EntityId),
    /// The table has no such entity.
    Unknown(EntityId),
    /// The entity is already exact, so there is nothing to refine.
    AlreadyExact(EntityId),
}

impl EntityError {
    /// The rule identifier: NP-18 for the count, `ENTITY-nn` for the identity rules of this crate.
    pub fn rule(&self) -> &'static str {
        match self {
            EntityError::TooMany => "NP-18",
            EntityError::Gap(_) | EntityError::Duplicate(_) => "ENTITY-01",
            EntityError::Unknown(_) => "ENTITY-02",
            EntityError::AlreadyExact(_) => "ENTITY-03",
        }
    }
}

impl fmt::Display for EntityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntityError::TooMany => write!(f, "more entities than the file limit ({})", self.rule()),
            EntityError::Gap(id) => write!(f, "entity ids are not dense: {id} is missing ({})", self.rule()),
            EntityError::Duplicate(id) => write!(f, "entity id {id} appears twice ({})", self.rule()),
            EntityError::Unknown(id) => write!(f, "no entity {id} ({})", self.rule()),
            EntityError::AlreadyExact(id) => write!(f, "entity {id} is already exact ({})", self.rule()),
        }
    }
}

impl std::error::Error for EntityError {}

/// What an entity currently stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Representation {
    /// A bounded-error stand-in (display fidelity), to be replaced by the exact geometry.
    Preview,
    /// The exact geometry.
    Exact,
}

/// The dense table of one artifact: ids are the positions, assigned in push order.
///
/// Pushing in the writer's canonical order is what makes the ids reproducible: the same input in the
/// same order gives the same ids, and reordering the input changes them.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EntityTable {
    kinds: Vec<CurveKind>,
    bounds: Vec<Bounds>,
    reps: Vec<Representation>,
}

/// The most a size hint reserves up front, so a hostile count cannot size an allocation.
const RESERVE_CAP: usize = 1 << 16;

impl EntityTable {
    /// An empty table.
    pub fn new() -> EntityTable {
        EntityTable::default()
    }

    /// An empty table for about `expected` entities. The count is checked against NP-18 first, and
    /// the up-front reservation is capped, because the number may come from an untrusted header.
    pub fn with_expected(expected: usize) -> Result<EntityTable, EntityError> {
        if expected > MAX_ENTITIES {
            return Err(EntityError::TooMany);
        }
        let n = expected.min(RESERVE_CAP);
        Ok(EntityTable {
            kinds: Vec::with_capacity(n),
            bounds: Vec::with_capacity(n),
            reps: Vec::with_capacity(n),
        })
    }

    /// Adds the next entity and returns its id, which is the previous length.
    pub fn push(
        &mut self,
        kind: CurveKind,
        bounds: Bounds,
        representation: Representation,
    ) -> Result<EntityId, EntityError> {
        let id = EntityId::new(u32::try_from(self.kinds.len()).map_err(|_| EntityError::TooMany)?)?;
        self.kinds.push(kind);
        self.bounds.push(bounds);
        self.reps.push(representation);
        Ok(id)
    }

    /// Number of entities.
    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    /// True when the table is empty.
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    /// How many more entities fit under the NP-18 limit.
    pub fn capacity_left(&self) -> usize {
        MAX_ENTITIES - self.kinds.len()
    }

    /// The id at a position, which is the position itself.
    pub fn id_at(&self, position: usize) -> Option<EntityId> {
        if position < self.kinds.len() { u32::try_from(position).ok().map(EntityId) } else { None }
    }

    fn slot(&self, id: EntityId) -> Result<usize, EntityError> {
        let i = id.index() as usize;
        if i < self.kinds.len() { Ok(i) } else { Err(EntityError::Unknown(id)) }
    }

    /// The entity's bounds, without touching any geometry payload.
    pub fn bounds(&self, id: EntityId) -> Result<Bounds, EntityError> {
        self.slot(id).map(|i| self.bounds[i])
    }

    /// The entity's curve kind.
    pub fn kind(&self, id: EntityId) -> Result<CurveKind, EntityError> {
        self.slot(id).map(|i| self.kinds[i])
    }

    /// Whether the entity is a preview or exact.
    pub fn representation(&self, id: EntityId) -> Result<Representation, EntityError> {
        self.slot(id).map(|i| self.reps[i])
    }

    /// Replaces a preview by its exact geometry. The identity does not change: the same id comes back,
    /// the kind stays, only the bounds (now the exact ones) and the representation are updated. An
    /// entity that is already exact is left untouched and reported.
    pub fn refine(&mut self, id: EntityId, exact_bounds: Bounds) -> Result<EntityId, EntityError> {
        let i = self.slot(id)?;
        if self.reps[i] == Representation::Exact {
            return Err(EntityError::AlreadyExact(id));
        }
        self.bounds[i] = exact_bounds;
        self.reps[i] = Representation::Exact;
        Ok(id)
    }
}

/// Checks that `ids` are exactly `0..n` for `n = ids.len()`, in any order, and returns `n`.
///
/// The bitmap is sized by the length of the list, never by the largest id, so a hostile id near the
/// limit cannot size an allocation. A list that is not dense reports the smallest missing id; a
/// repeated id is reported as such.
pub fn check_dense(ids: &[EntityId]) -> Result<usize, EntityError> {
    let n = ids.len();
    if n > MAX_ENTITIES {
        return Err(EntityError::TooMany);
    }
    let mut seen = vec![false; n];
    for id in ids {
        let i = id.index() as usize;
        if i < n {
            if seen[i] {
                return Err(EntityError::Duplicate(*id));
            }
            seen[i] = true;
        }
    }
    match seen.iter().position(|s| !s) {
        // `n <= MAX_ENTITIES`, so the position is a valid id.
        Some(missing) => Err(EntityError::Gap(EntityId(missing as u32))),
        None => Ok(n),
    }
}
