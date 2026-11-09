//! The capability-to-AP242 mapping table.
//!
//! One row per way a PVec capability is written. The writer looks its fidelity value and its Part 21
//! entity up here (`row_of`), and `docs/INTERCHANGE.md` chapter 1 carries [`mapping_table_markdown`]
//! byte for byte (a test compares them), so the table, the code and the document are one thing.
//!
//! The entity names are the ones `docs/ISO_PROFILE.md` gives for each capability's exact Part 21 form.

use crate::{FidelityKind, Representation};
use core::fmt::Write as _;

/// A way a PVec capability is mapped. Each variant is one row of the table in `docs/INTERCHANGE.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Case {
    /// A point, as the origin, vertex or control point of a curve.
    Point,
    /// A direction or vector, as the axis of a placement or the direction of a line.
    Direction,
    /// An axis placement, as the frame of a circle or ellipse.
    Placement,
    /// An unbounded line.
    Line,
    /// A polyline.
    Polyline,
    /// A circle.
    Circle,
    /// An ellipse.
    Ellipse,
    /// A trimmed line, circle, ellipse or spline.
    Trimmed,
    /// A composite curve whose segments are all carried unchanged.
    Composite,
    /// A composite curve with a trimmed segment.
    CompositeTrimmed,
    /// A Bezier curve.
    Bezier,
    /// A B-spline.
    BSpline,
    /// A rational B-spline.
    RationalBSpline,
    /// The affine transform.
    Transform,
    /// A component stream.
    Component,
}

/// One row of the mapping table: a capability, the case, the entity and the one value it carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MappingRow {
    /// The capability's registry name.
    pub capability: &'static str,
    /// The case within that capability.
    pub case: Case,
    /// The Part 21 entity written, or `none`.
    pub construct: &'static str,
    /// Native or tessellated. The STEP export has no tessellated row, and a test says so.
    pub representation: Representation,
    /// The fidelity value this case carries.
    pub fidelity: FidelityKind,
    /// For `Unsupported`, the reason. For the others, how the value is obtained.
    pub note: &'static str,
}

macro_rules! row {
    ($cap:expr, $case:ident, $construct:expr, $fid:ident, $note:expr) => {
        MappingRow {
            capability: $cap,
            case: Case::$case,
            construct: $construct,
            representation: Representation::Native,
            fidelity: FidelityKind::$fid,
            note: $note,
        }
    };
}

const COMPONENT_REASON: &str = "a component stream is not geometry; the AP242 reference export writes the 2D curve profile only, and presentation, layers and product structure are outside it";

/// The mapping table. Every capability of the v1 registry has at least one row, and every row carries
/// exactly one fidelity value.
pub const MAPPING: &[MappingRow] = &[
    row!(
        "cartesian_point",
        Point,
        "CARTESIAN_POINT",
        Exact,
        "two coordinates, each the shortest decimal that reads back to the same f64"
    ),
    row!(
        "direction_vector",
        Direction,
        "DIRECTION, VECTOR",
        Exact,
        "the stored unit direction; a line's vector has magnitude 1 so the parameter stays arc length"
    ),
    row!(
        "axis_placement_2d",
        Placement,
        "AXIS2_PLACEMENT_2D",
        Exact,
        "location and stored first axis; the reference direction is always written"
    ),
    row!(
        "line",
        Line,
        "LINE",
        Exact,
        "origin and unit direction carried; the unbounded carrier is a valid STEP curve"
    ),
    row!(
        "polyline",
        Polyline,
        "POLYLINE",
        Exact,
        "every vertex carried in order; a closed polyline repeats its first vertex, as in the container"
    ),
    row!("circle", Circle, "CIRCLE", Exact, "placement and radius carried"),
    row!("ellipse", Ellipse, "ELLIPSE", Exact, "placement and both semi-axes carried, not reordered"),
    row!(
        "trimmed_curve",
        Trimmed,
        "TRIMMED_CURVE",
        ExactReparameterized,
        "the stored (start, span, reversed) becomes the trim pair (start, start + span) with sense .T., or (start + span, start) with sense .F.; basis and start are carried bit for bit and the end is one addition"
    ),
    row!(
        "composite_curve",
        Composite,
        "COMPOSITE_CURVE",
        Exact,
        "one COMPOSITE_CURVE_SEGMENT per member in order, parents carried unchanged, .CONTINUOUS. transitions, self_intersect .U."
    ),
    row!(
        "composite_curve",
        CompositeTrimmed,
        "COMPOSITE_CURVE",
        ExactReparameterized,
        "as above, and at least one segment is a trimmed curve, so the entity takes the value of its weakest member"
    ),
    row!(
        "bezier_curve",
        Bezier,
        "B_SPLINE_CURVE_WITH_KNOTS",
        Exact,
        "degree and control points carried; the clamped single span is the explicit knots (0, 1) with multiplicity degree + 1"
    ),
    row!(
        "b_spline_curve",
        BSpline,
        "B_SPLINE_CURVE_WITH_KNOTS",
        Exact,
        "degree, control points, knot values and multiplicities and the declared-closed flag carried"
    ),
    row!(
        "rational_b_spline_curve",
        RationalBSpline,
        "B_SPLINE_CURVE_WITH_KNOTS, RATIONAL_B_SPLINE_CURVE",
        Exact,
        "a complex entity: the B-spline of the row above plus one weight per control point; equal weights stay rational"
    ),
    row!(
        "affine_transform_2d",
        Transform,
        "none",
        Unsupported,
        "the only 2D operator in Part 42 carries one scale factor and is an instancing mechanism the profile leaves out; the map is applied to the geometry exactly (a circle becomes an ellipse where the promotion table says so) and the matrix is stated in FILE_DESCRIPTION"
    ),
    row!("layer", Component, "none", Unsupported, COMPONENT_REASON),
    row!("line_style", Component, "none", Unsupported, COMPONENT_REASON),
    row!("text", Component, "none", Unsupported, COMPONENT_REASON),
    row!("hatch", Component, "none", Unsupported, COMPONENT_REASON),
    row!("source_provenance", Component, "none", Unsupported, COMPONENT_REASON),
    row!("dwg_semantics", Component, "none", Unsupported, COMPONENT_REASON),
    row!("semantic_class", Component, "none", Unsupported, COMPONENT_REASON),
    row!("paint_order", Component, "none", Unsupported, COMPONENT_REASON),
    row!("stroke_paint", Component, "none", Unsupported, COMPONENT_REASON),
    row!("fill_paint", Component, "none", Unsupported, COMPONENT_REASON),
    row!("clip", Component, "none", Unsupported, COMPONENT_REASON),
    row!(
        "raster",
        Component,
        "none",
        Unsupported,
        "a raster is pixels, not geometry; the AP242 reference export writes the 2D curve profile only and records the raster as Unsupported (ADR 0018)"
    ),
];

/// The row for a geometry case that the writer emits, which is unique per case.
pub(crate) fn row_of(case: Case) -> &'static MappingRow {
    MAPPING.iter().find(|r| r.case == case).expect("every emitted case has a row")
}

/// The table as the markdown `docs/INTERCHANGE.md` carries, so the document and the code cannot drift.
pub fn mapping_table_markdown() -> String {
    let mut s =
        String::from("| capability | case | Part 21 entity | fidelity | how |\n|---|---|---|---|---|\n");
    for r in MAPPING {
        let _ = writeln!(
            s,
            "| `{}` | {:?} | `{}` | `{}` | {} |",
            r.capability,
            r.case,
            r.construct,
            r.fidelity.name(),
            r.note
        );
    }
    s
}
