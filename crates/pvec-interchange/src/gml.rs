//! The GML export (issue P6.4).
//!
//! PVec curves go out as GML 3.2.1 (ISO 19136:2007) geometry: a `gml:MultiCurve` with one `gml:Curve`
//! per entity, each holding a list of curve segments. The export maps every curve onto the richest
//! construct GML has that says the same thing, and writes one fidelity record per segment it emitted
//! (`FidelityLedger`, from P6.1) so a consumer can tell the exact parts from the approximated ones.
//! The normative text, with the mapping table, is `docs/INTERCHANGE.md` chapter 3.
//!
//! Three rules shape the module:
//!
//! * The caller names the coordinate reference system and the source-to-CRS transform
//!   ([`CrsTransform`]). The export never picks one, and writes both into the output.
//! * Nothing is approximated unless GML has no construct for the curve. An approximation carries a
//!   measured error bound (in the units of the output CRS), and a mapping that cannot show its
//!   evidence is demoted to the next value instead of claiming the stronger one.
//! * No tolerance is written here. Comparisons use the numeric policy of P1.2 (`pvec_numeric::Policy`),
//!   and the only other number the caller supplies is the deviation budget for approximations.

use crate::report::{Report, ReportError};
use crate::{
    ErrorBound, Fidelity, FidelityKind, FidelityLedger, FidelityRecord, Reason, Representation,
    TargetConstruct,
};
use core::fmt::Write as _;
use pvec_capabilities::{CapabilityId, Registry};
use pvec_entity::EntityId;
use pvec_geometry::{BSpline, Bezier, Curve, Curve2d, Point2, RationalBSpline, Transform2, Trimmed};
use pvec_numeric::{MAX_POLYLINE_VERTICES, Policy};

/// The GML version the export declares. The schema files are the ones OGC publishes under
/// `schemas.opengis.net/gml/3.2.1/`.
pub const GML_VERSION: &str = "3.2.1";
/// The GML namespace of that version.
pub const GML_NAMESPACE: &str = "http://www.opengis.net/gml/3.2";
/// The code space of the `gml:identifier` that carries a PVec entity id.
pub const ENTITY_CODE_SPACE: &str = "urn:pvec:entity";

const POLICY: Policy = Policy::V1;

/// The factor a sampled deviation is multiplied by before it is recorded as a bound. Sampling a smooth
/// chord error finds the maximum to well within this, and the factor also covers the distance from the
/// chord back to the curve, which for the flat pieces used here is the same size as the sag.
const BOUND_SAFETY: f64 = 1.25;
/// Interior samples per piece when choosing the subdivision, and when measuring the final bound.
const SAMPLES_SEARCH: usize = 16;
const SAMPLES_FINAL: usize = 64;

/// Why an export was refused outright. An entity that cannot be mapped is not an error: it is an
/// `Unsupported` record in the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GmlError {
    /// The CRS name is blank or contains a control character.
    BadSrsName,
    /// The deviation budget is not finite and strictly positive.
    BadBudget,
    /// Two entities share an id, so their `gml:id` values would collide.
    DuplicateEntity(EntityId),
}

impl core::fmt::Display for GmlError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            GmlError::BadSrsName => {
                write!(f, "the CRS name must be a non-blank string without control characters")
            }
            GmlError::BadBudget => write!(f, "the deviation budget must be finite and strictly positive"),
            GmlError::DuplicateEntity(id) => write!(f, "entity {} appears twice", id.index()),
        }
    }
}

impl std::error::Error for GmlError {}

/// The target CRS and the transform into it, both supplied by the caller. The export applies the
/// transform to the geometry exactly (`Curve2d::transform`), writes the name as `srsName` and writes the
/// matrix into the output, so the file says what was done to the coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsTransform {
    srs_name: String,
    transform: Transform2,
}

impl CrsTransform {
    /// A CRS name (an EPSG URN, say) and the source-to-CRS transform. Nothing here knows what the name
    /// means: no CRS database is consulted and no reprojection is done beyond the affine map.
    pub fn new(srs_name: &str, transform: Transform2) -> Result<CrsTransform, GmlError> {
        if srs_name.trim().is_empty() || srs_name.chars().any(char::is_control) {
            return Err(GmlError::BadSrsName);
        }
        Ok(CrsTransform { srs_name: srs_name.to_string(), transform })
    }

    /// The CRS name.
    pub fn srs_name(&self) -> &str {
        &self.srs_name
    }

    /// The transform.
    pub fn transform(&self) -> &Transform2 {
        &self.transform
    }
}

/// The largest deviation, in the units of the output CRS, an approximated curve may have. It is the
/// caller's display budget; the export subdivides until the measured bound is inside it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Budget(f64);

impl Budget {
    /// A budget, which must be finite and strictly positive.
    pub fn new(max_deviation: f64) -> Result<Budget, GmlError> {
        if max_deviation.is_finite() && max_deviation > 0.0 {
            Ok(Budget(max_deviation))
        } else {
            Err(GmlError::BadBudget)
        }
    }

    /// The budget.
    pub fn get(self) -> f64 {
        self.0
    }
}

/// What an export produced.
#[derive(Clone, Debug, PartialEq)]
pub struct GmlExport {
    /// The document, UTF-8, byte-identical for identical input.
    pub xml: String,
    /// One record per emitted segment, and one `Unsupported` record per entity that was left out.
    pub ledger: FidelityLedger,
}

// ---- the mapping table ------------------------------------------------------------------------------

/// A way a PVec capability is mapped. Each variant is one row of the table in `docs/INTERCHANGE.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// A point of a curve.
    Point,
    /// A direction or vector.
    Direction,
    /// An axis placement.
    Placement,
    /// A line with no trim.
    LineUnbounded,
    /// A polyline.
    Polyline,
    /// A circle.
    CircleFull,
    /// A circle whose three-point form could not show the evidence.
    CircleTessellated,
    /// An ellipse, full or trimmed through its trim.
    EllipseTessellated,
    /// A trimmed line.
    TrimmedLine,
    /// A circular arc.
    TrimmedArc,
    /// A trimmed curve covering a whole circle.
    TrimmedFullCircle,
    /// A trimmed Bezier or B-spline, cut out of the basis by knot insertion.
    TrimmedSpline,
    /// A trimmed curve GML has no construct for.
    TrimmedTessellated,
    /// A composite curve.
    Composite,
    /// A Bezier curve.
    Bezier,
    /// A B-spline.
    BSpline,
    /// A rational B-spline whose weights are all the same value.
    RationalEqualWeights,
    /// A rational B-spline with differing weights.
    RationalTessellated,
    /// The affine transform.
    Transform,
    /// A component stream.
    Component,
}

/// One row of the mapping table: a capability, the case, the construct and the one value it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingRow {
    /// The capability's registry name.
    pub capability: &'static str,
    /// The case within that capability.
    pub case: Case,
    /// The GML construct written, or `none`.
    pub construct: &'static str,
    /// Native or tessellated.
    pub representation: Representation,
    /// The fidelity value this case carries.
    pub fidelity: FidelityKind,
    /// For `Unsupported`, the reason. For the others, how the value is obtained.
    pub note: &'static str,
}

const NATIVE: Representation = Representation::Native;
const TESS: Representation = Representation::Tessellated;

macro_rules! row {
    ($cap:expr, $case:ident, $construct:expr, $rep:expr, $fid:ident, $note:expr) => {
        MappingRow {
            capability: $cap,
            case: Case::$case,
            construct: $construct,
            representation: $rep,
            fidelity: FidelityKind::$fid,
            note: $note,
        }
    };
}

const COMPONENT_REASON: &str =
    "a component stream is not geometry; the GML export writes curve geometry only";

/// The mapping table. Every capability of the v1 registry has at least one row, and every row carries
/// exactly one fidelity value.
pub const MAPPING: &[MappingRow] = &[
    row!(
        "cartesian_point",
        Point,
        "gml:pos",
        NATIVE,
        Exact,
        "both coordinates are written as the shortest decimal that reads back to the same f64"
    ),
    row!(
        "direction_vector",
        Direction,
        "none",
        NATIVE,
        Unsupported,
        "a direction is not a curve and GML has no direction construct; it is consumed by the circle and line mappings"
    ),
    row!(
        "axis_placement_2d",
        Placement,
        "none",
        NATIVE,
        Unsupported,
        "a placement is not a curve and GML has no placement construct; it is consumed by the circle mapping"
    ),
    row!(
        "line",
        LineUnbounded,
        "none",
        NATIVE,
        Unsupported,
        "an unbounded line has no GML construct; a bounded one is a trimmed curve"
    ),
    row!("polyline", Polyline, "gml:LineStringSegment", NATIVE, Exact, "every vertex is carried unchanged"),
    row!(
        "circle",
        CircleFull,
        "gml:Circle",
        NATIVE,
        EquivalentCompound,
        "three points on the circle; the circle through them is checked against the source under NP-04 and the start point is the first one"
    ),
    row!(
        "circle",
        CircleTessellated,
        "gml:LineStringSegment",
        TESS,
        BoundedApproximation,
        "taken only when the three points cannot reproduce the circle under NP-04"
    ),
    row!(
        "ellipse",
        EllipseTessellated,
        "gml:LineStringSegment",
        TESS,
        BoundedApproximation,
        "GML 3.2.1 has no ellipse; a sampled polyline with a measured bound"
    ),
    row!(
        "trimmed_curve",
        TrimmedLine,
        "gml:LineStringSegment",
        NATIVE,
        ExactReparameterized,
        "the two end points, evaluated by the shared evaluator; origin and direction become the end points"
    ),
    row!(
        "trimmed_curve",
        TrimmedArc,
        "gml:Arc",
        NATIVE,
        EquivalentCompound,
        "start, middle and end point of the arc; checked against the source under NP-04"
    ),
    row!(
        "trimmed_curve",
        TrimmedFullCircle,
        "gml:Circle",
        NATIVE,
        EquivalentCompound,
        "as a circle; the start point and direction are kept"
    ),
    row!(
        "trimmed_curve",
        TrimmedSpline,
        "gml:BSpline",
        NATIVE,
        ExactReparameterized,
        "the basis with both trim parameters inserted as knots, then cut to a clamped spline over the trimmed curve's own parameter (tau = s - start, or start + span - s when reversed)"
    ),
    row!(
        "trimmed_curve",
        TrimmedTessellated,
        "gml:LineStringSegment",
        TESS,
        BoundedApproximation,
        "an ellipse or rational spline trim, or an arc that failed the three-point check; a sampled polyline with a measured bound"
    ),
    row!(
        "composite_curve",
        Composite,
        "gml:Curve",
        NATIVE,
        Exact,
        "the segment list keeps the order and the joins; each member has its own record"
    ),
    row!(
        "bezier_curve",
        Bezier,
        "gml:Bezier",
        NATIVE,
        Exact,
        "degree and control points carried; the two knots are the fixed domain 0 to 1"
    ),
    row!(
        "b_spline_curve",
        BSpline,
        "gml:BSpline",
        NATIVE,
        Exact,
        "degree, control points and knots (value and multiplicity) carried"
    ),
    row!(
        "rational_b_spline_curve",
        RationalEqualWeights,
        "gml:BSpline",
        NATIVE,
        EquivalentCompound,
        "equal weights cancel, so the polynomial spline has the same locus"
    ),
    row!(
        "rational_b_spline_curve",
        RationalTessellated,
        "gml:LineStringSegment",
        TESS,
        BoundedApproximation,
        "GML 3.2.1 cannot carry control point weights; a sampled polyline with a measured bound"
    ),
    row!(
        "affine_transform_2d",
        Transform,
        "none",
        NATIVE,
        Unsupported,
        "GML has no transform construct; the matrix is applied to the geometry and written into the description"
    ),
    row!("layer", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("line_style", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("text", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("hatch", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("source_provenance", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("dwg_semantics", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("semantic_class", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("paint_order", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("stroke_paint", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("fill_paint", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!("clip", Component, "none", NATIVE, Unsupported, COMPONENT_REASON),
    row!(
        "raster",
        Component,
        "none",
        NATIVE,
        Unsupported,
        "a raster is pixels, not geometry; the GML export writes curve geometry only and records the raster as Unsupported (ADR 0018)"
    ),
    row!(
        "calibration",
        Component,
        "none",
        NATIVE,
        Unsupported,
        "a calibration is a scale and a georeference for the page, not geometry; the GML export writes curve geometry only and records it as Unsupported (ADR 0019)"
    ),
];

/// The row for a geometry case, which is unique per `(capability, case)`.
fn row_of(case: Case) -> &'static MappingRow {
    // Component rows are the only ones that share a case, and nothing emits them.
    MAPPING.iter().find(|r| r.case == case).expect("every emitted case has a row")
}

/// The table as the markdown `docs/INTERCHANGE.md` carries, so the document and the code cannot drift.
pub fn mapping_table_markdown() -> String {
    let mut s =
        String::from("| capability | case | GML construct | fidelity | how |\n|---|---|---|---|---|\n");
    for r in MAPPING {
        let case = format!("{:?}", r.case);
        let _ = writeln!(
            s,
            "| `{}` | {} | `{}` | `{}` | {} |",
            r.capability,
            case,
            r.construct,
            r.fidelity.name(),
            r.note
        );
    }
    s
}

// ---- the export ---------------------------------------------------------------------------------------

/// Exports `entities` to GML. Each entity is a curve in the source coordinates; the transform in `crs`
/// is applied to every one before it is mapped, so the fidelity values describe the curve in the output
/// CRS. Entities appear in the order given.
pub fn export(
    entities: &[(EntityId, Curve)],
    crs: &CrsTransform,
    budget: Budget,
) -> Result<GmlExport, GmlError> {
    let registry = Registry::v1();
    let mut seen: Vec<u32> = entities.iter().map(|(id, _)| id.index()).collect();
    seen.sort_unstable();
    if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
        return Err(GmlError::DuplicateEntity(EntityId::new(w[0]).expect("an id that was already valid")));
    }

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        xml,
        "<gml:MultiCurve xmlns:gml=\"{GML_NAMESPACE}\" gml:id=\"pvec-export\" srsName=\"{}\">",
        escape(crs.srs_name())
    );
    let m = crs.transform().matrix();
    let _ = writeln!(
        xml,
        "<gml:description>PVec export, GML {GML_VERSION}. Source-to-CRS affine transform, rows [a b tx] and [c d ty]: \
         a={} b={} tx={} c={} d={} ty={}. Approximated curves stay within {} of the source.</gml:description>",
        num(m[0]),
        num(m[1]),
        num(m[4]),
        num(m[2]),
        num(m[3]),
        num(m[5]),
        num(budget.get())
    );

    let mut ledger = FidelityLedger::new();
    for (id, curve) in entities {
        let mapped = match curve.transform(crs.transform()) {
            Ok((moved, _)) => map_curve(&moved, budget),
            Err(e) => Err(Unmapped::new(
                capability_of(curve),
                format!("the CRS transform is refused: {}", e.rule()),
            )),
        };
        match mapped {
            Ok(segments) => {
                let _ = write!(
                    xml,
                    "<gml:curveMember><gml:Curve gml:id=\"e{n}\"><gml:identifier codeSpace=\"{ENTITY_CODE_SPACE}\">{n}</gml:identifier><gml:segments>",
                    n = id.index()
                );
                for s in segments {
                    xml.push_str(&s.xml);
                    ledger.record(s.record(*id, &registry));
                }
                xml.push_str("</gml:segments></gml:Curve></gml:curveMember>\n");
            }
            Err(u) => ledger.record(u.record(*id, &registry)),
        }
    }
    xml.push_str("</gml:MultiCurve>\n");
    Ok(GmlExport { xml, ledger })
}

/// One emitted segment and what to record about it.
struct Segment {
    xml: String,
    capability: &'static str,
    case: Case,
    bound: Option<f64>,
    note: String,
}

impl Segment {
    fn record(self, id: EntityId, registry: &Registry) -> FidelityRecord {
        let row = row_of(self.case);
        let fidelity = match (row.fidelity, self.bound) {
            (FidelityKind::BoundedApproximation, Some(b)) => Fidelity::BoundedApproximation(
                ErrorBound::new(b).expect("a measured bound is finite and positive"),
            ),
            (FidelityKind::Exact, _) => Fidelity::Exact,
            (FidelityKind::ExactReparameterized, _) => Fidelity::ExactReparameterized,
            (FidelityKind::EquivalentCompound, _) => Fidelity::EquivalentCompound,
            _ => unreachable!("a segment is never unsupported and a bounded one always has its bound"),
        };
        let target =
            TargetConstruct::new(row.construct, row.representation).expect("table names are not blank");
        let cap = capability_id(registry, self.capability);
        FidelityRecord::new(id, cap, target, fidelity, &self.note)
            .expect("the table never pairs a tessellation with an exact value")
    }
}

/// A curve the export left out, and why.
struct Unmapped {
    capability: &'static str,
    reason: String,
}

impl Unmapped {
    fn new(capability: &'static str, reason: String) -> Unmapped {
        Unmapped { capability, reason }
    }

    fn record(self, id: EntityId, registry: &Registry) -> FidelityRecord {
        let target = TargetConstruct::new("none", Representation::Native).expect("not blank");
        let reason = Reason::new(&self.reason).expect("every reason says something");
        FidelityRecord::new(
            id,
            capability_id(registry, self.capability),
            target,
            Fidelity::Unsupported(reason),
            "",
        )
        .expect("an unsupported record is always valid")
    }
}

fn capability_id(registry: &Registry, name: &str) -> CapabilityId {
    registry.by_name(name).expect("the mapping names registered capabilities").id
}

fn capability_of(c: &Curve) -> &'static str {
    match c {
        Curve::Line(_) => "line",
        Curve::Circle(_) => "circle",
        Curve::Ellipse(_) => "ellipse",
        Curve::Polyline(_) => "polyline",
        Curve::Trimmed(_) => "trimmed_curve",
        Curve::Composite(_) => "composite_curve",
        Curve::Bezier(_) => "bezier_curve",
        Curve::BSpline(_) => "b_spline_curve",
        Curve::RationalBSpline(_) => "rational_b_spline_curve",
        _ => "composite_curve",
    }
}

fn map_curve(c: &Curve, budget: Budget) -> Result<Vec<Segment>, Unmapped> {
    match c {
        Curve::Line(_) => Err(Unmapped::new("line", row_of(Case::LineUnbounded).note.to_string())),
        Curve::Polyline(p) => Ok(vec![Segment {
            xml: line_string(p.vertices()),
            capability: "polyline",
            case: Case::Polyline,
            bound: None,
            note: format!("{} vertices", p.vertices().len()),
        }]),
        Curve::Circle(circle) => {
            let o = circle.placement().origin();
            circle_or_arc(
                c,
                "circle",
                Case::CircleFull,
                Case::CircleTessellated,
                (o, circle.radius()),
                false,
                budget,
            )
        }
        Curve::Ellipse(_) => tessellate_segment(c, "ellipse", Case::EllipseTessellated, budget),
        Curve::Trimmed(t) => map_trimmed(c, t, budget),
        Curve::Composite(comp) => {
            let mut out = Vec::new();
            for member in comp.segments() {
                match map_curve(member, budget) {
                    Ok(mut s) => out.append(&mut s),
                    Err(u) => {
                        return Err(Unmapped::new(
                            "composite_curve",
                            format!("a member ({}) cannot be written: {}", u.capability, u.reason),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Curve::Bezier(b) => Ok(vec![bezier(b)]),
        Curve::BSpline(b) => {
            Ok(vec![b_spline(b, "b_spline_curve", Case::BSpline, "polynomial spline".to_string())])
        }
        Curve::RationalBSpline(r) => rational(c, r, budget),
        _ => Err(Unmapped::new("composite_curve", "a curve kind newer than this export".to_string())),
    }
}

fn map_trimmed(c: &Curve, t: &Trimmed, budget: Budget) -> Result<Vec<Segment>, Unmapped> {
    match t.basis() {
        Curve::Line(_) => {
            let d = c.domain();
            let (a, b) = (point(c.eval(d.lo()), "trimmed_curve")?, point(c.eval(d.hi()), "trimmed_curve")?);
            Ok(vec![Segment {
                xml: line_string(&[a, b]),
                capability: "trimmed_curve",
                case: Case::TrimmedLine,
                bound: None,
                note: "end points evaluated from the trimmed line".to_string(),
            }])
        }
        Curve::Circle(circle) => {
            let o = circle.placement().origin();
            let (exact, tess) = if t.is_full_period() {
                (Case::TrimmedFullCircle, Case::TrimmedTessellated)
            } else {
                (Case::TrimmedArc, Case::TrimmedTessellated)
            };
            circle_or_arc(c, "trimmed_curve", exact, tess, (o, circle.radius()), !t.is_full_period(), budget)
        }
        Curve::Bezier(_) | Curve::BSpline(_) => match t.as_bspline() {
            Ok(b) => {
                let tr = t.trim();
                let map = if tr.reversed {
                    format!("tau = {} - s", num(tr.start + tr.span))
                } else {
                    format!("tau = s - {}", num(tr.start))
                };
                Ok(vec![b_spline(
                    &b,
                    "trimmed_curve",
                    Case::TrimmedSpline,
                    format!(
                        "trim [{}, {}] of the basis parameter s cut by knot insertion; {map}",
                        num(tr.start),
                        num(tr.start + tr.span)
                    ),
                )])
            }
            // a trim the cut cannot represent keeps the measured polyline
            Err(_) => tessellate_segment(c, "trimmed_curve", Case::TrimmedTessellated, budget),
        },
        _ => tessellate_segment(c, "trimmed_curve", Case::TrimmedTessellated, budget),
    }
}

/// A circle or an arc as three points, kept only if the circle through those points is the source
/// circle under NP-04; otherwise a bounded polyline.
fn circle_or_arc(
    c: &Curve,
    capability: &'static str,
    exact: Case,
    fallback: Case,
    (centre, radius): (Point2, f64),
    arc: bool,
    budget: Budget,
) -> Result<Vec<Segment>, Unmapped> {
    let d = c.domain();
    let third = |k: f64| d.lo() + d.width() * k;
    let ts = if arc { [d.lo(), third(0.5), d.hi()] } else { [d.lo(), third(1.0 / 3.0), third(2.0 / 3.0)] };
    let mut pts = [Point2::default(); 3];
    for (p, t) in pts.iter_mut().zip(ts) {
        *p = point(c.eval(t), capability)?;
    }
    let scale = radius;
    let mag = centre.x.abs().max(centre.y.abs()) + radius;
    let ok = circumcircle(pts[0], pts[1], pts[2]).map(|(cc, r)| {
        let dev = cc.distance(centre).max((r - radius).abs());
        (dev, dev <= POLICY.tol(scale, mag))
    });
    match ok {
        Some((dev, true)) => {
            let tag = if arc { "gml:Arc" } else { "gml:Circle" };
            Ok(vec![Segment {
                xml: three_point(tag, pts),
                capability,
                case: exact,
                bound: None,
                note: format!("circle through the three points differs from the source by {}", num(dev)),
            }])
        }
        _ => tessellate_segment(c, capability, fallback, budget),
    }
}

/// The centre and radius of the circle through three points, or `None` for collinear ones.
fn circumcircle(a: Point2, b: Point2, c: Point2) -> Option<(Point2, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d == 0.0 || !d.is_finite() {
        return None;
    }
    let (a2, b2, c2) = (a.x * a.x + a.y * a.y, b.x * b.x + b.y * b.y, c.x * c.x + c.y * c.y);
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let centre = Point2::new(ux, uy);
    let r = centre.distance(a);
    (centre.x.is_finite() && centre.y.is_finite() && r.is_finite()).then_some((centre, r))
}

fn bezier(b: &Bezier) -> Segment {
    let degree = b.degree();
    let mut x = String::new();
    let _ = write!(
        x,
        "<gml:Bezier isPolynomial=\"true\">{}<gml:degree>{degree}</gml:degree>{}{}</gml:Bezier>",
        pos_list(b.control_points()),
        knot(0.0, degree + 1),
        knot(1.0, degree + 1)
    );
    Segment {
        xml: x,
        capability: "bezier_curve",
        case: Case::Bezier,
        bound: None,
        note: format!("degree {degree}"),
    }
}

fn b_spline(b: &BSpline, capability: &'static str, case: Case, note: String) -> Segment {
    Segment {
        xml: spline_xml(b.degree(), b.control_points(), b.knots()),
        capability,
        case,
        bound: None,
        note,
    }
}

fn rational(c: &Curve, r: &RationalBSpline, budget: Budget) -> Result<Vec<Segment>, Unmapped> {
    let w = r.weights();
    if w.iter().all(|x| x.to_bits() == w[0].to_bits()) {
        Ok(vec![Segment {
            xml: spline_xml(r.degree(), r.control_points(), r.knots()),
            capability: "rational_b_spline_curve",
            case: Case::RationalEqualWeights,
            bound: None,
            note: "all weights equal".to_string(),
        }])
    } else {
        tessellate_segment(c, "rational_b_spline_curve", Case::RationalTessellated, budget)
    }
}

/// The `gml:BSpline` element. The knot weight GML requires is the ISO 19107 averaging weight of the
/// knot, which PVec has no counterpart for; it is written as 1 and carries no information.
fn spline_xml(degree: usize, points: &[Point2], knots: &[f64]) -> String {
    let mut s =
        format!("<gml:BSpline isPolynomial=\"true\">{}<gml:degree>{degree}</gml:degree>", pos_list(points));
    let mut i = 0;
    while i < knots.len() {
        let mut j = i;
        while j < knots.len() && knots[j].to_bits() == knots[i].to_bits() {
            j += 1;
        }
        s.push_str(&knot(knots[i], j - i));
        i = j;
    }
    s.push_str("</gml:BSpline>");
    s
}

fn knot(value: f64, multiplicity: usize) -> String {
    format!(
        "<gml:knot><gml:Knot><gml:value>{}</gml:value><gml:multiplicity>{multiplicity}</gml:multiplicity><gml:weight>1</gml:weight></gml:Knot></gml:knot>",
        num(value)
    )
}

// ---- approximation -------------------------------------------------------------------------------------

fn tessellate_segment(
    c: &Curve,
    capability: &'static str,
    case: Case,
    budget: Budget,
) -> Result<Vec<Segment>, Unmapped> {
    let (pts, bound) = tessellate(c, budget.get()).map_err(|why| Unmapped::new(capability, why))?;
    Ok(vec![Segment {
        xml: line_string(&pts),
        capability,
        case,
        bound: Some(bound),
        note: format!(
            "{} vertices; bound measured over {SAMPLES_FINAL} samples per piece against the shared evaluator",
            pts.len()
        ),
    }])
}

/// The parameters the curve is not smooth across (the knots of a spline, in the curve's own
/// parameter), inside its domain.
fn breaks(c: &Curve) -> Vec<f64> {
    let d = c.domain();
    let mut out = vec![d.lo()];
    let knots: Vec<f64> = match c {
        Curve::BSpline(b) => b.knots().to_vec(),
        Curve::RationalBSpline(r) => r.knots().to_vec(),
        Curve::Trimmed(t) => {
            let tr = t.trim();
            let basis: &[f64] = match t.basis() {
                Curve::BSpline(b) => b.knots(),
                Curve::RationalBSpline(r) => r.knots(),
                _ => &[],
            };
            basis.iter().map(|&k| if tr.reversed { tr.start + tr.span - k } else { k - tr.start }).collect()
        }
        _ => Vec::new(),
    };
    let mut inner: Vec<f64> = knots.into_iter().filter(|&k| k > d.lo() && k < d.hi()).collect();
    inner.sort_by(f64::total_cmp);
    inner.dedup();
    out.extend(inner);
    out.push(d.hi());
    out
}

/// A polyline within `budget` of the curve, and the bound it was measured to have.
fn tessellate(c: &Curve, budget: f64) -> Result<(Vec<Point2>, f64), String> {
    let marks = breaks(c);
    let pieces = marks.len() - 1;
    let mut per = 1usize;
    loop {
        if per.saturating_mul(pieces).saturating_add(1) > MAX_POLYLINE_VERTICES {
            return Err(format!(
                "the deviation budget {} needs more than {MAX_POLYLINE_VERTICES} vertices",
                num(budget)
            ));
        }
        let params = parameters(&marks, per);
        let (pts, searched) = measure(c, &params, SAMPLES_SEARCH)?;
        if searched * BOUND_SAFETY <= budget {
            let (_, fine) = measure(c, &params, SAMPLES_FINAL)?;
            // A bound below what the numeric policy resolves at this size and magnitude would claim a
            // precision the coordinates do not have (a curve that is flat to the last bit measures
            // zero), so the policy tolerance is the floor.
            let floor = floor_of(c)?;
            let bound = (fine * BOUND_SAFETY).max(floor);
            if bound <= budget {
                return Ok((pts, bound));
            }
            if floor > budget {
                return Err(format!(
                    "the deviation budget {} is below what the numeric policy resolves for this curve ({})",
                    num(budget),
                    num(floor)
                ));
            }
        }
        per = per.checked_mul(2).ok_or("the subdivision overflowed")?;
    }
}

/// The NP-04 tolerance for the curve's own size and magnitude: the smallest error worth recording.
fn floor_of(c: &Curve) -> Result<f64, String> {
    let b = c.bounds().map_err(|e| format!("bounds failed: {}", e.rule()))?;
    Ok(POLICY.tol(b.diagonal(), b.magnitude()))
}

fn parameters(marks: &[f64], per: usize) -> Vec<f64> {
    let mut ts = Vec::with_capacity((marks.len() - 1) * per + 1);
    for w in marks.windows(2) {
        for k in 0..per {
            ts.push(w[0] + (w[1] - w[0]) * (k as f64 / per as f64));
        }
    }
    ts.push(marks[marks.len() - 1]);
    ts
}

/// The vertices at `params` and the largest distance from `samples` interior points of each piece to
/// that piece's chord.
fn measure(c: &Curve, params: &[f64], samples: usize) -> Result<(Vec<Point2>, f64), String> {
    let ev = |t: f64| c.eval(t).map_err(|e| format!("evaluation failed: {}", e.rule()));
    let pts = params.iter().map(|&t| ev(t)).collect::<Result<Vec<_>, _>>()?;
    let mut worst = 0.0f64;
    for (i, w) in params.windows(2).enumerate() {
        for s in 1..=samples {
            let t = w[0] + (w[1] - w[0]) * (s as f64 / (samples + 1) as f64);
            worst = worst.max(distance_to_segment(ev(t)?, pts[i], pts[i + 1]));
        }
    }
    Ok((pts, worst))
}

fn distance_to_segment(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    p.distance(Point2::new(a.x + t * dx, a.y + t * dy))
}

fn point(
    r: Result<Point2, pvec_geometry::GeometryError>,
    capability: &'static str,
) -> Result<Point2, Unmapped> {
    r.map_err(|e| Unmapped::new(capability, format!("evaluation failed: {}", e.rule())))
}

// ---- writing -----------------------------------------------------------------------------------------

/// A number as the shortest decimal that reads back to the same `f64`, with negative zero as zero.
fn num(v: f64) -> String {
    if v == 0.0 { "0".to_string() } else { format!("{v}") }
}

fn pos_list(points: &[Point2]) -> String {
    let mut s = String::from("<gml:posList srsDimension=\"2\">");
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        let _ = write!(s, "{} {}", num(p.x), num(p.y));
    }
    s.push_str("</gml:posList>");
    s
}

fn line_string(points: &[Point2]) -> String {
    format!("<gml:LineStringSegment>{}</gml:LineStringSegment>", pos_list(points))
}

fn three_point(tag: &str, p: [Point2; 3]) -> String {
    format!("<{tag}>{}</{tag}>", pos_list(&p))
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c => o.push(c),
        }
    }
    o
}

impl GmlExport {
    /// The fidelity report of this export (P6.8), for an artifact of `artifact_entities` entities
    /// whose digest is `source_digest`.
    pub fn report(&self, source_digest: [u8; 32], artifact_entities: u32) -> Result<Report, ReportError> {
        Report::new("GML", GML_VERSION, source_digest, artifact_entities, self.ledger.records())
    }
}
