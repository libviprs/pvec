//! Exact evaluation and adaptive tessellation by curve type, transform and error budget (issue P8.4).
//!
//! ```text
//! geometry run PLATFORM HOST [PROTOTYPE_KV]   the rows of docs/bench/geometry.csv, no header, on stdout
//! geometry header                             the header
//! geometry report CSV                         the verdict lines the document records
//! ```
//!
//! `PROTOTYPE_KV` is the output of `spikes/geometry-api`'s `corpus_eval`, the P1.3 baseline on the same curve.
//! A global allocator that counts allocation calls sits in this file because a global allocator needs `unsafe`
//! and the library crates deny it; it forwards every call to the system allocator.
#![allow(unsafe_code)]

use pvec_bench::Spec;
use pvec_bench::geometry_bench::{HEADER, Row, parse_rows, verdicts};
use pvec_corpus::json::{self, Value};
use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Curve2d, Ellipse, Line, Placement2, Point2, Polyline,
    RationalBSpline, Side, Transform2, Trimmed, Vector2,
};
use pvec_interchange::mvt::tessellate;
use pvec_numeric::Policy;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

struct Counting;

// SAFETY: every call is forwarded unchanged to the system allocator; the only addition is a relaxed counter.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const OFFSET: [f64; 2] = [6_480_000.125, 5_900_000.25];
const VARIANTS: usize = 64;
const PARAMS: usize = 256;
const BUDGETS: [f64; 4] = [0.1, 0.25, 0.5, 1.0];
const ARMS: [&str; 5] = ["identity", "translation", "uniform-scale", "rotation-scale", "chain-offset"];
const CURVES: [&str; 10] = [
    "line",
    "polyline",
    "circle",
    "ellipse",
    "trimmed",
    "composite",
    "bezier",
    "bspline3",
    "bspline7",
    "nurbs",
];

fn lcg(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (*seed >> 11) as f64 / (1u64 << 53) as f64
}

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn place(x: f64, y: f64) -> Placement2 {
    Placement2::new(p(x, y), Vector2::new(1.0, 0.0)).expect("a frame")
}

/// The base curve of each type, in a drawing of a few hundred units.
fn base(kind: &str) -> Curve {
    let spline = |degree: usize, ncp: usize| {
        let cps: Vec<Point2> = (0..ncp).map(|i| p(10.0 * i as f64, 3.0 * ((37 * i) % 11) as f64)).collect();
        let spans = ncp - degree;
        let mut knots = vec![0.0; degree + 1];
        knots.extend((1..spans).map(|k| k as f64 / spans as f64));
        knots.extend(vec![1.0; degree + 1]);
        (cps, knots)
    };
    let circle = || Curve::Circle(Circle::new(place(0.0, 0.0), 100.0).expect("circle"));
    match kind {
        "line" => Curve::Trimmed(
            Trimmed::new(
                Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(3.0, 4.0)).expect("line")),
                0.0,
                50.0,
                true,
            )
            .expect("trim"),
        ),
        "polyline" => Curve::Polyline(
            Polyline::new((0..16).map(|i| p(10.0 * i as f64, 8.0 * ((i * 5) % 7) as f64)).collect())
                .expect("polyline"),
        ),
        "circle" => circle(),
        "ellipse" => Curve::Ellipse(Ellipse::new(place(0.0, 0.0), 120.0, 45.0).expect("ellipse")),
        "trimmed" => Curve::Trimmed(Trimmed::new(circle(), 0.3, 2.9, true).expect("trim")),
        "composite" => {
            let a = Trimmed::new(
                Curve::Line(Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).expect("line")),
                0.0,
                20.0,
                true,
            )
            .expect("trim");
            let c = Circle::new(place(20.0, 10.0), 10.0).expect("circle");
            let half = core::f64::consts::FRAC_PI_2;
            let b = Trimmed::new(Curve::Circle(c), -half, half, true).expect("trim");
            let z = Bezier::new(vec![p(20.0, 20.0), p(10.0, 30.0), p(0.0, 30.0), p(-10.0, 20.0)])
                .expect("bezier");
            Curve::Composite(
                Composite::new(vec![Curve::Trimmed(a), Curve::Trimmed(b), Curve::Bezier(z)])
                    .expect("composite"),
            )
        }
        "bezier" => Curve::Bezier(
            Bezier::new(vec![p(0.0, 0.0), p(10.0, 30.0), p(40.0, 30.0), p(50.0, 0.0)]).expect("bezier"),
        ),
        "bspline3" => {
            let (cps, knots) = spline(3, 12);
            Curve::BSpline(BSpline::new(3, cps, knots).expect("spline"))
        }
        "bspline7" => {
            let (cps, knots) = spline(7, 20);
            Curve::BSpline(BSpline::new(7, cps, knots).expect("spline"))
        }
        "nurbs" => {
            let (cps, knots) = spline(3, 10);
            let w = (0..10).map(|i| 0.5 + 0.25 * (i % 7) as f64).collect();
            Curve::RationalBSpline(RationalBSpline::new(3, cps, knots, w).expect("nurbs"))
        }
        other => panic!("no curve {other}"),
    }
}

/// `VARIANTS` exact images of the base: scaled 1 to 4 and moved a little, so the sizes (and so the tessellation
/// tolerances) differ from curve to curve.
fn variants(kind: &str) -> Vec<Curve> {
    let b = base(kind);
    (0..VARIANTS)
        .map(|i| {
            let k = 1.0 + (i % 4) as f64;
            let m = Transform2::scale(k, k)
                .and_then(|s| s.then(&Transform2::translate(7.0 * i as f64, -3.0 * i as f64)?))
                .expect("compose");
            b.transform(&m).expect("image").0
        })
        .collect()
}

fn arm(name: &str) -> Transform2 {
    match name {
        "identity" => Transform2::IDENTITY,
        "translation" => Transform2::translate(1234.5, -678.25).expect("t"),
        "uniform-scale" => Transform2::scale(3.0, 3.0).expect("s"),
        "rotation-scale" => {
            Transform2::rotate(0.5).and_then(|r| r.then(&Transform2::scale(2.0, 2.0)?)).expect("compose")
        }
        // the source to map step of a chain: the drawing placed at UTM-scale offsets
        "chain-offset" => Transform2::translate(OFFSET[0], OFFSET[1]).expect("t"),
        other => panic!("no transform {other}"),
    }
}

/// The median of seven timings of `f`, each long enough to last 30 ms, in nanoseconds per op (`ops` per call).
fn ns_per_op(ops: usize, mut f: impl FnMut() -> f64) -> f64 {
    let mut runs: Vec<f64> = (0..7)
        .map(|_| {
            let mut reps = 0u32;
            let start = Instant::now();
            let mut acc = 0.0;
            while start.elapsed().as_millis() < 30 {
                acc += f();
                reps += 1;
            }
            black_box(acc);
            start.elapsed().as_nanos() as f64 / (f64::from(reps) * ops as f64)
        })
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[3]
}

fn params(c: &Curve, n: usize, seed: u64) -> Vec<f64> {
    let d = c.domain();
    let mut s = seed;
    (0..n).map(|_| d.lo() + (d.hi() - d.lo()) * (0.001 + 0.998 * lcg(&mut s))).collect()
}

struct Out {
    platform: String,
}

impl Out {
    fn row(&self, a: [&str; 4], budget: Option<f64>, metric: &str, value: Option<f64>, note: &str) {
        let [table, entry, curve, transform] = a;
        let note = match (note.is_empty(), value) {
            (true, None) => "not measured".to_string(),
            (true, Some(_)) => String::new(),
            (false, _) => note.to_string(),
        };
        let r = Row {
            table: table.into(),
            entry: entry.into(),
            platform: self.platform.clone(),
            curve: curve.into(),
            transform: transform.into(),
            budget_px: budget,
            metric: metric.into(),
            value,
            note,
        };
        println!("{}", r.line());
    }
}

/// `point_at`, `derivative` and `bounds` on a set of curves, in nanoseconds per call.
fn eval_costs(curves: &[Curve]) -> (f64, f64, f64) {
    let ps: Vec<Vec<f64>> =
        curves.iter().enumerate().map(|(i, c)| params(c, PARAMS, 99 + i as u64)).collect();
    let ops = curves.len() * PARAMS;
    let at = ns_per_op(ops, || {
        let mut a = 0.0;
        for (c, t) in curves.iter().zip(&ps) {
            for &u in t {
                a += c.eval(black_box(u)).map_or(0.0, |q| q.x);
            }
        }
        a
    });
    let der = ns_per_op(ops, || {
        let mut a = 0.0;
        for (c, t) in curves.iter().zip(&ps) {
            for &u in t {
                a += c.derivative(black_box(u), 1, Side::Right).map_or(0.0, |q| q.x);
            }
        }
        a
    });
    let bd =
        ns_per_op(curves.len(), || curves.iter().map(|c| c.bounds().map_or(0.0, |b| b.diagonal())).sum());
    (at, der, bd)
}

/// The deviation of a polyline from a curve in world units: the largest distance from 4096 points of the curve to
/// the polyline. Measured here, not taken from the tessellator's own report.
fn deviation(c: &Curve, poly: &[Point2]) -> f64 {
    let d = c.domain();
    let dist = |q: Point2| {
        poly.windows(2)
            .map(|w| {
                let (dx, dy) = (w[1].x - w[0].x, w[1].y - w[0].y);
                let l2 = dx * dx + dy * dy;
                let t = if l2 == 0.0 {
                    0.0
                } else {
                    (((q.x - w[0].x) * dx + (q.y - w[0].y) * dy) / l2).clamp(0.0, 1.0)
                };
                q.distance(p(w[0].x + t * dx, w[0].y + t * dy))
            })
            .fold(f64::INFINITY, f64::min)
    };
    (0..4096)
        .map(|k| c.eval(d.lo() + (d.hi() - d.lo()) * (k as f64 / 4095.0)).map_or(0.0, dist))
        .fold(0.0, f64::max)
}

fn eval_table(out: &Out, prototype: Option<f64>) -> BTreeMap<String, f64> {
    let mut at_identity = BTreeMap::new();
    for kind in CURVES {
        let vs = variants(kind);
        for t in ARMS {
            let m = arm(t);
            let moved: Result<Vec<Curve>, String> =
                vs.iter().map(|c| c.transform(&m).map(|x| x.0).map_err(|e| e.to_string())).collect();
            let row = |metric: &str, v: Option<f64>, note: &str| {
                out.row(["eval", "synthetic", kind, t], None, metric, v, note)
            };
            match moved {
                Ok(cs) => {
                    let (at, der, bd) = eval_costs(&cs);
                    if t == "identity" {
                        at_identity.insert(kind.to_string(), at);
                    }
                    row(
                        "point_at_ns",
                        Some(at),
                        "mean over 64 images of the base curve, 256 parameters each",
                    );
                    row("derivative_ns", Some(der), "first derivative from the right");
                    row("bounds_ns", Some(bd), "");
                }
                Err(why) => {
                    for metric in ["point_at_ns", "derivative_ns", "bounds_ns"] {
                        row(metric, None, &format!("the exact image is refused: {why}"));
                    }
                }
            }
            row("hit_test_ns", None, "there is no analytic hit test in the codebase yet (P7.5, pvec#56)");
        }
    }
    // The reference cubic on its own, the way the prototype is measured, for the 150 ns verdict.
    let reference = base("bspline3");
    let mut s = 12345u64;
    let d = reference.domain();
    let ps: Vec<f64> =
        (0..200_000).map(|_| d.lo() + (d.hi() - d.lo()) * (0.001 + 0.998 * lcg(&mut s))).collect();
    let mut runs: Vec<f64> = (0..15)
        .map(|_| {
            let start = Instant::now();
            let mut acc = 0.0;
            for &t in &ps {
                acc += reference.eval(black_box(t)).map_or(0.0, |q| q.x);
            }
            black_box(acc);
            start.elapsed().as_nanos() as f64 / ps.len() as f64
        })
        .collect();
    runs.sort_by(f64::total_cmp);
    let r = ["eval", "synthetic", "bspline3", "identity"];
    out.row(
        r,
        None,
        "point_at_ns",
        Some(runs[7]),
        "the reference cubic alone, 200,000 parameters, median of 15, as the prototype is measured",
    );
    out.row(
        r,
        None,
        "prototype_point_at_ns",
        prototype,
        "the P1.3 prototype on the same curve and parameters (spikes/geometry-api corpus_eval)",
    );
    at_identity
}

/// The offset image against the local frame: the drawing near the origin evaluated exactly, then placed with a
/// two-sum, so the reference has no rounding of its own at the offset.
fn accuracy_table(out: &Out) {
    for kind in CURVES {
        let local = variants(kind);
        let moved: Result<Vec<Curve>, _> =
            local.iter().map(|c| c.transform(&arm("chain-offset")).map(|x| x.0)).collect();
        let Ok(placed) = moved else { continue };
        let (mut worst, mut tol) = (0.0f64, 0.0f64);
        for (i, (l, o)) in local.iter().zip(&placed).enumerate() {
            let scale = l.bounds().map_or(1.0, |b| b.diagonal());
            for t in params(l, 64, 7 + i as u64) {
                if let (Ok(a), Ok(b)) = (l.eval(t), o.eval(t)) {
                    for (got, loc, off) in [(b.x, a.x, OFFSET[0]), (b.y, a.y, OFFSET[1])] {
                        let hi = off + loc;
                        let lo = (off - (hi - loc)) + (loc - (hi - off));
                        worst = worst.max(((got - hi) - lo).abs());
                    }
                    tol = tol.max(Policy::V1.tol(scale, OFFSET[0]));
                }
            }
        }
        out.row(
            ["accuracy", "synthetic", kind, "chain-offset"],
            None,
            "error",
            Some(worst),
            "world units, against the local frame placed with a two-sum",
        );
        out.row(
            ["accuracy", "synthetic", kind, "chain-offset"],
            None,
            "tolerance",
            Some(tol),
            "P1.2 NP-04 tol(scale = curve diagonal, mag = offset)",
        );
    }
}

/// The huge-coordinate corpus entry: its own curves at their own offsets, against exact copies near the origin.
fn huge_entry(out: &Out) {
    let shapes = Spec::named("huge-coordinate-1e5").expect("entry").shapes();
    let there: Vec<Curve> =
        shapes.iter().take(6000).filter_map(|s| pvec_bench::part21::curve(s).ok()).collect();
    let back = Transform2::translate(-OFFSET[0], -OFFSET[1]).expect("t");
    let near: Vec<Curve> = there.iter().filter_map(|c| c.transform(&back).ok().map(|x| x.0)).collect();
    let e = "huge-coordinate-1e5";
    if near.len() == there.len() && !there.is_empty() {
        out.row(
            ["eval", e, "mixed", "chain-offset"],
            None,
            "point_at_ns",
            Some(eval_costs(&there).0),
            &format!("{} curves of the entry at their own coordinates", there.len()),
        );
        out.row(
            ["eval", e, "mixed", "identity"],
            None,
            "point_at_ns",
            Some(eval_costs(&near).0),
            "the same curves moved to the origin by an exact translation",
        );
    } else {
        out.row(
            ["eval", e, "mixed", "chain-offset"],
            None,
            "point_at_ns",
            None,
            "some curves of the entry could not be built or moved",
        );
    }
}

/// Tessellation by curve type and screen budget. One pixel is a two-thousandth of the curve's diagonal, so a curve
/// is drawn about two thousand pixels across whatever its size.
fn tess_table(out: &Out) -> BTreeMap<(String, u64), f64> {
    let mut tess_ns = BTreeMap::new();
    for kind in CURVES {
        let vs = variants(kind);
        for budget in BUDGETS {
            let (mut segs, mut allocs, mut worst_px, mut n, mut failures, mut nanos) =
                (0.0, 0.0f64, 0.0f64, 0.0, 0, 0.0);
            for c in &vs {
                let px = c.bounds().map_or(1.0, |b| b.diagonal()) / 2000.0;
                ALLOCS.store(0, Ordering::Relaxed);
                let start = Instant::now();
                let r = tessellate(c, budget * px);
                let dt = start.elapsed().as_nanos() as f64;
                let a = ALLOCS.load(Ordering::Relaxed) as f64;
                match r {
                    Ok(t) => {
                        nanos += dt;
                        segs += t.points.len().saturating_sub(1) as f64;
                        allocs = allocs.max(a);
                        worst_px = worst_px.max(deviation(c, &t.points) / px);
                        n += 1.0;
                    }
                    Err(_) => failures += 1,
                }
            }
            let r = ["tess", "synthetic", kind, "identity"];
            if n == 0.0 {
                for m in ["segments_per_curve", "ns_per_segment", "allocs_per_tess", "achieved_error_px"] {
                    out.row(
                        r,
                        Some(budget),
                        m,
                        None,
                        "tessellate refused every instance (the unbounded line, or the tolerance not met)",
                    );
                }
                continue;
            }
            let note = if failures > 0 {
                format!("{failures} of {VARIANTS} instances refused")
            } else {
                String::new()
            };
            out.row(r, Some(budget), "segments_per_curve", Some(segs / n), &note);
            out.row(r, Some(budget), "ns_per_segment", Some(nanos / segs.max(1.0)), &note);
            out.row(r, Some(budget), "ns_per_tess", Some(nanos / n), &note);
            out.row(
                r,
                Some(budget),
                "allocs_per_tess",
                Some(allocs),
                "the most allocation calls any one instance made, reallocations included",
            );
            out.row(
                r,
                Some(budget),
                "achieved_error_px",
                Some(worst_px),
                "the most any instance deviated, against 4096 points of the exact curve",
            );
            tess_ns.insert((kind.to_string(), (budget * 100.0).round() as u64), nanos / n);
        }
    }
    out.row(["tess", "synthetic", "all", "identity"], None, "simd_variance", None, "perf is not available in the container and pvec-geometry has no SIMD kernel, so there is no scalar against SIMD pair to measure");
    tess_ns
}

/// The viewport-weighted summary: the architectural entry's real entity mix over the per-type costs.
fn summary(out: &Out, eval_ns: &BTreeMap<String, f64>, tess_ns: &BTreeMap<(String, u64), f64>) {
    let lock =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/benches/corpus/corpus.lock.json"))
            .expect("lock");
    let lock = json::parse(lock.as_bytes()).expect("json");
    let entry = lock
        .get("entries")
        .and_then(Value::as_arr)
        .expect("entries")
        .iter()
        .find(|e| e.get("name").and_then(Value::as_str) == Some("architectural-1e5"))
        .expect("entry");
    let counts = entry.get("counts").and_then(Value::as_obj).expect("counts");
    let total: f64 = counts.iter().map(|(_, v)| v.as_int().unwrap_or(0) as f64).sum();
    fn kind_of(cap: &str) -> &str {
        match cap {
            "trimmed_curve" => "trimmed",
            "b_spline_curve" => "bspline3",
            "rational_b_spline_curve" => "nurbs",
            other => other,
        }
    }
    let (mut at, mut ts, mut covered) = (0.0, 0.0, 0.0);
    for (cap, v) in counts {
        let w = v.as_int().unwrap_or(0) as f64 / total;
        let kind = kind_of(cap);
        if let (Some(e), Some(t)) = (eval_ns.get(kind), tess_ns.get(&(kind.to_string(), 50))) {
            at += w * e;
            ts += w * t;
            covered += w;
        }
    }
    let r = ["summary", "architectural-1e5", "mix", "identity"];
    let note = format!(
        "per-type cost weighted by the entry's capability counts; {:.1}% of the entities mapped",
        covered * 100.0
    );
    out.row(r, None, "weighted_point_at_ns", Some(at / covered), &note);
    out.row(r, Some(0.5), "weighted_ns_per_tess", Some(ts / covered), "at 0.5 px, same weights");
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    match a.first().map(String::as_str) {
        Some("header") => println!("{HEADER}"),
        Some("report") => {
            let rows = parse_rows(&std::fs::read_to_string(&a[1]).expect("the CSV"))
                .unwrap_or_else(|e| panic!("{e}"));
            for v in verdicts(&rows) {
                println!("- {} on {}: {} ({})", v.id, v.platform, v.status.label(), v.detail);
            }
        }
        Some("run") => {
            let prototype = a.get(3).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| {
                t.lines().find_map(|l| l.strip_prefix("prototype_point_at_ns=")?.parse::<f64>().ok())
            });
            let out = Out { platform: a[1].clone() };
            let eval_ns = eval_table(&out, prototype);
            accuracy_table(&out);
            huge_entry(&out);
            let tess_ns = tess_table(&out);
            summary(&out, &eval_ns, &tess_ns);
        }
        _ => eprintln!("usage: geometry run PLATFORM HOST [PROTOTYPE_KV] | header | report CSV"),
    }
}
