//! The Part 21 writer (issue P6.2).
//!
//! The file is an ISO 10303-21:2016 exchange structure for the AP242 schema. Every curve is written
//! from the semantic model (`pvec_geometry::Curve`) straight onto the Part 42 entities of
//! `docs/ISO_PROFILE.md`: degree, control points, knots with multiplicities, weights, the declared-closed
//! flag and the trim domain are read off the curve and written as they are. There is no path from a
//! sampled or preview representation to this writer, because its input type, [`StepEntity`], can only be
//! made from an exact curve (see [`StepEntity::from_table`] and the examples on [`export`]).
//!
//! Three rules shape the module:
//!
//! * Output is a function of the input. Instance numbers are assigned in the order entities are given,
//!   children before their parent, from 1 and without gaps; numbers are the shortest decimal that
//!   reads back to the same `f64`; and the only time in the header is the one the caller passes.
//! * One record per entity goes in the [`FidelityLedger`], with the value the mapping table
//!   ([`MAPPING`]) gives that case. An entity the curve model refuses to transform is an `Unsupported`
//!   record with the reason, and nothing is written for it.
//! * No tolerance is written here. The curve model decides what a valid curve is.

use super::mapping::{Case, row_of};
use crate::report::RasterRow;
use crate::{
    Fidelity, FidelityKind, FidelityLedger, FidelityRecord, Reason, Representation, TargetConstruct,
};
use core::fmt::Write as _;
use pvec_capabilities::{CapabilityId, Registry};
use pvec_entity::{EntityId, EntityTable, Representation as EntityRepresentation};
use pvec_geometry::{
    BSpline, Bezier, Curve, Curve2d, Placement2, Point2, RationalBSpline, Transform2, Vector2,
};

/// The schema identifier in `FILE_SCHEMA`: AP242 managed model based 3D engineering, MIM long form.
pub const SCHEMA_ID: &str = "AP242_MANAGED_MODEL_BASED_3D_ENGINEERING_MIM_LF { 1 0 10303 442 1 1 4 }";

/// Why an export was refused outright. An entity that cannot be mapped is not an error: it is an
/// `Unsupported` record in the ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepError {
    /// The file name is blank or contains a control character.
    BadFileName,
    /// The timestamp is not `YYYY-MM-DDThh:mm:ss`.
    BadTimestamp,
    /// Two entities share an id.
    DuplicateEntity(EntityId),
    /// The entity is a preview, which is a stand-in and not geometry.
    Preview(EntityId),
    /// The table has no such entity.
    UnknownEntity(EntityId),
}

impl core::fmt::Display for StepError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StepError::BadFileName => {
                write!(f, "the file name must be non-blank and free of control characters")
            }
            StepError::BadTimestamp => write!(f, "the timestamp must be YYYY-MM-DDThh:mm:ss"),
            StepError::DuplicateEntity(id) => write!(f, "entity {} appears twice", id.index()),
            StepError::Preview(id) => {
                write!(f, "entity {} is a preview, and a preview is not exported", id.index())
            }
            StepError::UnknownEntity(id) => write!(f, "the table has no entity {}", id.index()),
        }
    }
}

impl std::error::Error for StepError {}

/// An entity the writer accepts: an exact curve under an id.
///
/// This is the type that keeps tessellated and preview geometry out. It has no constructor from a list of
/// points that is not a [`Polyline`](pvec_geometry::Polyline) the model validated, and the table-aware
/// constructor refuses an entity the table marks as a preview.
#[derive(Clone, Debug, PartialEq)]
pub struct StepEntity {
    id: EntityId,
    curve: Curve,
}

impl StepEntity {
    /// An exact curve under `id`.
    pub fn exact(id: EntityId, curve: Curve) -> StepEntity {
        StepEntity { id, curve }
    }

    /// The entity `id` of `table`, with its decoded `curve`. A table that marks the entity as a preview
    /// refuses it, because the curve in hand would then be a display stand-in.
    pub fn from_table(table: &EntityTable, id: EntityId, curve: Curve) -> Result<StepEntity, StepError> {
        match table.representation(id) {
            Ok(EntityRepresentation::Exact) => Ok(StepEntity { id, curve }),
            Ok(EntityRepresentation::Preview) => Err(StepError::Preview(id)),
            Err(_) => Err(StepError::UnknownEntity(id)),
        }
    }

    /// The entity id.
    pub fn id(&self) -> EntityId {
        self.id
    }

    /// The curve.
    pub fn curve(&self) -> &Curve {
        &self.curve
    }
}

/// What the caller supplies for the header and the coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOptions {
    file_name: String,
    timestamp: String,
    transform: Transform2,
    rasters: Vec<u32>,
}

impl StepOptions {
    /// The file name and the timestamp that go in the header. The timestamp is the caller's and is the
    /// only time the file contains; it must read `YYYY-MM-DDThh:mm:ss`. The transform starts as the
    /// identity.
    pub fn new(file_name: &str, timestamp: &str) -> Result<StepOptions, StepError> {
        if file_name.trim().is_empty() || file_name.chars().any(char::is_control) {
            return Err(StepError::BadFileName);
        }
        if !valid_timestamp(timestamp) {
            return Err(StepError::BadTimestamp);
        }
        Ok(StepOptions {
            file_name: file_name.to_string(),
            timestamp: timestamp.to_string(),
            transform: Transform2::IDENTITY,
            rasters: Vec::new(),
        })
    }

    /// The ids of the rasters the artifact has. AP242 as this export writes it has no image construct, so
    /// the file carries none of them: each gets an `Unsupported` row with a reason in
    /// [`StepExport::rasters`], and the geometry is written exactly as without them.
    pub fn with_rasters(mut self, rasters: Vec<u32>) -> StepOptions {
        self.rasters = rasters;
        self
    }

    /// Applies `transform` to every curve, exactly (`Curve2d::transform`), before it is written. The file
    /// states the matrix in `FILE_DESCRIPTION`.
    pub fn with_transform(mut self, transform: Transform2) -> StepOptions {
        self.transform = transform;
        self
    }

    /// The file name.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// The timestamp.
    pub fn timestamp(&self) -> &str {
        &self.timestamp
    }

    /// The transform.
    pub fn transform(&self) -> &Transform2 {
        &self.transform
    }
}

fn valid_timestamp(t: &str) -> bool {
    let b = t.as_bytes();
    if b.len() != 19 || !t.is_ascii() {
        return false;
    }
    let digits = |r: core::ops::Range<usize>| -> Option<u32> {
        let s = &t[r];
        if s.bytes().all(|c| c.is_ascii_digit()) { s.parse().ok() } else { None }
    };
    let sep_ok = b[4] == b'-' && b[7] == b'-' && b[10] == b'T' && b[13] == b':' && b[16] == b':';
    match (digits(0..4), digits(5..7), digits(8..10), digits(11..13), digits(14..16), digits(17..19)) {
        (Some(_), Some(mo), Some(d), Some(h), Some(mi), Some(s)) => {
            sep_ok && (1..=12).contains(&mo) && (1..=31).contains(&d) && h < 24 && mi < 60 && s < 61
        }
        _ => false,
    }
}

/// What an export produced.
#[derive(Clone, Debug, PartialEq)]
pub struct StepExport {
    /// The file, ASCII, byte-identical for identical input.
    pub part21: String,
    /// One record per entity: the value of the mapping that wrote it, or `Unsupported` with the reason.
    pub ledger: FidelityLedger,
    /// One `Unsupported` row per raster the options named: the file holds no image, and the row says so.
    pub rasters: Vec<RasterRow>,
}

/// Exports `entities`.
///
/// The input type is the guard: only a [`StepEntity`] goes in, and a `StepEntity` is an exact curve.
/// A bare `(EntityId, Curve)` pair, a list of sampled points, or anything a display export produced
/// does not type-check:
///
/// ```compile_fail
/// use pvec_interchange::step::{StepOptions, export};
/// use pvec_geometry::Point2;
/// let sampled: Vec<Point2> = vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let _ = export(&sampled, &options);
/// ```
///
/// ```compile_fail
/// use pvec_interchange::step::{StepOptions, export};
/// use pvec_entity::EntityId;
/// use pvec_geometry::{Curve, Polyline, Point2};
/// let preview = Curve::Polyline(Polyline::new(vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)]).unwrap());
/// let pairs: Vec<(EntityId, Curve)> = vec![(EntityId::new(0).unwrap(), preview)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let _ = export(&pairs, &options);
/// ```
///
/// The same call with the right input type compiles and runs:
///
/// ```
/// use pvec_interchange::step::{StepEntity, StepOptions, export};
/// use pvec_entity::EntityId;
/// use pvec_geometry::{Curve, Polyline, Point2};
/// let line = Curve::Polyline(Polyline::new(vec![Point2::new(0.0, 0.0), Point2::new(1.0, 1.0)]).unwrap());
/// let entities = vec![StepEntity::exact(EntityId::new(0).unwrap(), line)];
/// let options = StepOptions::new("a.stp", "2026-10-24T12:00:00").unwrap();
/// let out = export(&entities, &options).unwrap();
/// assert!(out.part21.starts_with("ISO-10303-21;"));
/// ```
pub fn export(entities: &[StepEntity], options: &StepOptions) -> Result<StepExport, StepError> {
    let mut seen: Vec<u32> = entities.iter().map(|e| e.id.index()).collect();
    seen.sort_unstable();
    if let Some(w) = seen.windows(2).find(|w| w[0] == w[1]) {
        return Err(StepError::DuplicateEntity(EntityId::new(w[0]).expect("an id that was already valid")));
    }
    let registry = Registry::v1();
    let identity = options.transform.matrix() == Transform2::IDENTITY.matrix();

    let mut em = Emitter::default();
    // #1 the plane angle unit (radians, as the profile requires), #2 the 2D representation context.
    let unit = em.add("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )".to_string());
    let context = em.add(format!(
        "( GEOMETRIC_REPRESENTATION_CONTEXT(2) GLOBAL_UNIT_ASSIGNED_CONTEXT((#{unit})) REPRESENTATION_CONTEXT('pvec-2d','2D') )"
    ));

    let mut ledger = FidelityLedger::new();
    let mut tops: Vec<u32> = Vec::new();
    for e in entities {
        let moved;
        let curve = if identity {
            &e.curve
        } else {
            match e.curve.transform(&options.transform) {
                Ok((c, _)) => {
                    moved = c;
                    &moved
                }
                Err(err) => {
                    let why = format!("the transform is refused by the curve model: {}", err.rule());
                    ledger.record(unsupported(e.id, capability_of(&e.curve), &why, &registry));
                    continue;
                }
            }
        };
        let label = format!("e{}", e.id.index());
        let mut mapped = Mapped::default();
        let top = em.curve(curve, &label, &mut mapped);
        tops.push(top);
        ledger.record(mapped.record(e.id, curve, &registry));
    }

    let items: String = tops.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(",");
    let set = em.add(format!("GEOMETRIC_CURVE_SET('pvec',({items}))"));
    em.add(format!("SHAPE_REPRESENTATION('pvec',(#{set}),#{context})"));

    let mut out = String::new();
    out.push_str("ISO-10303-21;\nHEADER;\n");
    let _ = writeln!(
        out,
        "FILE_DESCRIPTION(('PVec 2D semantic geometry, AP242 reference export','{}'),'2;1');",
        string(&transform_text(&options.transform))
    );
    let _ = writeln!(
        out,
        "FILE_NAME('{}','{}',(''),(''),'pvec-interchange {}','','');",
        string(&options.file_name),
        options.timestamp,
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(out, "FILE_SCHEMA(('{SCHEMA_ID}'));");
    out.push_str("ENDSEC;\nDATA;\n");
    for l in &em.lines {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    let rasters = options.rasters.iter().map(|r| RasterRow::left_out(*r, "STEP AP242")).collect();
    Ok(StepExport { part21: out, ledger, rasters })
}

fn transform_text(t: &Transform2) -> String {
    let m = t.matrix();
    if m == Transform2::IDENTITY.matrix() {
        return "transform: identity".to_string();
    }
    format!(
        "transform applied to the geometry, rows [a b tx] and [c d ty]: a={} b={} tx={} c={} d={} ty={}",
        real(m[0]),
        real(m[1]),
        real(m[4]),
        real(m[2]),
        real(m[3]),
        real(m[5])
    )
}

// ---- the instance writer ------------------------------------------------------------------------------

/// What the mapping of one entity found out as it went: the weakest case and a note.
#[derive(Default)]
struct Mapped {
    case: Option<Case>,
    note: String,
}

impl Mapped {
    fn record(self, id: EntityId, curve: &Curve, registry: &Registry) -> FidelityRecord {
        let case = self.case.expect("a written curve has a case");
        let row = row_of(case);
        let fidelity = match row.fidelity {
            FidelityKind::Exact => Fidelity::Exact,
            FidelityKind::ExactReparameterized => Fidelity::ExactReparameterized,
            _ => unreachable!("the STEP table has only exact rows for curves"),
        };
        let target =
            TargetConstruct::new(row.construct, Representation::Native).expect("table names are not blank");
        FidelityRecord::new(id, capability_id(registry, capability_of(curve)), target, fidelity, &self.note)
            .expect("a native exact record is always valid")
    }
}

fn unsupported(id: EntityId, capability: &'static str, why: &str, registry: &Registry) -> FidelityRecord {
    let target = TargetConstruct::new("none", Representation::Native).expect("not blank");
    FidelityRecord::new(
        id,
        capability_id(registry, capability),
        target,
        Fidelity::Unsupported(Reason::new(why).expect("every reason says something")),
        "",
    )
    .expect("an unsupported record is always valid")
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
        // Curve is non-exhaustive; a kind newer than this writer is accounted for under the composite row.
        _ => "composite_curve",
    }
}

#[derive(Default)]
struct Emitter {
    lines: Vec<String>,
}

impl Emitter {
    /// Adds an instance and returns its number. Numbers start at 1 and have no gaps.
    fn add(&mut self, body: String) -> u32 {
        let n = self.lines.len() as u32 + 1;
        self.lines.push(format!("#{n}={body};"));
        n
    }

    fn point(&mut self, p: Point2) -> u32 {
        self.add(format!("CARTESIAN_POINT('',({},{}))", real(p.x), real(p.y)))
    }

    fn direction(&mut self, v: Vector2) -> u32 {
        self.add(format!("DIRECTION('',({},{}))", real(v.x), real(v.y)))
    }

    fn points(&mut self, ps: &[Point2]) -> String {
        let ids: Vec<u32> = ps.iter().map(|p| self.point(*p)).collect();
        list_of_refs(&ids)
    }

    fn placement(&mut self, p: &Placement2) -> u32 {
        let loc = self.point(p.origin());
        let dir = self.direction(p.x_axis());
        self.add(format!("AXIS2_PLACEMENT_2D('',#{loc},#{dir})"))
    }

    /// Writes `c` and everything it refers to; returns the number of the curve itself. `label` is the
    /// name attribute of the curve (the entity's `e<index>` at the top, empty inside).
    fn curve(&mut self, c: &Curve, label: &str, mapped: &mut Mapped) -> u32 {
        let name = string(label);
        match c {
            Curve::Line(l) => {
                let origin = self.point(l.origin());
                let dir = self.direction(l.direction());
                let vec = self.add(format!("VECTOR('',#{dir},1.)"));
                mapped.note_case(Case::Line, "origin and unit direction".to_string());
                self.add(format!("LINE('{name}',#{origin},#{vec})"))
            }
            Curve::Circle(circle) => {
                let pl = self.placement(circle.placement());
                mapped.note_case(Case::Circle, format!("radius {}", real(circle.radius())));
                self.add(format!("CIRCLE('{name}',#{pl},{})", real(circle.radius())))
            }
            Curve::Ellipse(e) => {
                let pl = self.placement(e.placement());
                mapped.note_case(
                    Case::Ellipse,
                    format!("semi-axes {} and {}", real(e.semi_axis_1()), real(e.semi_axis_2())),
                );
                self.add(format!(
                    "ELLIPSE('{name}',#{pl},{},{})",
                    real(e.semi_axis_1()),
                    real(e.semi_axis_2())
                ))
            }
            Curve::Polyline(p) => {
                let pts = self.points(p.vertices());
                mapped.note_case(Case::Polyline, format!("{} vertices", p.vertices().len()));
                self.add(format!("POLYLINE('{name}',{pts})"))
            }
            Curve::Trimmed(t) => {
                let basis = self.curve(t.basis(), "", &mut Mapped::default());
                let tr = t.trim();
                let end = tr.start + tr.span;
                let (t1, t2, sense) =
                    if tr.reversed { (end, tr.start, ".F.") } else { (tr.start, end, ".T.") };
                mapped.note_case(
                    Case::Trimmed,
                    format!("trim domain [{}, {}], end = start + span", real(tr.start), real(end)),
                );
                self.add(format!(
                    "TRIMMED_CURVE('{name}',#{basis},(PARAMETER_VALUE({})),(PARAMETER_VALUE({})),{sense},.PARAMETER.)",
                    real(t1),
                    real(t2)
                ))
            }
            Curve::Composite(comp) => {
                let mut segs = Vec::new();
                let mut trimmed = false;
                for member in comp.segments() {
                    trimmed |= matches!(member, Curve::Trimmed(_));
                    let parent = self.curve(member, "", &mut Mapped::default());
                    segs.push(self.add(format!("COMPOSITE_CURVE_SEGMENT(.CONTINUOUS.,.T.,#{parent})")));
                }
                let case = if trimmed { Case::CompositeTrimmed } else { Case::Composite };
                mapped.note_case(case, format!("{} segments", segs.len()));
                self.add(format!("COMPOSITE_CURVE('{name}',{},.U.)", list_of_refs(&segs)))
            }
            Curve::Bezier(b) => {
                mapped.note_case(Case::Bezier, format!("degree {}", b.degree()));
                self.bezier(b, &name)
            }
            Curve::BSpline(b) => {
                mapped.note_case(
                    Case::BSpline,
                    format!("degree {}, {} control points", b.degree(), b.control_points().len()),
                );
                self.bspline(b, &name)
            }
            Curve::RationalBSpline(r) => {
                mapped.note_case(
                    Case::RationalBSpline,
                    format!("degree {}, {} control points and weights", r.degree(), r.control_points().len()),
                );
                self.rational(r, &name)
            }
            _ => unreachable!("Curve is closed in the v1 profile; a newer kind needs a mapping row first"),
        }
    }

    fn bezier(&mut self, b: &Bezier, name: &str) -> u32 {
        let pts = self.points(b.control_points());
        let d = b.degree();
        let closed = logical(b.is_declared_closed());
        self.add(format!(
            "B_SPLINE_CURVE_WITH_KNOTS('{name}',{d},{pts},.UNSPECIFIED.,{closed},.U.,({m},{m}),(0.,1.),.UNSPECIFIED.)",
            m = d + 1
        ))
    }

    fn bspline(&mut self, b: &BSpline, name: &str) -> u32 {
        let pts = self.points(b.control_points());
        let (mults, values) = knot_lists(b.knots());
        let closed = logical(b.is_declared_closed());
        self.add(format!(
            "B_SPLINE_CURVE_WITH_KNOTS('{name}',{},{pts},.UNSPECIFIED.,{closed},.U.,{mults},{values},.UNSPECIFIED.)",
            b.degree()
        ))
    }

    fn rational(&mut self, r: &RationalBSpline, name: &str) -> u32 {
        let pts = self.points(r.control_points());
        let (mults, values) = knot_lists(r.knots());
        let closed = logical(r.is_declared_closed());
        let weights: Vec<String> = r.weights().iter().map(|w| real(*w)).collect();
        self.add(format!(
            "( BOUNDED_CURVE() B_SPLINE_CURVE({},{pts},.UNSPECIFIED.,{closed},.U.) B_SPLINE_CURVE_WITH_KNOTS({mults},{values},.UNSPECIFIED.) CURVE() GEOMETRIC_REPRESENTATION_ITEM() RATIONAL_B_SPLINE_CURVE(({})) REPRESENTATION_ITEM('{name}') )",
            r.degree(),
            weights.join(",")
        ))
    }
}

impl Mapped {
    fn note_case(&mut self, case: Case, note: String) {
        // Only the outermost curve sets the case: members are written with a throw-away `Mapped`.
        self.case = Some(case);
        self.note = note;
    }
}

fn logical(b: bool) -> &'static str {
    if b { ".T." } else { ".F." }
}

fn list_of_refs(ids: &[u32]) -> String {
    let mut s = String::from("(");
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "#{id}");
    }
    s.push(')');
    s
}

/// Distinct knot values, in order, with their multiplicities, as the two Part 21 lists.
fn knot_lists(knots: &[f64]) -> (String, String) {
    let mut mults = Vec::new();
    let mut values = Vec::new();
    let mut i = 0;
    while i < knots.len() {
        let mut j = i;
        while j < knots.len() && knots[j].to_bits() == knots[i].to_bits() {
            j += 1;
        }
        mults.push((j - i).to_string());
        values.push(real(knots[i]));
        i = j;
    }
    (format!("({})", mults.join(",")), format!("({})", values.join(",")))
}

// ---- lexical forms ----------------------------------------------------------------------------------------

/// A Part 21 REAL: the shortest decimal that reads back to the same `f64`, always with a decimal point,
/// an upper-case `E` for the exponent, and negative zero as `0.`.
fn real(v: f64) -> String {
    if v == 0.0 {
        return "0.".to_string();
    }
    let s = format!("{v:?}");
    match s.split_once('e') {
        None => {
            if s.contains('.') {
                s
            } else {
                format!("{s}.")
            }
        }
        Some((m, e)) => {
            let m = if m.contains('.') { m.to_string() } else { format!("{m}.") };
            format!("{m}E{e}")
        }
    }
}

/// The body of a Part 21 string: apostrophes and backslashes doubled, everything outside printable
/// ASCII as `\X2\hhhh\X0\` (or `\X4\hhhhhhhh\X0\` past the BMP), so the file stays 7-bit.
fn string(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\'' => o.push_str("''"),
            '\\' => o.push_str("\\\\"),
            ' '..='~' => o.push(c),
            c if (c as u32) <= 0xFFFF => {
                let _ = write!(o, "\\X2\\{:04X}\\X0\\", c as u32);
            }
            c => {
                let _ = write!(o, "\\X4\\{:08X}\\X0\\", c as u32);
            }
        }
    }
    o
}

impl StepExport {
    /// The fidelity report of this export (P6.8), for an artifact of `artifact_entities` entities
    /// whose digest is `source_digest`.
    pub fn report(
        &self,
        source_digest: [u8; 32],
        artifact_entities: u32,
    ) -> Result<crate::report::Report, crate::report::ReportError> {
        crate::report::Report::new(
            "STEP AP242",
            "ISO 10303-21:2016",
            source_digest,
            artifact_entities,
            self.ledger.records(),
        )?
        .with_rasters(self.rasters.iter().cloned())
    }
}
