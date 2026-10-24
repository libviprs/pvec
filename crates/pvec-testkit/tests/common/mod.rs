//! Shared helpers: the vendored bundle, and a builder for records the corpus does not have, so a
//! test can hand the generator exactly one defect.
#![allow(dead_code)]

use pvec_corpus::{Bundle, ExpectedMath};

pub fn bundle() -> Bundle {
    Bundle::vendored().expect("the vendored bundle opens")
}

pub const REVISION: &str = "r0-open";

/// `{decimal, f64_hex}` for a value.
pub fn sc(v: f64) -> String {
    format!("{{\"decimal\": \"{:?}\", \"f64_hex\": \"0x{:016x}\"}}", v, v.to_bits())
}

pub fn pt(x: f64, y: f64, z: f64) -> String {
    format!("[{}, {}, {}]", sc(x), sc(y), sc(z))
}

pub fn list(items: &[String]) -> String {
    format!("[{}]", items.join(", "))
}

const CLOSURE: &str = "{\"closed_by_endpoint\": false, \"closed_by_knots\": false, \"periodic\": false}";

fn entity(id: &str, class: &str, geometry: String, closure: &str) -> String {
    format!(
        "{{\"id\": \"{id}\", \"class\": \"{class}\", \"geometry\": {geometry}, \"parameter_domain\": {{\"start\": {}, \"end\": {}}}, \"closure\": {closure}, \"structure\": \"CURVE_ONLY\"}}",
        sc(0.0),
        sc(1.0)
    )
}

pub fn line(id: &str, a: [f64; 3], b: [f64; 3]) -> String {
    entity(
        id,
        "line",
        format!("{{\"start\": {}, \"end\": {}}}", pt(a[0], a[1], a[2]), pt(b[0], b[1], b[2])),
        CLOSURE,
    )
}

pub fn circle(id: &str, c: [f64; 3], normal: [f64; 3], x_axis: [f64; 3], r: f64) -> String {
    entity(
        id,
        "circle",
        format!(
            "{{\"center\": {}, \"normal\": {}, \"x_axis\": {}, \"radius\": {}}}",
            pt(c[0], c[1], c[2]),
            pt(normal[0], normal[1], normal[2]),
            pt(x_axis[0], x_axis[1], x_axis[2]),
            sc(r)
        ),
        CLOSURE,
    )
}

pub fn arc(id: &str, c: [f64; 3], r: f64, start: f64, sweep: f64) -> String {
    entity(
        id,
        "circular_arc",
        format!(
            "{{\"center\": {}, \"normal\": {}, \"x_axis\": {}, \"radius\": {}, \"start_angle\": {}, \"sweep_angle\": {}}}",
            pt(c[0], c[1], c[2]),
            pt(0.0, 0.0, 1.0),
            pt(1.0, 0.0, 0.0),
            sc(r),
            sc(start),
            sc(sweep)
        ),
        CLOSURE,
    )
}

pub fn ellipse(id: &str, c: [f64; 3], major: [f64; 3], a: f64, b: f64) -> String {
    entity(
        id,
        "ellipse",
        format!(
            "{{\"center\": {}, \"normal\": {}, \"major_axis\": {}, \"semi_major\": {}, \"semi_minor\": {}}}",
            pt(c[0], c[1], c[2]),
            pt(0.0, 0.0, 1.0),
            pt(major[0], major[1], major[2]),
            sc(a),
            sc(b)
        ),
        CLOSURE,
    )
}

pub fn polyline(id: &str, vertices: &[[f64; 3]], bulges: &[f64], closed: bool) -> String {
    let v: Vec<String> = vertices.iter().map(|p| pt(p[0], p[1], p[2])).collect();
    let b: Vec<String> = bulges.iter().map(|x| sc(*x)).collect();
    let closure = if closed {
        "{\"closed_by_endpoint\": true, \"closed_by_knots\": false, \"periodic\": false}"
    } else {
        CLOSURE
    };
    entity(id, "polyline", format!("{{\"vertices\": {}, \"bulges\": {}}}", list(&v), list(&b)), closure)
}

/// A B-spline given its distinct knots with multiplicities.
pub fn bspline(
    id: &str,
    degree: u32,
    cps: &[[f64; 3]],
    knots: &[(f64, u32)],
    weights: Option<&[f64]>,
    closure: &str,
) -> String {
    let c: Vec<String> = cps.iter().map(|p| pt(p[0], p[1], p[2])).collect();
    let k: Vec<String> =
        knots.iter().map(|(v, m)| format!("{{\"value\": {}, \"multiplicity\": {m}}}", sc(*v))).collect();
    match weights {
        None => entity(
            id,
            "bspline",
            format!("{{\"degree\": {degree}, \"control_points\": {}, \"knots\": {}}}", list(&c), list(&k)),
            closure,
        ),
        Some(w) => {
            let w: Vec<String> = w.iter().map(|x| sc(*x)).collect();
            entity(
                id,
                "nurbs",
                format!(
                    "{{\"degree\": {degree}, \"control_points\": {}, \"knots\": {}, \"weights\": {}, \"weights_all_unit\": false}}",
                    list(&c),
                    list(&k),
                    list(&w)
                ),
                closure,
            )
        }
    }
}

pub const PERIODIC: &str = "{\"closed_by_endpoint\": false, \"closed_by_knots\": true, \"periodic\": true}";
pub const OPEN: &str = CLOSURE;

/// A whole record with these entities, parsed by the corpus loader like the real ones.
pub fn record(fixture_id: &str, entities: &[String]) -> ExpectedMath {
    let text = format!(
        "{{\"schema_version\": \"pvec-gold/expected.math/1\", \"fixture_id\": \"{fixture_id}\", \"evidence_class\": \"KNOWN_TRUTH_EXACT\", \"units\": {{\"name\": \"metre\", \"canonical\": \"metre\", \"scale_to_canonical\": {}}}, \"provenance\": {{\"derivation\": \"AUTHORING_PARAMETERS\"}}, \"entities\": {}, \"assertions\": [{{\"id\": \"a1\", \"kind\": \"STRUCTURAL\", \"entity\": \"x\", \"claim\": \"CLASS_PRESERVED\", \"statement\": \"x\"}}]}}",
        sc(1.0),
        list(entities)
    );
    ExpectedMath::parse(text.as_bytes()).unwrap_or_else(|e| panic!("the doctored record does not load: {e}\n{text}"))
}

/// A record in another unit.
pub fn record_in_unit(fixture_id: &str, unit: &str, scale: f64, entities: &[String]) -> ExpectedMath {
    let text = format!(
        "{{\"schema_version\": \"pvec-gold/expected.math/1\", \"fixture_id\": \"{fixture_id}\", \"evidence_class\": \"KNOWN_TRUTH_EXACT\", \"units\": {{\"name\": \"{unit}\", \"canonical\": \"metre\", \"scale_to_canonical\": {}}}, \"provenance\": {{\"derivation\": \"AUTHORING_PARAMETERS\"}}, \"entities\": {}, \"assertions\": [{{\"id\": \"a1\", \"kind\": \"STRUCTURAL\", \"entity\": \"x\", \"claim\": \"CLASS_PRESERVED\", \"statement\": \"x\"}}]}}",
        sc(scale),
        list(entities)
    );
    ExpectedMath::parse(text.as_bytes()).expect("the doctored record loads")
}

/// The record's entities as a rotated copy, for order tests: a fixed shuffle.
pub fn permutations(n: usize) -> Vec<Vec<usize>> {
    let id: Vec<usize> = (0..n).collect();
    let rev: Vec<usize> = (0..n).rev().collect();
    let rot: Vec<usize> = (0..n).map(|i| (i + n / 2 + 1) % n).collect();
    let evens_odds: Vec<usize> = (0..n).step_by(2).chain((1..n).step_by(2)).collect();
    vec![id, rev, rot, evens_odds]
}
