//! Spatial query and picking over stable entity ids (issue P7.5). Skeleton: the bodies arrive with the fix.

use crate::{Bounds2, CapabilityId, Curve, EntityId, Point2, PvecError, PvecReader, Transform2};

/// A pick radius expressed in screen pixels and tied to the source-to-screen scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickTolerance {
    px: f64,
    scale: f64,
}

impl PickTolerance {
    /// A radius of `px` screen pixels under `source_to_screen`.
    pub fn new(px: f64, source_to_screen: &Transform2) -> Result<PickTolerance, PvecError> {
        let _ = (px, source_to_screen);
        Ok(PickTolerance { px: 0.0, scale: 1.0 })
    }
    /// The radius in screen pixels.
    pub fn px(&self) -> f64 {
        self.px
    }
    /// Screen pixels per source unit.
    pub fn scale(&self) -> f64 {
        self.scale
    }
    /// The radius in source units.
    pub fn source_radius(&self) -> f64 {
        0.0
    }
}

/// How a rectangle selects entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectMode {
    /// Entities whose bounds meet the rectangle.
    Intersects,
    /// Entities whose bounds lie inside the rectangle.
    Contains,
}

/// Entity ids packed as one flat `u32` array, ascending.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityIdBatch {
    ids: Vec<u32>,
}

impl EntityIdBatch {
    /// The flat id array.
    pub fn as_slice(&self) -> &[u32] {
        &self.ids
    }
    /// Number of ids.
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    /// True when there are no ids.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// What a query hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    /// The entity.
    pub entity: EntityId,
    /// Distance from the query point to the curve, in source units.
    pub distance: f64,
    /// The curve parameter of the nearest point, `None` while only a preview is held.
    pub param: Option<f64>,
    /// The entity's source-space bounds.
    pub bounds: Bounds2,
    /// The capability the entity's curve needs.
    pub capability: CapabilityId,
}

/// The pick index of one file.
#[derive(Debug)]
pub struct PickIndex {
    count: u64,
}

impl PickIndex {
    /// An empty index for the entities of a validated file.
    pub fn new(reader: &PvecReader) -> Result<PickIndex, PvecError> {
        Ok(PickIndex { count: reader.entity_count()? })
    }
    /// Adds an entity held as exact geometry.
    pub fn insert_exact(&mut self, id: EntityId, curve: Curve) -> Result<(), PvecError> {
        let _ = (id, curve, self.count);
        Ok(())
    }
    /// Adds an entity held only as a preview.
    pub fn insert_preview(
        &mut self,
        id: EntityId,
        capability: CapabilityId,
        bounds: Bounds2,
        points: Vec<[f64; 2]>,
        error_bound: f64,
    ) -> Result<(), PvecError> {
        let _ = (id, capability, bounds, points, error_bound);
        Ok(())
    }
    /// Replaces a preview by its exact curve; the id and bounds stay.
    pub fn refine(&mut self, id: EntityId, curve: Curve) -> Result<EntityId, PvecError> {
        let _ = curve;
        Ok(id)
    }
    /// The nearest entity within the tolerance.
    pub fn query_point(&self, p: Point2, tol: PickTolerance) -> Option<Pick> {
        let _ = (p, tol);
        None
    }
    /// The ids selected by a source-space rectangle.
    pub fn query_rect(&self, r: Bounds2, mode: RectMode) -> EntityIdBatch {
        let _ = (r, mode);
        EntityIdBatch::default()
    }
    /// The nearest entity within `max_dist` source units.
    pub fn query_nearest(&self, p: Point2, max_dist: f64) -> Option<Pick> {
        let _ = (p, max_dist);
        None
    }
}
