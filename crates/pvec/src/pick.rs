//! Spatial query and picking over stable entity ids (issue P7.5).
//!
//! A pick answers with the file-local [`EntityId`], the distance and exact curve parameter of the nearest
//! point, and the entity's source-space bounds. None of it depends on how the entity is currently
//! rendered: a Tier 0 preview and the exact geometry that replaces it answer with the same id and the
//! same bounds, and hit testing on exact geometry is analytic (see `hit.rs`), never a tessellation.
//!
//! Two pieces of P7.2 and P7.3 are not in yet, so this is their minimal form. [`PickTolerance`] takes the
//! source-to-screen scale from a [`Transform2`] and nothing else of the screen context; [`EntityIdBatch`]
//! is one flat `u32` array (the 4 bytes per id that ADR 0004 spends), which is the part of the packed
//! layout that needs no per-entity object.

use crate::hit::{nearest, nearest_on_polyline};
use crate::{Bounds2, CapabilityId, Curve, EntityId, Point2, PvecError, PvecReader, Transform2};
use pvec_geometry::Curve2d;

fn refused(rule: &str) -> PvecError {
    PvecError::Query { rule: rule.to_string() }
}

/// A pick radius in screen pixels, tied to the source-to-screen scale it was made under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickTolerance {
    px: f64,
    scale: f64,
}

impl PickTolerance {
    /// A radius of `px` screen pixels under the map `source_to_screen`.
    ///
    /// The scale is the smallest singular value of the map's linear part, which is the screen length of
    /// one source unit along the most compressed direction. For a similarity (pan, zoom, rotation) it is
    /// the zoom. For an anisotropic map it makes the source radius the largest one that can still be
    /// within `px` on screen, so no pick that is inside the radius on screen is lost. A negative or
    /// non-finite radius, or a map that collapses an axis, is refused (`PICK-10`, `PICK-11`).
    pub fn new(px: f64, source_to_screen: &Transform2) -> Result<PickTolerance, PvecError> {
        if !(px.is_finite() && px >= 0.0) {
            return Err(refused("PICK-10"));
        }
        let ex = source_to_screen.apply_vector(pvec_geometry::Vector2::new(1.0, 0.0));
        let ey = source_to_screen.apply_vector(pvec_geometry::Vector2::new(0.0, 1.0));
        let (a, b, c) = (ex.dot(ex), ex.dot(ey), ey.dot(ey));
        let scale = (0.5 * (a + c) - (0.25 * (a - c) * (a - c) + b * b).sqrt()).max(0.0).sqrt();
        if !(scale.is_finite() && scale > 0.0) {
            return Err(refused("PICK-11"));
        }
        Ok(PickTolerance { px, scale })
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
        self.px / self.scale
    }
}

/// How a rectangle selects entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RectMode {
    /// Entities whose source bounds meet the rectangle (touching counts).
    Intersects,
    /// Entities whose source bounds lie inside the rectangle.
    Contains,
}

/// Entity ids packed as one flat `u32` array in ascending order: nothing per entity, so it crosses the
/// WASM boundary as a single slice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityIdBatch {
    ids: Vec<u32>,
}

impl EntityIdBatch {
    /// The ids as a flat array.
    pub fn as_slice(&self) -> &[u32] {
        &self.ids
    }

    /// How many ids.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when there are none.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
}

/// What a query hit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    /// The entity. The same id before and after its preview is replaced by exact geometry.
    pub entity: EntityId,
    /// Distance from the query point to the curve, in source units. For a preview it is the distance to
    /// the preview, which is within the preview's error bound of the true one.
    pub distance: f64,
    /// The curve parameter of the nearest point under the curve's own domain, or `None` while the entity
    /// is held only as a preview.
    pub param: Option<f64>,
    /// The entity's source-space bounds, whatever represents it.
    pub bounds: Bounds2,
    /// The capability the entity's curve needs.
    pub capability: CapabilityId,
}

#[derive(Debug)]
enum Held {
    Exact(Curve),
    Preview { points: Vec<[f64; 2]>, error_bound: f64 },
}

#[derive(Debug)]
struct Entry {
    id: u32,
    bounds: Bounds2,
    /// Length of the bounds' diagonal, the accuracy scale of the hit test.
    size: f64,
    capability: CapabilityId,
    held: Held,
}

/// The most vertices a preview may hand the index, so one call cannot size an allocation.
const MAX_PREVIEW_POINTS: usize = 1 << 20;

/// The pick index of one file. Entries are kept in ascending id order, so results come out ordered and
/// ties go to the lowest id.
#[derive(Debug)]
pub struct PickIndex {
    /// The entities the manifest declares: the only ids that can ever be returned.
    count: u64,
    entries: Vec<Entry>,
}

impl PickIndex {
    /// An empty index for a file the reader has validated. The id range is the file's entity count; no
    /// storage is sized from it.
    pub fn new(reader: &PvecReader) -> Result<PickIndex, PvecError> {
        Ok(PickIndex { count: reader.entity_count()?, entries: Vec::new() })
    }

    fn slot(&self, id: EntityId) -> Result<Result<usize, usize>, PvecError> {
        if u64::from(id.index()) >= self.count {
            return Err(refused("PICK-01"));
        }
        Ok(self.entries.binary_search_by_key(&id.index(), |e| e.id))
    }

    /// Adds an entity held as exact geometry. Its bounds come from the curve. Refused: an id the file
    /// does not declare (`PICK-01`), an id already present (`PICK-02`), a curve with no bounds such as a
    /// bare line (`PICK-03`), a kind with no capability (`PICK-04`).
    pub fn insert_exact(&mut self, id: EntityId, curve: Curve) -> Result<(), PvecError> {
        let at = self.slot(id)?;
        let Err(at) = at else { return Err(refused("PICK-02")) };
        let b = curve.bounds().map_err(|_| refused("PICK-03"))?;
        let capability = pvec_codec::capability_for(curve.kind()).ok_or_else(|| refused("PICK-04"))?;
        let bounds = Bounds2 { min: [b.min().x, b.min().y], max: [b.max().x, b.max().y] };
        self.entries.insert(
            at,
            Entry { id: id.index(), bounds, size: b.diagonal(), capability, held: Held::Exact(curve) },
        );
        Ok(())
    }

    /// Adds an entity held only as a Tier 0 preview: its polyline `points`, the preview's stored error
    /// bound, and the exact bounds and capability the file's entity table gives it. Besides the rules of
    /// [`PickIndex::insert_exact`], refused: bounds or points that are not finite, bounds that are
    /// inverted, no points or too many, a negative error bound (`PICK-05`).
    pub fn insert_preview(
        &mut self,
        id: EntityId,
        capability: CapabilityId,
        bounds: Bounds2,
        points: Vec<[f64; 2]>,
        error_bound: f64,
    ) -> Result<(), PvecError> {
        let at = self.slot(id)?;
        let Err(at) = at else { return Err(refused("PICK-02")) };
        let finite = bounds.min.iter().chain(&bounds.max).all(|v| v.is_finite())
            && points.iter().flatten().all(|v| v.is_finite());
        if !finite
            || bounds.min[0] > bounds.max[0]
            || bounds.min[1] > bounds.max[1]
            || points.is_empty()
            || points.len() > MAX_PREVIEW_POINTS
            || !(error_bound.is_finite() && error_bound >= 0.0)
        {
            return Err(refused("PICK-05"));
        }
        let size = (bounds.max[0] - bounds.min[0]).hypot(bounds.max[1] - bounds.min[1]);
        self.entries.insert(
            at,
            Entry { id: id.index(), bounds, size, capability, held: Held::Preview { points, error_bound } },
        );
        Ok(())
    }

    /// Replaces a preview by its exact curve. The id and the bounds do not change, which is what keeps a
    /// selection pointing at the same entity across a refinement. Refused: an entity that is not in the
    /// index (`PICK-06`), one that is already exact (`PICK-07`), a curve of another capability (`PICK-08`).
    pub fn refine(&mut self, id: EntityId, curve: Curve) -> Result<EntityId, PvecError> {
        let at = self.slot(id)?.map_err(|_| refused("PICK-06"))?;
        let e = &mut self.entries[at];
        if matches!(e.held, Held::Exact(_)) {
            return Err(refused("PICK-07"));
        }
        if pvec_codec::capability_for(curve.kind()) != Some(e.capability) {
            return Err(refused("PICK-08"));
        }
        e.held = Held::Exact(curve);
        Ok(id)
    }

    /// The entity nearest to `p` among those within the tolerance's source radius, or `None`.
    pub fn query_point(&self, p: Point2, tol: PickTolerance) -> Option<Pick> {
        self.within(p, tol.source_radius())
    }

    /// The ids selected by the source-space rectangle `r`. It has no scale, so it returns the same set
    /// at every display scale. An inverted or non-finite rectangle selects nothing.
    pub fn query_rect(&self, r: Bounds2, mode: RectMode) -> EntityIdBatch {
        let ok =
            r.min.iter().chain(&r.max).all(|v| v.is_finite()) && r.min[0] <= r.max[0] && r.min[1] <= r.max[1];
        let ids = if ok {
            self.entries
                .iter()
                .filter(|e| {
                    let b = &e.bounds;
                    match mode {
                        RectMode::Intersects => {
                            b.min[0] <= r.max[0]
                                && b.max[0] >= r.min[0]
                                && b.min[1] <= r.max[1]
                                && b.max[1] >= r.min[1]
                        }
                        RectMode::Contains => {
                            b.min[0] >= r.min[0]
                                && b.max[0] <= r.max[0]
                                && b.min[1] >= r.min[1]
                                && b.max[1] <= r.max[1]
                        }
                    }
                })
                .map(|e| e.id)
                .collect()
        } else {
            Vec::new()
        };
        EntityIdBatch { ids }
    }

    /// The entity nearest to `p` within `max_dist` source units, or `None`. `f64::INFINITY` asks for the
    /// nearest entity wherever it is; a negative or NaN distance finds nothing.
    pub fn query_nearest(&self, p: Point2, max_dist: f64) -> Option<Pick> {
        self.within(p, max_dist)
    }

    fn within(&self, p: Point2, radius: f64) -> Option<Pick> {
        if !(radius >= 0.0 && p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        let mut best: Option<(usize, f64, Option<f64>)> = None;
        for (i, e) in self.entries.iter().enumerate() {
            let slack = match &e.held {
                Held::Exact(_) => 0.0,
                Held::Preview { error_bound, .. } => *error_bound,
            };
            let cutoff = best.map_or(radius + slack, |(_, d, _)| d.min(radius + slack));
            // The box test: cheap, and exact bounds are a superset of the curve.
            let dx = (e.bounds.min[0] - p.x).max(0.0).max(p.x - e.bounds.max[0]);
            let dy = (e.bounds.min[1] - p.y).max(0.0).max(p.y - e.bounds.max[1]);
            if dx.hypot(dy) > cutoff + slack {
                continue;
            }
            let found = match &e.held {
                Held::Exact(c) => nearest(c, p, cutoff, e.size).map(|n| (n.distance, Some(n.param))),
                Held::Preview { points, .. } => nearest_on_polyline(points, p, cutoff).map(|d| (d, None)),
            };
            if let Some((d, param)) = found {
                if best.is_none_or(|(_, bd, _)| d < bd) {
                    best = Some((i, d, param));
                }
            }
        }
        best.map(|(i, distance, param)| {
            let e = &self.entries[i];
            Pick {
                // The id was checked against the manifest count when the entry went in.
                entity: EntityId::new(e.id).expect("entry ids are below the entity limit"),
                distance,
                param,
                bounds: e.bounds,
                capability: e.capability,
            }
        })
    }
}
