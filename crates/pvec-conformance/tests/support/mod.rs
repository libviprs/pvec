//! Fixtures and a way to run the binary, for the tests of `pvec-conformance`.
#![allow(dead_code)]

pub mod json;

use pvec::{Bounds2, CapabilityId, Curve, EntityId, Manifest, PvecWriter};
use pvec_container::directory::parse_directory;
use pvec_container::integrity::Integrity;
use pvec_container::writer::{RawBlock, assemble};
use pvec_geometry::{BSpline, Circle, Curve2d, Placement2, Point2, Side, Vector2};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;

pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("pvec-conformance");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

pub fn file(name: &str, bytes: &[u8]) -> String {
    // written beside it and renamed, so a test that reads the file while another writes it never sees half
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let p = scratch(name);
    let tmp = scratch(&format!(
        "{name}.{}.{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&tmp, bytes).unwrap();
    std::fs::rename(&tmp, &p).unwrap();
    p.to_string_lossy().into_owned()
}

pub struct Run {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Run {
    pub fn json(&self) -> json::V {
        json::parse(&self.stdout).unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {}", self.stdout))
    }
}

/// Runs the binary with an empty environment: no PATH, so nothing it needs can be found outside it.
pub fn pvec(args: &[&str]) -> Run {
    let out = Command::new(env!("CARGO_BIN_EXE_pvec"))
        .args(args)
        .env_clear()
        .current_dir(scratch(""))
        .output()
        .expect("the binary runs");
    Run {
        code: out.status.code().expect("exited with a code"),
        stdout: String::from_utf8(out.stdout).unwrap(),
        stderr: String::from_utf8(out.stderr).unwrap(),
    }
}

pub fn schema() -> json::V {
    let p = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli-report.schema.json");
    json::parse(&std::fs::read_to_string(p).expect("docs/cli-report.schema.json is checked in")).unwrap()
}

pub fn pt(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn manifest(caps: &[u32]) -> Manifest {
    Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [-20.0, -20.0], max: [20.0, 20.0] },
        capabilities: caps.iter().map(|c| CapabilityId::new(*c)).collect(),
        producer_id: "pvec-conformance-tests".into(),
        producer_version: "0".into(),
    }
}

/// A circle and a cubic B-spline: the curves a comparison with an evaluator is made over.
pub fn curves() -> Vec<(u32, Curve)> {
    let circle = Curve::Circle(
        Circle::new(Placement2::new(pt(5.0, 5.0), Vector2::new(1.0, 0.0)).unwrap(), 4.0).unwrap(),
    );
    let cps = vec![pt(0.0, 0.0), pt(1.0, 3.0), pt(3.0, 4.0), pt(5.0, 1.0), pt(8.0, 2.0), pt(9.0, 6.0)];
    let knots = vec![0.0, 0.0, 0.0, 0.0, 0.25, 0.75, 1.0, 1.0, 1.0, 1.0];
    vec![(1, circle), (2, Curve::BSpline(BSpline::new(3, cps, knots).unwrap()))]
}

pub fn write(caps: &[u32], items: &[(u32, Curve)]) -> Vec<u8> {
    let mut w = PvecWriter::new(manifest(caps)).unwrap();
    for (id, c) in items {
        w.push(EntityId::new(*id).unwrap(), c).unwrap();
    }
    w.finish().unwrap()
}

/// A valid file with a polyline, a circle and a spline, sealed with checksums and a digest.
pub fn good_file() -> Vec<u8> {
    let mut items = curves();
    items.push((
        3,
        Curve::Polyline(
            pvec_geometry::Polyline::new(vec![pt(0.0, 0.0), pt(4.0, 3.0), pt(8.0, 3.0)]).unwrap(),
        ),
    ));
    write(&[5, 6, 11], &items)
}

/// The file `curves()` makes, for the comparison tests.
pub fn compare_file() -> Vec<u8> {
    write(&[6, 11], &curves())
}

/// `bytes` laid out again after `edit` changed its manifest or its blocks, unsealed.
pub fn rebuild(
    bytes: &[u8],
    edit: impl FnOnce(&mut pvec_container::Manifest, &mut Vec<RawBlock>),
) -> Vec<u8> {
    let len = bytes.len() as u64;
    let header = pvec_container::Header::parse(&bytes[..bytes.len().min(4096)], len).unwrap();
    let (m0, m1) =
        (header.manifest_offset as usize, (header.manifest_offset + header.manifest_length) as usize);
    let mut manifest = pvec_container::Manifest::parse(&bytes[m0..m1], &header, len).unwrap();
    let d = manifest.directories[0];
    let entries = parse_directory(
        &bytes[d.offset as usize..(d.offset + d.length) as usize],
        u64::from(header.header_len),
        len,
    )
    .unwrap();
    let mut blocks: Vec<RawBlock> = entries
        .iter()
        .map(|e| RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags,
            codec: e.codec,
            capability: e.capability,
            uncompressed_length: e.uncompressed_length,
            stored: bytes[e.offset as usize..e.offset as usize + e.stored_length as usize].to_vec(),
        })
        .collect();
    edit(&mut manifest, &mut blocks);
    assemble(manifest, &blocks, Integrity::Full).unwrap()
}

fn f(x: f64) -> String {
    format!("{x:?}")
}

fn p(q: Point2) -> String {
    format!("{},{}", f(q.x), f(q.y))
}

/// What an evaluator would print for `curves` in order if it agreed with PVec, one `EDGE` per curve,
/// with the parameter range of the spline stretched by `stretch` (1.0 is no change) so that the file
/// is the same locus under another parameterisation. This is the first pass: edges, no samples.
pub fn evaluator_report(curves: &[Curve], stretch: f64) -> String {
    let edges: Vec<String> = curves
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let d = c.domain();
            let k = if matches!(c, Curve::BSpline(_)) { stretch } else { 1.0 };
            let mut head = format!(
                "first={} last={} start={} end={} z=0.0,0.0",
                f(d.lo() * k),
                f(d.hi() * k),
                p(c.eval(d.lo()).unwrap()),
                p(c.eval(d.hi()).unwrap())
            );
            let kind = match c {
                Curve::Circle(c) => {
                    let _ = write!(
                        head,
                        " length={:?} center={} radius={} xdir={}",
                        std::f64::consts::TAU * c.radius(),
                        p(c.placement().origin()),
                        f(c.radius()),
                        p(Point2::new(c.placement().x_axis().x, c.placement().x_axis().y))
                    );
                    "Circle"
                }
                Curve::BSpline(b) => {
                    let (mut mults, mut values): (Vec<usize>, Vec<f64>) = (Vec::new(), Vec::new());
                    for k2 in b.knots() {
                        if values.last() == Some(&(k2 * k)) {
                            *mults.last_mut().unwrap() += 1;
                        } else {
                            values.push(k2 * k);
                            mults.push(1);
                        }
                    }
                    let _ = write!(
                        head,
                        " length=0.0 degree={} poles={} knots={} mults={} weights={} periodic=0.0",
                        b.degree(),
                        b.control_points().iter().map(|q| p(*q)).collect::<Vec<_>>().join(","),
                        values.iter().map(|v| f(*v)).collect::<Vec<_>>().join(","),
                        mults.iter().map(|m| format!("{m}.0")).collect::<Vec<_>>().join(","),
                        b.control_points().iter().map(|_| "1.0").collect::<Vec<_>>().join(",")
                    );
                    "BSplineCurve"
                }
                _ => panic!("the mirror knows circles and splines"),
            };
            format!("EDGE {i} {kind} {head}")
        })
        .collect();
    format!("EVALUATOR freecad=0.20.2 occt=7.6.3 edges={}\n{}\n", edges.len(), edges.join("\n"))
}

/// The second evaluator pass: the first pass plus a `SAMPLE` for every parameter in `requests` (the JSON
/// `pvec compare-step --requests` prints), answered from PVec's own curve at the same fraction of the
/// edge's range, moved up by `bend`.
pub fn answer(first: &str, requests: &str, curves: &[Curve], stretch: f64, bend: f64) -> String {
    let doc = json::parse(requests).expect("the requests are JSON");
    let mut out = first.to_string();
    for (i, c) in curves.iter().enumerate() {
        let Some(us) = (match &doc {
            json::V::Obj(m) => m.get(&i.to_string()),
            _ => None,
        }) else {
            continue;
        };
        let d = c.domain();
        let k = if matches!(c, Curve::BSpline(_)) { stretch } else { 1.0 };
        for u in us.arr() {
            let u = u.num();
            let t = u / k;
            let side = if t >= d.hi() { Side::Left } else { Side::Right };
            let q = c.eval(t).unwrap();
            let tan = c.derivative(t, 1, side).unwrap();
            let n = (tan.x * tan.x + tan.y * tan.y).sqrt();
            let _ =
                writeln!(out, "SAMPLE {i} {u:?} {:?},{:?},{:?},{:?}", q.x, q.y + bend, tan.x / n, tan.y / n);
        }
    }
    out
}
