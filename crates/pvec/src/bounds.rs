use pvec_entity::EntityId;

/// Axis-aligned bounds in source space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds2 {
    /// `[min_x, min_y]`.
    pub min: [f64; 2],
    /// `[max_x, max_y]`.
    pub max: [f64; 2],
}

/// A curve named by the entity that holds it, which is how a composite refers to its segments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurveRef {
    /// The entity holding the referenced curve.
    pub entity: EntityId,
}
