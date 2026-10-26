//! The STEP round trip over the corpus, wired into the frozen gates (issue P6.3b, pvec#150).
//!
//! `curves_of` and `transform_of` read a fixture's authored mathematics (`expected.math.json`) as PVec curves and
//! the map into the world frame, so the STEP export can be written from them and read back by the independent
//! evaluator (`pvec_interchange::step::verify`). `Runner::step_roundtrip` then records the comparison under the
//! structural and the geometric gate, with the fixture's own evidence class. Nothing here writes a tolerance: the
//! comparison takes them from `pvec_numeric::Policy::V1` and the gates only count what it found.

use crate::error::Result;
use crate::gates::{Gate, GateVerdict, Runner};
use pvec_capabilities::EvidenceClass;
use pvec_corpus::expected::{Entity, EntityClass, Node};
use pvec_corpus::ExpectedMath;
use pvec_entity::EntityId;
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline,
    Transform2, Trimmed, Vector2,
};
use pvec_interchange::step::verify::Verdict;

fn scalar(node: Option<&Node>, what: &str) -> std::result::Result<f64, String> {
    node.and_then(Node::as_scalar).map(|s| s.value()).ok_or_else(|| format!("{what} is not a scalar"))
}

fn list<'a>(node: Option<&'a Node>, what: &str) -> std::result::Result<&'a [Node], String> {
    node.and_then(Node::as_list).ok_or_else(|| format!("{what} is not a list"))
}

fn point(node: Option<&Node>, what: &str) -> std::result::Result<Point2, String> {
    let c = list(node, what)?;
    Ok(Point2::new(scalar(c.first(), what)?, scalar(c.get(1), what)?))
}

fn planar(node: Option<&Node>, what: &str) -> std::result::Result<(), String> {
    match list(node, what)?.get(2) {
        Some(z) if scalar(Some(z), what)? == 0.0 => Ok(()),
        _ => Err(format!("{what} leaves the plane z = 0")),
    }
}

fn points(node: Option<&Node>, what: &str) -> std::result::Result<Vec<Point2>, String> {
    let mut out = Vec::new();
    for (i, p) in list(node, what)?.iter().enumerate() {
        planar(Some(p), &format!("{what}[{i}]"))?;
        out.push(point(Some(p), &format!("{what}[{i}]"))?);
    }
    Ok(out)
}

fn placement(g: &Node, axis: &str) -> std::result::Result<Placement2, String> {
    let normal = list(g.get("normal"), "normal")?;
    if scalar(normal.get(2), "normal")? != 1.0 {
        return Err("a normal other than +z needs the 3D model".to_string());
    }
    planar(g.get("center"), "center")?;
    let a = list(g.get(axis), axis)?;
    let origin = point(g.get("center"), "center")?;
    let dir = Vector2::new(scalar(a.first(), axis)?, scalar(a.get(1), axis)?);
    Placement2::new(origin, dir).map_err(|e| e.to_string())
}

/// A conic arc: the basis trimmed from `start` through `sweep`, counter-clockwise for a positive sweep.
fn arc(basis: Curve, g: &Node) -> std::result::Result<Curve, String> {
    let start = scalar(g.get("start_angle"), "start_angle")?;
    let sweep = scalar(g.get("sweep_angle"), "sweep_angle")?;
    Trimmed::new(basis, start, start + sweep, sweep > 0.0).map(Curve::Trimmed).map_err(|e| e.to_string())
}

fn spline(e: &Entity, g: &Node) -> std::result::Result<Curve, String> {
    let degree = match g.get("degree") {
        Some(Node::Int(d)) if *d >= 1 => *d as usize,
        _ => return Err("degree is missing".to_string()),
    };
    let cps = points(g.get("control_points"), "control_points")?;
    if e.class == EntityClass::Bezier {
        return Bezier::new(cps).map(Curve::Bezier).map_err(|e| e.to_string());
    }
    let mut knots = Vec::new();
    for (i, k) in list(g.get("knots"), "knots")?.iter().enumerate() {
        let v = scalar(k.get("value"), &format!("knots[{i}].value"))?;
        match k.get("multiplicity") {
            Some(Node::Int(m)) if *m >= 1 => knots.extend(std::iter::repeat_n(v, *m as usize)),
            _ => return Err(format!("knots[{i}].multiplicity is missing")),
        }
    }
    let periodic = matches!(e.node.get("closure").and_then(|c| c.get("periodic")), Some(Node::Bool(true)));
    if periodic && e.class == EntityClass::Bspline {
        // the record keeps the wrapped form (one knot per control point and one more); the exchange form is the
        // clamped one, which `from_periodic` makes with the same locus and domain
        return BSpline::from_periodic(degree, cps, knots).map(|u| Curve::BSpline(u.curve)).map_err(|e| e.to_string());
    }
    let curve = if e.class == EntityClass::Nurbs {
        let w = list(g.get("weights"), "weights")?
            .iter()
            .enumerate()
            .map(|(i, w)| scalar(Some(w), &format!("weights[{i}]")))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        RationalBSpline::new(degree, cps, knots, w).map(Curve::RationalBSpline)
    } else {
        BSpline::new(degree, cps, knots).map(Curve::BSpline)
    };
    curve.map_err(|e| e.to_string())
}

/// A polyline with bulges: runs of straight segments stay one polyline, each bulged segment is the arc through
/// it, and several pieces join as a composite.
fn bulged(e: &Entity, g: &Node) -> std::result::Result<Curve, String> {
    let v = points(g.get("vertices"), "vertices")?;
    let b = list(g.get("bulges"), "bulges")?
        .iter()
        .enumerate()
        .map(|(i, x)| scalar(Some(x), &format!("bulges[{i}]")))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if b.len() != v.len() {
        return Err("a bulge for each vertex is expected".to_string());
    }
    let closed = matches!(e.node.get("closure").and_then(|c| c.get("closed_by_endpoint")), Some(Node::Bool(true)));
    let segments = if closed { v.len() } else { v.len() - 1 };
    let mut pieces: Vec<Curve> = Vec::new();
    let mut run: Vec<Point2> = Vec::new();
    for i in 0..segments {
        let (p0, p1) = (v[i], v[(i + 1) % v.len()]);
        if b[i] == 0.0 {
            if run.is_empty() {
                run.push(p0);
            }
            run.push(p1);
            continue;
        }
        if !run.is_empty() {
            pieces.push(Curve::Polyline(Polyline::new(std::mem::take(&mut run)).map_err(|e| e.to_string())?));
        }
        pieces.push(Curve::Trimmed(Trimmed::from_bulge(p0, p1, b[i]).map_err(|e| e.to_string())?));
    }
    if !run.is_empty() {
        pieces.push(Curve::Polyline(Polyline::new(run).map_err(|e| e.to_string())?));
    }
    if pieces.len() == 1 {
        return Ok(pieces.remove(0));
    }
    let composite = Composite::new(pieces).map_err(|e| e.to_string())?;
    if closed {
        return composite.declare_closed().map(Curve::Composite).map_err(|e| e.to_string());
    }
    Ok(Curve::Composite(composite))
}

fn curve_of(e: &Entity) -> std::result::Result<Curve, String> {
    let g = &e.geometry;
    let closed = matches!(
        e.node.get("closure"),
        Some(c) if matches!(c.get("closed_by_knots"), Some(Node::Bool(true)))
            || matches!(c.get("closed_by_endpoint"), Some(Node::Bool(true)))
    );
    match e.class {
        EntityClass::Line => {
            planar(g.get("start"), "start")?;
            planar(g.get("end"), "end")?;
            let (a, b) = (point(g.get("start"), "start")?, point(g.get("end"), "end")?);
            let line = Line::new(a, Vector2::new(b.x - a.x, b.y - a.y)).map_err(|e| e.to_string())?;
            let len = (b.x - a.x).hypot(b.y - a.y);
            Trimmed::new(Curve::Line(line), 0.0, len, true).map(Curve::Trimmed).map_err(|e| e.to_string())
        }
        EntityClass::Polyline => bulged(e, g),
        EntityClass::Circle | EntityClass::CircularArc => {
            let r = scalar(g.get("radius"), "radius")?;
            let circle = Circle::new(placement(g, "x_axis")?, r).map(Curve::Circle).map_err(|e| e.to_string())?;
            if e.class == EntityClass::Circle { Ok(circle) } else { arc(circle, g) }
        }
        EntityClass::Ellipse | EntityClass::EllipticalArc => {
            let (a, b) = (scalar(g.get("semi_major"), "semi_major")?, scalar(g.get("semi_minor"), "semi_minor")?);
            let ellipse =
                Ellipse::new(placement(g, "major_axis")?, a, b).map(Curve::Ellipse).map_err(|e| e.to_string())?;
            if e.class == EntityClass::Ellipse { Ok(ellipse) } else { arc(ellipse, g) }
        }
        EntityClass::Bezier | EntityClass::Bspline | EntityClass::Nurbs => {
            let curve = spline(e, g)?;
            match (&curve, closed) {
                (Curve::BSpline(b), true) => {
                    b.clone().declare_closed().map(Curve::BSpline).map_err(|e| e.to_string())
                }
                _ => Ok(curve),
            }
        }
    }
}

/// The curves a fixture's authored mathematics describes in its local frame, one per entity, numbered from 1.
/// An entity outside the plane z = 0 or with a normal other than +z is refused: PVec's curves are planar.
pub fn curves_of(expected: &ExpectedMath) -> std::result::Result<Vec<(EntityId, Curve)>, String> {
    let mut out = Vec::new();
    for (i, e) in expected.entities.iter().enumerate() {
        let id = EntityId::new(i as u32 + 1).map_err(|e| format!("{e:?}"))?;
        out.push((id, curve_of(e).map_err(|why| format!("entity `{}`: {why}", e.id))?));
    }
    Ok(out)
}

fn matrix(node: &Node) -> std::result::Result<Transform2, String> {
    let m = list(node.get("matrix"), "matrix")?
        .iter()
        .enumerate()
        .map(|(i, x)| scalar(Some(x), &format!("matrix[{i}]")))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if m.len() != 16 {
        return Err(format!("a 4x4 matrix has 16 entries, not {}", m.len()));
    }
    // row-major, column vectors: the plane keeps rows 0 and 1 and the x, y and translation columns
    Transform2::new([m[0], m[1], m[4], m[5], m[3], m[7]]).map_err(|e| e.to_string())
}

/// The map from a fixture's local frame to its world frame: the `composed` matrix of a `transform_chain`, else the
/// `transform` matrix, else the identity. Every entity of a fixture has to agree.
pub fn transform_of(expected: &ExpectedMath) -> std::result::Result<Transform2, String> {
    let mut found: Option<Transform2> = None;
    for e in &expected.entities {
        let t = match (e.node.get("transform_chain"), e.node.get("transform")) {
            (Some(chain), _) => matrix(chain.get("composed").ok_or("transform_chain.composed is missing")?)?,
            (None, Some(t)) => matrix(t)?,
            (None, None) => Transform2::IDENTITY,
        };
        match found {
            Some(prev) if prev != t => return Err("the entities of one fixture sit under different maps".to_string()),
            _ => found = Some(t),
        }
    }
    found.ok_or_else(|| "a record has at least one entity".to_string())
}

impl Runner {
    /// Records a STEP round trip verdict under the structural and the geometric gate, both with the fixture's
    /// `class`. A fixture whose class cannot satisfy those gates is refused and leaves no row.
    ///
    /// Structural: a disagreement fails it; a verdict that compared no parameter because none was directly
    /// comparable is routed to the geometric gate; one that compared nothing at all fails, since silence is not
    /// agreement. Geometric: a disagreement or an empty comparison fails it. Display results are not recorded
    /// here: the round trip of an exact fixture has none, and an approximation is never an exactness row.
    pub fn step_roundtrip(&mut self, fixture: &str, class: EvidenceClass, verdict: &Verdict) -> Result<()> {
        let structural = if !verdict.structural.is_empty() {
            GateVerdict::Fail
        } else if verdict.structural_checked > 0 {
            GateVerdict::Pass
        } else if !verdict.structural_not_comparable.is_empty() {
            GateVerdict::Routed
        } else {
            GateVerdict::Fail
        };
        let geometric = if verdict.geometric.is_empty() && verdict.geometric_checked > 0 {
            GateVerdict::Pass
        } else {
            GateVerdict::Fail
        };
        let n = |x: usize| x as f64;
        self.claim(
            Gate::StructuralExact,
            fixture,
            class,
            structural,
            &[
                ("parameters", n(verdict.structural_checked)),
                ("failing_fields", n(verdict.structural.len())),
                ("not_comparable", n(verdict.structural_not_comparable.len())),
            ],
        )?;
        self.claim(
            Gate::GeometricExact,
            fixture,
            class,
            geometric,
            &[
                ("samples", n(verdict.geometric_checked)),
                ("deviations", n(verdict.geometric.len())),
                ("exact_passes", n(verdict.exact_passes)),
            ],
        )
    }
}
