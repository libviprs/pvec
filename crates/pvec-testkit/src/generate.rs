//! The canonical artifact generator (issue P2.4).
//!
//! It turns the authored mathematics of a known-truth fixture (`expected.math.json`) into a `.pvec`
//! artifact, through the production deterministic writer and nothing else: no CAD reader, no
//! `libviprs`, no second encoder. The path is
//!
//! ```text
//! ExpectedMath --plan--> Plan (validated through the semantic model) --write--> bytes
//! ```
//!
//! # The mapping
//!
//! The records are three dimensional and the PVec profile is two dimensional with no implicit
//! promotion (`docs/ISO_PROFILE.md`), so the generator holds a record to the plane exactly: every `z`
//! is zero and every normal is `+z`. Beyond that it maps class by class, and stores the numbers as
//! authored, bit for bit:
//!
//! | record class | stored as |
//! |---|---|
//! | `line` | hot line (`push_line`) |
//! | `polyline`, all bulges zero | hot polyline; a closed one repeats its first vertex (profile section 5) |
//! | `circle` | circle through `push_curve`, domain `0..2 pi` |
//! | `circular_arc` | hot circle arc with the authored start angle and sweep |
//! | `ellipse` with its major axis along `x` | ellipse through `push_curve` |
//! | `bezier` | `push_curve` |
//! | `bspline`, `nurbs` | `push_curve`, knots expanded from their multiplicities; `closed_by_endpoint` declares the curve closed |
//!
//! # What is refused, and why that is the point
//!
//! Anything the capability set cannot hold exactly is refused with the missing capability named
//! ([`Cause::MissingCapability`]), never approximated: a bulge (an arc whose angles need atan2), a periodic
//! B-spline, an ellipse or circle whose frame is rotated (its angle needs `atan2`, whose last bit
//! belongs to the C library and not to the format), geometry off the drawing plane, layer state, and
//! an instance with a transform. One entity that cannot be held refuses the whole fixture, so no
//! artifact ever claims to be a fixture it only partly is.

use pvec_capabilities::{CapabilityId, Registry};
use pvec_container::writer::{ContainerWriter, EntityMeta, SortKey, WriteError, WriterConfig};
use pvec_container::{Bounds, Header, LengthUnit, Manifest, Metadata};
use pvec_corpus::expected::{Entity, EntityClass, Node};
use pvec_corpus::sha256::hex_digest;
use pvec_corpus::{Bundle, CorpusError, ExpectedMath, Fixture, FixtureRegistry};
use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Bezier, Circle, Curve, Ellipse, GeometryError, Placement2, Point2, Polyline, RationalBSpline,
    Vector2,
};
use std::fmt;
use std::path::{Path, PathBuf};

/// The generator's version, recorded in every artifact. Change it when the mapping changes.
pub const GENERATOR_VERSION: &str = "pvec-gen/1";

/// The prefix of the producer id; the fixture id follows it.
pub const PRODUCER_PREFIX: &str = "pvec-gen:";

const GENERATOR_PREFIX: &str = "pvec-gen/";

/// One generated artifact.
#[derive(Clone, Debug)]
pub struct Artifact {
    /// The fixture it was generated for.
    pub fixture_id: String,
    /// The corpus revision it came from.
    pub corpus_revision: String,
    /// The `.pvec` bytes.
    pub bytes: Vec<u8>,
    /// For each entity of the record, in record order: its id there and the `EntityId` it was given.
    pub ids: Vec<(String, u32)>,
}

impl Artifact {
    /// SHA-256 of the bytes, lowercase hex.
    pub fn sha256(&self) -> String {
        hex_digest(&self.bytes)
    }
}

/// Why a fixture was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum Cause {
    /// The current capability set cannot represent the geometry exactly.
    MissingCapability {
        /// The registry capability that would be needed.
        capability: String,
        /// What about the geometry needs it.
        what: String,
    },
    /// The entry has no record.
    Quarantined,
    /// The manifest says no curve may be emitted for it.
    NotEmitted,
    /// Not something a capability would fix (units, a record that does not validate).
    Unsupported {
        /// What it was.
        what: String,
    },
    /// The writer refused.
    Write(WriteError),
    /// The semantic model refused (a numeric policy rule, for example).
    Geometry(GeometryError),
}

/// A refusal, naming the fixture, the entity and the cause.
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// The fixture.
    pub fixture: String,
    /// The entity of the record, when it was one entity that could not be represented.
    pub entity: Option<String>,
    /// Why.
    pub cause: Cause,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.fixture)?;
        if let Some(e) = &self.entity {
            write!(f, "/{e}")?;
        }
        match &self.cause {
            Cause::MissingCapability { capability, what } => {
                write!(f, ": missing capability `{capability}`: {what}")
            }
            Cause::Quarantined => write!(f, ": quarantined, the corpus has no record for it"),
            Cause::NotEmitted => write!(f, ": the corpus says no curve may be emitted for it"),
            Cause::Unsupported { what } => write!(f, ": {what}"),
            Cause::Write(e) => write!(f, ": the writer refused: {e}"),
            Cause::Geometry(e) => write!(f, ": the model refused: {e}"),
        }
    }
}

impl std::error::Error for Refusal {}

/// What one entity becomes.
#[derive(Clone, Debug)]
enum Shape {
    Line([f64; 2], [f64; 2]),
    Arc { centre: [f64; 2], radius: f64, start_sweep: [f64; 2] },
    EllipseArc { centre: [f64; 2], radii: [f64; 2], angles: [f64; 3] },
    Curve(Box<Curve>),
}

#[derive(Clone, Debug)]
struct Op {
    id: EntityId,
    name: String,
    shape: Shape,
}

/// A record turned into writer calls, not yet written.
#[derive(Clone, Debug)]
pub struct Plan {
    fixture: String,
    corpus_revision: String,
    capabilities: Vec<CapabilityId>,
    ops: Vec<Op>,
}

fn boxed(points: &[[f64; 2]]) -> Bounds {
    let first = points.first().copied().unwrap_or([0.0, 0.0]);
    points.iter().fold(Bounds { min: first, max: first }, |b, p| Bounds {
        min: [b.min[0].min(p[0]), b.min[1].min(p[1])],
        max: [b.max[0].max(p[0]), b.max[1].max(p[1])],
    })
}

impl Plan {
    /// How many entities.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// True when there are none.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// For each entity of the record, in record order: its id there and the `EntityId` it was given.
    pub fn ids(&self) -> Vec<(String, u32)> {
        self.ops.iter().map(|o| (o.name.clone(), o.id.index())).collect()
    }

    fn refuse(&self, entity: Option<&str>, cause: Cause) -> Refusal {
        Refusal { fixture: self.fixture.clone(), entity: entity.map(str::to_string), cause }
    }

    fn writer(&self, skip: Option<SortKey>, order: &[usize]) -> Result<ContainerWriter, Refusal> {
        let mut seen = vec![false; self.ops.len()];
        if order.len() != self.ops.len()
            || order.iter().any(|i| *i >= seen.len() || std::mem::replace(&mut seen[*i], true))
        {
            return Err(self.refuse(
                None,
                Cause::Unsupported { what: "the order is not a permutation of the entities".to_string() },
            ));
        }
        let mut w = ContainerWriter::new(WriterConfig {
            unit: LengthUnit::Metre,
            metadata: Metadata {
                producer_id: format!("{PRODUCER_PREFIX}{}", self.fixture),
                producer_version: GENERATOR_VERSION.to_string(),
                corpus_revision: Some(self.corpus_revision.clone()),
            },
            bounds: None,
            capabilities: self.capabilities.clone(),
        });
        if let Some(k) = skip {
            w.skip_sort(k);
        }
        for i in order {
            let op = &self.ops[*i];
            let at = |e: WriteError| self.refuse(Some(&op.name), Cause::Write(e));
            match &op.shape {
                Shape::Line(a, b) => {
                    let meta = EntityMeta { id: op.id, style: 0, cell: 0, bounds: boxed(&[*a, *b]) };
                    w.push_line(meta, *a, *b).map_err(at)?;
                }
                Shape::Arc { centre, radius, start_sweep } => {
                    let bounds = Bounds {
                        min: [centre[0] - radius, centre[1] - radius],
                        max: [centre[0] + radius, centre[1] + radius],
                    };
                    let meta = EntityMeta { id: op.id, style: 0, cell: 0, bounds };
                    w.push_circle_arc(meta, *centre, *radius, *start_sweep).map_err(at)?;
                }
                Shape::EllipseArc { centre, radii, angles } => {
                    let bounds = Bounds {
                        min: [centre[0] - radii[0], centre[1] - radii[1]],
                        max: [centre[0] + radii[0], centre[1] + radii[1]],
                    };
                    let meta = EntityMeta { id: op.id, style: 0, cell: 0, bounds };
                    w.push_ellipse_arc(meta, *centre, *radii, *angles).map_err(at)?;
                }
                Shape::Curve(c) => w.push_curve(op.id, 0, 0, c).map_err(at)?,
            }
        }
        Ok(w)
    }

    /// Writes with the entities pushed in the order of `order`, a permutation of `0..len()`. The
    /// bytes do not depend on it; this is how a test shows that.
    pub fn write_in_order(&self, order: &[usize]) -> Result<Vec<u8>, Refusal> {
        self.writer(None, order)?.finish().map_err(|e| self.refuse(None, Cause::Write(e)))
    }

    /// As [`Plan::write_in_order`] with one sort of the writer switched off, so a test can show that
    /// the determinism check notices a collection left in arrival order.
    pub fn write_skipping(&self, order: &[usize], skip: SortKey) -> Result<Vec<u8>, Refusal> {
        self.writer(Some(skip), order)?.finish().map_err(|e| self.refuse(None, Cause::Write(e)))
    }

    /// Writes in record order.
    pub fn write(&self) -> Result<Vec<u8>, Refusal> {
        let order: Vec<usize> = (0..self.ops.len()).collect();
        self.write_in_order(&order)
    }
}

// ---------------------------------------------------------------- reading a record

struct Ctx<'a> {
    fixture: &'a str,
    entity: &'a str,
}

impl Ctx<'_> {
    fn missing(&self, capability: &str, what: &str) -> Refusal {
        Refusal {
            fixture: self.fixture.to_string(),
            entity: Some(self.entity.to_string()),
            cause: Cause::MissingCapability { capability: capability.to_string(), what: what.to_string() },
        }
    }

    fn unsupported(&self, what: impl Into<String>) -> Refusal {
        Refusal {
            fixture: self.fixture.to_string(),
            entity: Some(self.entity.to_string()),
            cause: Cause::Unsupported { what: what.into() },
        }
    }

    fn geometry(&self, e: GeometryError) -> Refusal {
        Refusal {
            fixture: self.fixture.to_string(),
            entity: Some(self.entity.to_string()),
            cause: Cause::Geometry(e),
        }
    }

    fn real(&self, node: Option<&Node>, what: &str) -> Result<f64, Refusal> {
        node.and_then(Node::as_scalar)
            .map(|s| s.value())
            .ok_or_else(|| self.unsupported(format!("the record has no real for `{what}`")))
    }

    fn triple(&self, node: Option<&Node>, what: &str) -> Result<[f64; 3], Refusal> {
        let list = node.and_then(Node::as_list).filter(|l| l.len() == 3);
        let list = list.ok_or_else(|| self.unsupported(format!("`{what}` is not a point of three reals")))?;
        Ok([
            self.real(Some(&list[0]), what)?,
            self.real(Some(&list[1]), what)?,
            self.real(Some(&list[2]), what)?,
        ])
    }

    /// The point in the drawing plane, or the refusal for geometry off it.
    fn plane(&self, capability: &str, p: [f64; 3], what: &str) -> Result<[f64; 2], Refusal> {
        if p[2] != 0.0 {
            return Err(self.missing(
                capability,
                &format!("`{what}` is off the drawing plane (z is not 0), and the profile is 2D with no implicit promotion"),
            ));
        }
        Ok([p[0], p[1]])
    }

    fn frame(&self, capability: &str, g: &Node, axis_key: &str) -> Result<(), Refusal> {
        let normal = self.triple(g.get("normal"), "normal")?;
        if normal != [0.0, 0.0, 1.0] {
            return Err(self.missing(
                capability,
                "the normal is not +z: a tilted or flipped plane is not the drawing plane",
            ));
        }
        let axis = self.triple(g.get(axis_key), axis_key)?;
        if axis != [1.0, 0.0, 0.0] {
            return Err(self.missing(
                capability,
                "rotated placement: the angle of a frame that is not along x needs atan2, whose last bit is the C library's, not the format's",
            ));
        }
        Ok(())
    }

    fn points(&self, capability: &str, node: Option<&Node>, what: &str) -> Result<Vec<[f64; 2]>, Refusal> {
        let list = node
            .and_then(Node::as_list)
            .ok_or_else(|| self.unsupported(format!("the record has no list for `{what}`")))?;
        list.iter().map(|p| self.plane(capability, self.triple(Some(p), what)?, what)).collect()
    }

    fn knots(&self, node: Option<&Node>) -> Result<Vec<f64>, Refusal> {
        let list =
            node.and_then(Node::as_list).ok_or_else(|| self.unsupported("the record has no knot list"))?;
        let mut out = Vec::new();
        for k in list {
            let v = self.real(k.get("value"), "knots.value")?;
            let m = match k.get("multiplicity") {
                Some(Node::Int(m)) if (1..=64).contains(m) => *m as usize,
                _ => return Err(self.unsupported("a knot multiplicity is not an integer from 1 to 64")),
            };
            out.extend(std::iter::repeat_n(v, m));
        }
        Ok(out)
    }
}

fn pts(points: &[[f64; 2]]) -> Vec<Point2> {
    points.iter().map(|p| Point2::new(p[0], p[1])).collect()
}

fn closed_by_endpoint(e: &Entity) -> bool {
    e.node.get("closure").and_then(|c| c.get("closed_by_endpoint")) == Some(&Node::Bool(true))
}

fn periodic(e: &Entity) -> bool {
    e.node.get("closure").and_then(|c| c.get("periodic")) == Some(&Node::Bool(true))
}

fn shape_of(fixture: &str, e: &Entity) -> Result<(Shape, &'static [&'static str]), Refusal> {
    let cx = Ctx { fixture, entity: &e.id };
    let g = &e.geometry;
    let frame = Placement2::new;
    let origin = |p: [f64; 2]| Point2::new(p[0], p[1]);
    let x_axis = Vector2::new(1.0, 0.0);
    match e.class {
        EntityClass::Line => {
            let a = cx.plane("line", cx.triple(g.get("start"), "start")?, "start")?;
            let b = cx.plane("line", cx.triple(g.get("end"), "end")?, "end")?;
            Ok((Shape::Line(a, b), &["line"]))
        }
        EntityClass::Polyline => {
            let bulges = g.get("bulges").and_then(Node::as_list).unwrap_or(&[]);
            for b in bulges {
                if cx.real(Some(b), "bulges")? != 0.0 {
                    return Err(cx.missing(
                        "trimmed_curve",
                        "a bulge is one circular arc between two vertices; storing it needs the arc's start angle and sweep, which come from atan2 (its last bit is the C library's, not the format's), so it stays refused until pvec-geometry has an exact constructor (#91, from_bulge)",
                    ));
                }
            }
            let mut v = cx.points("polyline", g.get("vertices"), "vertices")?;
            if closed_by_endpoint(e) {
                // the profile has no closed flag on a polyline: a closed one repeats its first vertex
                if let Some(first) = v.first().copied() {
                    v.push(first);
                }
            }
            let poly = Polyline::new(pts(&v)).map_err(|x| cx.geometry(x))?;
            Ok((Shape::Curve(Box::new(Curve::Polyline(poly))), &["polyline"]))
        }
        EntityClass::Circle | EntityClass::CircularArc => {
            let c = cx.plane("circle", cx.triple(g.get("center"), "center")?, "center")?;
            cx.frame("circle", g, "x_axis")?;
            let r = cx.real(g.get("radius"), "radius")?;
            let place = frame(origin(c), x_axis).map_err(|x| cx.geometry(x))?;
            let circle = Circle::new(place, r).map_err(|x| cx.geometry(x))?;
            if e.class == EntityClass::Circle {
                Ok((Shape::Curve(Box::new(Curve::Circle(circle))), &["circle"]))
            } else {
                let start = cx.real(g.get("start_angle"), "start_angle")?;
                let sweep = cx.real(g.get("sweep_angle"), "sweep_angle")?;
                Ok((
                    Shape::Arc { centre: c, radius: circle.radius(), start_sweep: [start, sweep] },
                    &["circle", "trimmed_curve"],
                ))
            }
        }
        EntityClass::Ellipse | EntityClass::EllipticalArc => {
            let c = cx.plane("ellipse", cx.triple(g.get("center"), "center")?, "center")?;
            cx.frame("ellipse", g, "major_axis")?;
            let (a, b) =
                (cx.real(g.get("semi_major"), "semi_major")?, cx.real(g.get("semi_minor"), "semi_minor")?);
            let place = frame(origin(c), x_axis).map_err(|x| cx.geometry(x))?;
            let ellipse = Ellipse::new(place, a, b).map_err(|x| cx.geometry(x))?;
            if e.class == EntityClass::Ellipse {
                Ok((Shape::Curve(Box::new(Curve::Ellipse(ellipse))), &["ellipse"]))
            } else {
                let start = cx.real(g.get("start_angle"), "start_angle")?;
                let sweep = cx.real(g.get("sweep_angle"), "sweep_angle")?;
                Ok((
                    Shape::EllipseArc {
                        centre: c,
                        radii: [ellipse.semi_axis_1(), ellipse.semi_axis_2()],
                        angles: [0.0, start, sweep],
                    },
                    &["ellipse", "trimmed_curve"],
                ))
            }
        }
        EntityClass::Bezier => {
            let cps = cx.points("bezier_curve", g.get("control_points"), "control_points")?;
            let bez = Bezier::new(pts(&cps)).map_err(|x| cx.geometry(x))?;
            Ok((Shape::Curve(Box::new(Curve::Bezier(bez))), &["bezier_curve"]))
        }
        EntityClass::Bspline | EntityClass::Nurbs => {
            let rational = e.class == EntityClass::Nurbs;
            let name: &'static str = if rational { "rational_b_spline_curve" } else { "b_spline_curve" };
            if periodic(e) {
                return Err(cx.missing(
                    name,
                    "a periodic B-spline: the profile has no wrapped periodic form (an unclamped knot vector is not a clamped curve)",
                ));
            }
            let degree = match g.get("degree") {
                Some(Node::Int(d)) if (1..=16).contains(d) => *d as usize,
                _ => return Err(cx.unsupported("the degree is not an integer from 1 to 16")),
            };
            let cps = pts(&cx.points(name, g.get("control_points"), "control_points")?);
            let knots = cx.knots(g.get("knots"))?;
            let curve = if rational {
                let w = g
                    .get("weights")
                    .and_then(Node::as_list)
                    .ok_or_else(|| cx.unsupported("a nurbs record has weights"))?;
                let w =
                    w.iter().map(|x| cx.real(Some(x), "weights")).collect::<Result<Vec<f64>, Refusal>>()?;
                let mut s = RationalBSpline::new(degree, cps, knots, w).map_err(|x| cx.geometry(x))?;
                if closed_by_endpoint(e) {
                    s = s.declare_closed().map_err(|x| cx.geometry(x))?;
                }
                Curve::RationalBSpline(s)
            } else {
                let mut s = BSpline::new(degree, cps, knots).map_err(|x| cx.geometry(x))?;
                if closed_by_endpoint(e) {
                    s = s.declare_closed().map_err(|x| cx.geometry(x))?;
                }
                Curve::BSpline(s)
            };
            let needs: &'static [&'static str] =
                if rational { &["rational_b_spline_curve"] } else { &["b_spline_curve"] };
            Ok((Shape::Curve(Box::new(curve)), needs))
        }
    }
}

/// Maps one record onto writer calls. Refuses the whole record when any entity cannot be held.
pub fn plan(math: &ExpectedMath, corpus_revision: &str) -> Result<Plan, Refusal> {
    let fixture = math.fixture_id.as_str();
    let whole = |what: String| Refusal {
        fixture: fixture.to_string(),
        entity: None,
        cause: Cause::Unsupported { what },
    };
    if math.units.name != "metre" {
        return Err(whole(format!(
            "the unit `{}` is not the metre, and the generator stores metres only",
            math.units.name
        )));
    }
    if math.units.scale_to_canonical.value() != 1.0 {
        return Err(whole(
            "the scale to the canonical unit is not exactly 1, and the generator does not rescale"
                .to_string(),
        ));
    }
    let registry = Registry::v1();
    let mut caps: Vec<CapabilityId> = Vec::new();
    let mut want = |name: &str| {
        if let Some(c) = registry.by_name(name) {
            if !caps.contains(&c.id) {
                caps.push(c.id);
            }
        }
    };
    want("cartesian_point");
    let mut ops = Vec::with_capacity(math.entities.len());
    for (i, e) in math.entities.iter().enumerate() {
        let cx = Ctx { fixture, entity: &e.id };
        if e.node.get("structure") != Some(&Node::Str("CURVE_ONLY".to_string())) {
            return Err(cx.missing(
                "affine_transform_2d",
                "the record keeps an instance plus a transform, and the container stores neither; emitting the world-space result would pass a geometry check and fail the structural one",
            ));
        }
        if e.node.get("layer").is_some() {
            return Err(cx.missing(
                "layer (on/frozen state)",
                "the `layer` capability exists, but its stream stores a name table and an index per entity and has no field for whether a layer is on or frozen, which this record carries",
            ));
        }
        let (shape, needs) = shape_of(fixture, e)?;
        for n in needs {
            want(n);
        }
        let id = u32::try_from(i)
            .ok()
            .and_then(|i| EntityId::new(i).ok())
            .ok_or_else(|| cx.unsupported("too many entities for the id space"))?;
        ops.push(Op { id, name: e.id.clone(), shape });
    }
    Ok(Plan {
        fixture: fixture.to_string(),
        corpus_revision: corpus_revision.to_string(),
        capabilities: caps,
        ops,
    })
}

/// Generates the artifact for one record.
pub fn generate_record(math: &ExpectedMath, corpus_revision: &str) -> Result<Artifact, Refusal> {
    let p = plan(math, corpus_revision)?;
    let bytes = p.write()?;
    Ok(Artifact {
        fixture_id: math.fixture_id.clone(),
        corpus_revision: corpus_revision.to_string(),
        bytes,
        ids: p.ids(),
    })
}

/// Generates the artifact for one registered fixture.
pub fn generate(fixture: &Fixture, corpus_revision: &str) -> Result<Artifact, Refusal> {
    let refuse = |cause| Refusal { fixture: fixture.id.clone(), entity: None, cause };
    let Some(math) = fixture.expected.as_ref() else {
        return Err(refuse(Cause::Quarantined));
    };
    if fixture.must_not_emit_curve {
        return Err(refuse(Cause::NotEmitted));
    }
    generate_record(math, corpus_revision)
}

/// What happened to one fixture.
#[derive(Clone, Debug)]
pub struct Outcome {
    /// The fixture id.
    pub fixture: String,
    /// The artifact or the refusal.
    pub result: Result<Artifact, Refusal>,
}

/// Generates for every fixture of a bundle, in manifest order, quarantined entries last.
pub fn generate_all(bundle: &Bundle) -> Result<Vec<Outcome>, CorpusError> {
    let registry =
        FixtureRegistry::load(bundle, &Registry::v1(), include_str!("../../../docs/ISO_PROFILE.md"))?;
    let revision = bundle.pin().corpus_revision.as_str();
    Ok(registry
        .fixtures()
        .iter()
        .map(|f| Outcome { fixture: f.id.clone(), result: generate(f, revision) })
        .collect())
}

/// What an artifact says about where it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    /// The fixture id.
    pub fixture_id: String,
    /// The corpus revision.
    pub corpus_revision: String,
    /// The generator version.
    pub generator_version: String,
}

/// Why provenance could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvenanceError {
    /// The field that is missing or wrong (`fixture_id`, `corpus_revision`, `generator_version`),
    /// or `manifest` when the artifact has none that parses.
    pub field: &'static str,
}

/// Reads the provenance out of an artifact; a missing or empty field is an error naming it.
pub fn provenance(bytes: &[u8]) -> Result<Provenance, ProvenanceError> {
    let bad = |field| ProvenanceError { field };
    let len = bytes.len() as u64;
    let header = Header::parse(&bytes[..bytes.len().min(4096)], len).map_err(|_| bad("manifest"))?;
    let start = usize::try_from(header.manifest_offset).map_err(|_| bad("manifest"))?;
    let end = usize::try_from(header.manifest_offset.saturating_add(header.manifest_length))
        .map_err(|_| bad("manifest"))?;
    let manifest = Manifest::parse(bytes.get(start..end).ok_or(bad("manifest"))?, &header, len)
        .map_err(|_| bad("manifest"))?;
    let m = manifest.metadata;
    let fixture_id = m
        .producer_id
        .strip_prefix(PRODUCER_PREFIX)
        .filter(|s| !s.is_empty())
        .ok_or(bad("fixture_id"))?
        .to_string();
    if !m.producer_version.starts_with(GENERATOR_PREFIX) || m.producer_version.len() == GENERATOR_PREFIX.len()
    {
        return Err(bad("generator_version"));
    }
    let corpus_revision = m.corpus_revision.filter(|s| !s.is_empty()).ok_or(bad("corpus_revision"))?;
    Ok(Provenance { fixture_id, corpus_revision, generator_version: m.producer_version })
}

/// Writes every generated artifact as `<dir>/<fixture>.pvec` and returns the paths, in fixture order.
/// A refused fixture writes nothing.
pub fn emit(dir: &Path, outcomes: &[Outcome]) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for o in outcomes {
        if let Ok(a) = &o.result {
            let path = dir.join(format!("{}.pvec", a.fixture_id));
            std::fs::write(&path, &a.bytes)?;
            out.push(path);
        }
    }
    Ok(out)
}
