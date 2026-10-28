//! Issue #192 (P8.6, pvec#65): what the P8.4 benchmark found about `mvt::tessellate`, as tests that hold on any host.
//!
//! The benchmark measured three things against the tessellator that the committed run listed as failing: it
//! allocates up to 19 times per curve against a limit of 1, halving the budget of a circle doubles its segment
//! count against a limit of 1.5x, and the cubic at 0.25 px reached 0.250185 px. The curves, the variants and the
//! budgets below are the ones `crates/pvec-bench/benches/geometry.rs` uses (a pixel is a two-thousandth of the
//! curve's diagonal), so a number here is the number there. These were written before the tessellator changed.
#![allow(missing_docs, unsafe_code)]

use pvec_geometry::{
    BSpline, Bezier, Circle, Composite, Curve, Curve2d, Ellipse, Line, Placement2, Point2, RationalBSpline,
    Transform2, Trimmed, Vector2,
};
use pvec_interchange::mvt::tessellate;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

struct Counting;

fn bump() {
    // `try_with`: the thread-local may already be gone while a thread shuts down
    let _ = ALLOCS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every call is forwarded unchanged to the system allocator; the only addition is a per-thread counter.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        bump();
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        bump();
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static A: Counting = Counting;

const BUDGETS: [f64; 4] = [0.1, 0.25, 0.5, 1.0];
const VARIANTS: usize = 64;
const KINDS: [&str; 8] =
    ["circle", "ellipse", "trimmed", "composite", "bezier", "bspline3", "bspline7", "nurbs"];

fn p(x: f64, y: f64) -> Point2 {
    Point2::new(x, y)
}

fn place(x: f64, y: f64) -> Placement2 {
    Placement2::new(p(x, y), Vector2::new(1.0, 0.0)).expect("a frame")
}

fn spline(degree: usize, ncp: usize) -> (Vec<Point2>, Vec<f64>) {
    let cps: Vec<Point2> = (0..ncp).map(|i| p(10.0 * i as f64, 3.0 * ((37 * i) % 11) as f64)).collect();
    let spans = ncp - degree;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..spans).map(|k| k as f64 / spans as f64));
    knots.extend(vec![1.0; degree + 1]);
    (cps, knots)
}

fn base(kind: &str) -> Curve {
    let circle = || Curve::Circle(Circle::new(place(0.0, 0.0), 100.0).expect("circle"));
    match kind {
        "circle" => circle(),
        "ellipse" => Curve::Ellipse(Ellipse::new(place(0.0, 0.0), 120.0, 45.0).expect("ellipse")),
        "trimmed" => Curve::Trimmed(Trimmed::new(circle(), 0.3, 2.9, true).expect("trim")),
        "composite" => {
            let line = Line::new(p(0.0, 0.0), Vector2::new(1.0, 0.0)).expect("line");
            let a = Trimmed::new(Curve::Line(line), 0.0, 20.0, true).expect("trim");
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

/// One screen pixel of this curve, in world units.
fn pixel(c: &Curve) -> f64 {
    c.bounds().map_or(1.0, |b| b.diagonal()) / 2000.0
}

/// The largest distance from 4096 points of the curve to the polyline, as the benchmark measures it.
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

#[test]
fn issue_192_one_tessellation_allocates_once_whatever_the_curve_and_the_budget() {
    let mut over = Vec::new();
    for kind in KINDS {
        for c in variants(kind) {
            for budget in BUDGETS {
                let tol = budget * pixel(&c);
                ALLOCS.with(|a| a.set(0));
                let r = tessellate(&c, tol);
                let n = ALLOCS.with(Cell::get);
                if r.is_ok() && n > 1 {
                    over.push(format!("{kind} at {budget} px: {n}"));
                }
            }
        }
    }
    over.dedup();
    assert!(
        over.is_empty(),
        "{} tessellations allocated more than once, first {:?}",
        over.len(),
        &over[..over.len().min(6)]
    );
}

#[test]
fn issue_192_halving_the_budget_of_a_circle_or_an_ellipse_grows_it_by_at_most_one_and_a_half() {
    for kind in ["circle", "ellipse"] {
        let segs = |budget: f64| {
            let vs = variants(kind);
            let n: usize = vs
                .iter()
                .map(|c| tessellate(c, budget * pixel(c)).expect("tessellated").points.len() - 1)
                .sum();
            n as f64 / vs.len() as f64
        };
        for (hi, lo) in [(1.0, 0.5), (0.5, 0.25), (0.25, 0.125)] {
            let (a, b) = (segs(hi), segs(lo));
            assert!(b / a <= 1.5, "{kind}: {hi} px to {lo} px grew {a} to {b} segments, {:.2}x", b / a);
        }
    }
}

#[test]
fn issue_192_no_tessellation_is_further_from_the_curve_than_its_budget() {
    let mut over = Vec::new();
    for kind in KINDS {
        for budget in BUDGETS {
            let mut worst = 0.0f64;
            for c in variants(kind) {
                let px = pixel(&c);
                let t = tessellate(&c, budget * px).expect("tessellated");
                worst = worst.max(deviation(&c, &t.points) / px);
            }
            if worst > budget {
                over.push(format!("{kind} at {budget} px reached {worst:.6} px"));
            }
        }
    }
    assert!(over.is_empty(), "over the budget: {over:?}");
}
