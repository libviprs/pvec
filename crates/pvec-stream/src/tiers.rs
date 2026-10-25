//! The Tier 0 complete-coverage stream (issue P5.4, pvec#13).
//!
//! Tier 0 is the first thing a reader paints. Every entity of the artifact is in it, either `Exact`
//! (cheap geometry, which lives in its hot block and is named here by id) or as a `Preview`: a
//! bounded-error stand-in that carries the `EntityId` of the entity it stands for. The error bound is
//! measured by the writer against the exact curve and stored beside the preview. A reader never infers
//! it. Previews are display fidelity only. The design is in `docs/STREAMING.md` chapter 4.

use pvec_entity::EntityId;
use pvec_geometry::{Curve, GeometryError};

/// The most vertices one preview may hold.
pub const MAX_PREVIEW_VERTICES: usize = 4096;

/// Most vertices of a polyline that still counts as cheap exact geometry.
pub const EXACT_POLYLINE_VERTICES: usize = 64;

/// The shape a preview takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreviewForm {
    /// The control polygon of the curve.
    ControlPolygon,
    /// A polyline whose chords stay within the error bound of the curve.
    ChordPolyline,
}

/// How faithful a Tier 0 entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fidelity {
    /// The entity's own geometry.
    Exact,
    /// A bounded-error stand-in, never reported as exact.
    Display,
}

impl Fidelity {
    /// The label conformance output prints.
    pub fn label(self) -> &'static str {
        todo!()
    }
}

/// Why no bounded preview could be made for an entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InfeasibleReason {
    /// The exact evaluator refused the curve, under this rule.
    EvaluatorRefused(&'static str),
    /// No form met the error bound within `MAX_PREVIEW_VERTICES` vertices.
    VertexCap,
}

/// An entity declared infeasible for a bounded preview.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Infeasible {
    /// The entity.
    pub id: EntityId,
    /// Why.
    pub reason: InfeasibleReason,
}

/// Why a Tier 0 plan was refused or could not be read.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Tier0Error {
    /// This entity of the artifact is not in Tier 0.
    Gap(EntityId),
    /// This entity is in Tier 0 twice.
    Duplicate(EntityId),
    /// This id is not an entity of the artifact.
    OutOfRange(EntityId),
    /// A bound or parameter is not usable.
    BadBound,
    /// The stream is malformed.
    Format {
        /// What is wrong.
        reason: &'static str,
    },
    /// A count or length is over a limit.
    Limit,
    /// The exact geometry does not match the preview it is meant to replace.
    Mismatch(EntityId),
    /// The entity is already exact with different geometry.
    Conflict(EntityId),
    /// The scene has no such entity.
    Unknown(EntityId),
    /// The exact evaluator refused.
    Geometry(GeometryError),
}

impl Tier0Error {
    /// The rule identifier.
    pub fn rule(&self) -> &'static str {
        todo!()
    }
}

impl core::fmt::Display for Tier0Error {
    fn fmt(&self, _f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        todo!()
    }
}

impl std::error::Error for Tier0Error {}

/// A bounded-error stand-in for one entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Preview {
    replaces: EntityId,
    form: PreviewForm,
    error_bound: f32,
    origin: [f64; 2],
    vertices: Vec<[f32; 2]>,
}

impl Preview {
    /// A preview from its parts, checked: the bound finite and above zero, between one and
    /// `MAX_PREVIEW_VERTICES` vertices, all finite.
    pub fn new(
        replaces: EntityId,
        form: PreviewForm,
        error_bound: f32,
        origin: [f64; 2],
        vertices: Vec<[f32; 2]>,
    ) -> Result<Preview, Tier0Error> {
        let _ = (replaces, form, error_bound, origin, vertices);
        todo!()
    }
    /// The entity this preview stands for, and the only identity it has.
    pub fn replaces(&self) -> EntityId {
        self.replaces
    }
    /// The form.
    pub fn form(&self) -> PreviewForm {
        self.form
    }
    /// The stored world-space error bound.
    pub fn error_bound(&self) -> f64 {
        f64::from(self.error_bound)
    }
    /// The origin the stored vertices are offsets from.
    pub fn origin(&self) -> [f64; 2] {
        self.origin
    }
    /// The stored vertices, as `f32` offsets from the origin.
    pub fn offsets(&self) -> &[[f32; 2]] {
        &self.vertices
    }
    /// The vertices in world space, after the stored `f32` quantisation.
    pub fn points(&self) -> Vec<[f64; 2]> {
        todo!()
    }
    /// Always `Fidelity::Display`.
    pub fn fidelity(&self) -> Fidelity {
        Fidelity::Display
    }
    /// Always false: a preview is never exact.
    pub fn is_exact(&self) -> bool {
        false
    }
}

/// One entry of Tier 0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tier0Entry<'a> {
    /// Cheap geometry, emitted in full in its hot block.
    Exact {
        /// The entity.
        id: EntityId,
    },
    /// A bounded-error preview.
    Preview(&'a Preview),
}

impl Tier0Entry<'_> {
    /// The entity this entry stands for.
    pub fn id(&self) -> EntityId {
        todo!()
    }
    /// How faithful the entry is.
    pub fn fidelity(&self) -> Fidelity {
        todo!()
    }
}

/// What the writer is asked to hold previews to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tier0Params {
    /// The most a preview may deviate from its curve, in world units.
    pub max_error: f64,
}

/// What a conformance report says about Tier 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Conformance {
    /// Entities in Tier 0 as exact.
    pub exact: usize,
    /// Entities in Tier 0 as previews, labelled display fidelity.
    pub display_fidelity: usize,
    /// Entities declared infeasible.
    pub infeasible: usize,
}

/// The Tier 0 plan of one artifact.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier0Plan {
    entities: u32,
    exact: Vec<EntityId>,
    previews: Vec<Preview>,
    infeasible: Vec<Infeasible>,
}

/// Assembles a plan and checks coverage when it finishes.
#[derive(Debug)]
pub struct Tier0Builder {
    entities: u32,
    params: Tier0Params,
    exact: Vec<EntityId>,
    previews: Vec<Preview>,
    infeasible: Vec<Infeasible>,
}

impl Tier0Builder {
    /// A builder for an artifact of `entities` entities, ids `0..entities`.
    pub fn new(entities: u32, params: Tier0Params) -> Result<Tier0Builder, Tier0Error> {
        let _ = (entities, params);
        todo!()
    }
    /// Records an entity whose cheap geometry goes out exact.
    pub fn add_exact(&mut self, id: EntityId) -> Result<(), Tier0Error> {
        let _ = id;
        todo!()
    }
    /// Records a ready preview (its bound must not exceed the budget).
    pub fn add_preview(&mut self, preview: Preview) -> Result<(), Tier0Error> {
        let _ = preview;
        todo!()
    }
    /// Classifies a curve: cheap geometry is exact, anything else gets a measured preview, or is
    /// declared infeasible.
    pub fn add_curve(&mut self, id: EntityId, curve: &Curve) -> Result<(), Tier0Error> {
        let _ = (id, curve);
        todo!()
    }
    /// Finishes the plan. An entity missing from Tier 0 is a writer error.
    pub fn finish(self) -> Result<Tier0Plan, Tier0Error> {
        todo!()
    }
}

/// Builds the preview of a deferred curve, measured against the exact curve, or says why it cannot.
pub fn build_preview(id: EntityId, curve: &Curve, params: &Tier0Params) -> Result<Result<Preview, InfeasibleReason>, Tier0Error> {
    let _ = (id, curve, params);
    todo!()
}

/// The two-sided maximum deviation between a preview and its exact curve, in world units, taken by
/// sampling the curve through the exact evaluator.
pub fn measure_deviation(curve: &Curve, preview: &Preview) -> Result<f64, Tier0Error> {
    let _ = (curve, preview);
    todo!()
}

/// Whether a curve is cheap enough to go out exact in Tier 0.
pub fn is_cheap_exact(curve: &Curve) -> bool {
    let _ = curve;
    todo!()
}

impl Tier0Plan {
    /// A plan from curves, one per entity.
    pub fn build<I>(entities: u32, curves: I, params: Tier0Params) -> Result<Tier0Plan, Tier0Error>
    where
        I: IntoIterator<Item = (EntityId, Curve)>,
    {
        let _ = (entities, curves, params);
        todo!()
    }
    /// The number of entities in the artifact.
    pub fn artifact_entities(&self) -> usize {
        self.entities as usize
    }
    /// The number of entities reachable from Tier 0.
    pub fn tier0_entities(&self) -> usize {
        todo!()
    }
    /// Checks that the entities reachable from Tier 0 (and the declared infeasible ones) are exactly
    /// the given set.
    pub fn check_coverage(&self, artifact: &[EntityId]) -> Result<(), Tier0Error> {
        let _ = artifact;
        todo!()
    }
    /// The ids that go out exact, ascending.
    pub fn exact_ids(&self) -> &[EntityId] {
        &self.exact
    }
    /// The previews, ascending by the entity they replace.
    pub fn previews(&self) -> &[Preview] {
        &self.previews
    }
    /// The entities declared infeasible, with their reasons.
    pub fn infeasible(&self) -> &[Infeasible] {
        &self.infeasible
    }
    /// The entry for an entity.
    pub fn entry(&self, id: EntityId) -> Option<Tier0Entry<'_>> {
        let _ = id;
        todo!()
    }
    /// What conformance output says about this plan.
    pub fn conformance(&self) -> Conformance {
        todo!()
    }
    /// The Tier 0 stream.
    pub fn encode(&self) -> Vec<u8> {
        todo!()
    }
    /// Reads a Tier 0 stream, checking every length against the bytes that remain.
    pub fn decode(bytes: &[u8]) -> Result<Tier0Plan, Tier0Error> {
        let _ = bytes;
        todo!()
    }
}

/// How a refinement ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refined {
    /// The preview was replaced by the exact geometry.
    Replaced,
    /// Nothing changed: the entity was already exact.
    Unchanged,
}

/// What stands for one entity in a scene.
#[derive(Clone, Debug, PartialEq)]
enum Slot {
    Exact,
    Preview(Preview),
    Refined(Curve),
}

/// The drawable state of an artifact: exactly one thing per entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    slots: Vec<Option<Slot>>,
}

impl Scene {
    /// The scene a reader has after Tier 0.
    pub fn from_plan(plan: &Tier0Plan) -> Scene {
        let _ = plan;
        todo!()
    }
    /// The number of things drawn: one per entity, before and after any refinement.
    pub fn drawn(&self) -> usize {
        todo!()
    }
    /// The fidelity of what stands for the entity.
    pub fn fidelity(&self, id: EntityId) -> Option<Fidelity> {
        let _ = id;
        todo!()
    }
    /// Replaces the preview of `id` by its exact geometry. The curve must agree with the preview
    /// (within its stored bound). Applying the same refinement again changes nothing.
    pub fn refine(&mut self, id: EntityId, exact: &Curve) -> Result<Refined, Tier0Error> {
        let _ = (id, exact);
        todo!()
    }
}
