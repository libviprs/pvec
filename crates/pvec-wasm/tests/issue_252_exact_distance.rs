//! Issue #252: `entity_distance` and `entity_touches_box`, the exact distance from a point to an entity and a box
//! test, at ABI version 2 (additive; the 50 exports before them are untouched). `docs/WASM_ABI.md` is the contract.
//! Every curve family is compared with a dense-sampling reference that shares no code with the module.
#![allow(missing_docs)]

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter};
use pvec_geometry::{
    BSpline, Bezier, Circle, Curve2d, Ellipse, Line, Placement2, Point2, Polyline, RationalBSpline, Trimmed,
    Vector2,
};
use pvec_wasm::{AbiError, Runtime, WASM_ABI_VERSION};
use std::fs;
use std::path::Path;

fn pt(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn frame(x: f64, y: f64, angle: f64) -> Placement2 {
    Placement2::new(pt(x, y), Vector2::new(angle.cos(), angle.sin())).unwrap()
}

/// The corpus: one entity of every curve family `hit.rs` handles, ids in this order.
fn curves() -> Vec<(&'static str, Curve)> {
    let line = Line::new(pt(5.0, 5.0), Vector2::new(3.0, 1.0)).unwrap();
    let arc = Trimmed::new(Curve::Circle(Circle::new(frame(300.0, 40.0, 0.0), 35.0).unwrap()), 0.4, 2.6, true);
    let nurbs = RationalBSpline::new(
        2,
        vec![pt(500.0, 0.0), pt(560.0, 90.0), pt(640.0, 10.0)],
        vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        vec![1.0, 2.0, 1.0],
    )
    .unwrap();
    vec![
        ("line", Curve::Trimmed(Trimmed::new(Curve::Line(line), 0.0, 60.0, true).unwrap())),
        ("arc", Curve::Trimmed(arc.unwrap())),
        ("circle", Curve::Circle(Circle::new(frame(150.0, 150.0, 0.0), 40.0).unwrap())),
        ("ellipse", Curve::Ellipse(Ellipse::new(frame(250.0, 150.0, 0.5), 70.0, 20.0).unwrap())),
        (
            "bezier",
            Curve::Bezier(Bezier::new(vec![pt(0.0, 300.0), pt(30.0, 380.0), pt(70.0, 240.0), pt(100.0, 310.0)]).unwrap()),
        ),
        (
            "bspline",
            Curve::BSpline(
                BSpline::new(
                    3,
                    vec![pt(150.0, 300.0), pt(170.0, 350.0), pt(200.0, 270.0), pt(230.0, 340.0), pt(270.0, 300.0)],
                    vec![0.0, 0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0],
                )
                .unwrap(),
            ),
        ),
        ("nurbs", Curve::RationalBSpline(nurbs)),
        (
            "polyline",
            Curve::Polyline(Polyline::new(vec![pt(400.0, 300.0), pt(450.0, 320.0), pt(480.0, 290.0)]).unwrap()),
        ),
    ]
}

fn file() -> Vec<u8> {
    let mut w = PvecWriter::new(Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [-100.0, -100.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-252".into(),
        producer_version: "1".into(),
    })
    .unwrap();
    for (i, (_, c)) in curves().iter().enumerate() {
        w.push(EntityId::new(i as u32).unwrap(), c).unwrap();
    }
    w.finish().unwrap()
}

fn open() -> (Runtime, u32) {
    let mut rt = Runtime::new();
    let f = rt.open(file()).unwrap();
    (rt, f)
}

fn samples(c: &Curve) -> Vec<Point2> {
    let d = c.domain();
    let n = 400_000;
    (0..=n).map(|k| c.eval(d.lo() + (d.hi() - d.lo()) * f64::from(k) / f64::from(n)).unwrap()).collect()
}

fn sampled_distance(s: &[Point2], p: Point2) -> f64 {
    s.iter().map(|q| (q.x - p.x).hypot(q.y - p.y)).fold(f64::INFINITY, f64::min)
}

fn sampled_box_distance(s: &[Point2], b: [f64; 4]) -> f64 {
    s.iter()
        .map(|q| (b[0] - q.x).max(0.0).max(q.x - b[2]).hypot((b[1] - q.y).max(0.0).max(q.y - b[3])))
        .fold(f64::INFINITY, f64::min)
}

/// Points around a curve: on it, near it, far from it, inside its bounds and off to the side.
fn probes(s: &[Point2]) -> Vec<Point2> {
    let (mut lo, mut hi) = (pt(f64::INFINITY, f64::INFINITY), pt(f64::NEG_INFINITY, f64::NEG_INFINITY));
    for q in s {
        lo = pt(lo.x.min(q.x), lo.y.min(q.y));
        hi = pt(hi.x.max(q.x), hi.y.max(q.y));
    }
    let (cx, cy) = ((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0);
    let (w, h) = (hi.x - lo.x, hi.y - lo.y);
    vec![
        pt(cx, cy),
        pt(lo.x - 0.3 * w - 1.0, cy),
        pt(hi.x + 0.2 * w + 1.0, hi.y + 0.4 * h + 1.0),
        pt(cx + 0.13 * w, lo.y - 0.07 * h - 1.0),
        pt(lo.x + 0.31 * w, lo.y + 0.62 * h),
        pt(hi.x - 0.17 * w, lo.y + 0.21 * h),
        pt(cx + 5000.0, cy - 3000.0),
    ]
}

#[test]
fn issue_252_the_abi_stays_2_and_the_old_50_exports_are_still_there_beside_the_two_new_ones() {
    assert_eq!(WASM_ABI_VERSION, 2);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = fs::read_to_string(root.join("src/bindings.rs")).unwrap();
    let names: Vec<&str> = src
        .split("#[wasm_bindgen]\npub fn ")
        .skip(1)
        .map(|rest| rest.split('(').next().unwrap())
        .collect();
    assert_eq!(names.len(), 52, "exports: {names:?}");
    for new in ["entity_distance", "entity_touches_box"] {
        assert!(names.contains(&new), "{new} is not exported");
    }
    // the 50 before this issue, by name
    for old in [
        "wasm_abi_version", "open", "query_viewport", "batch_len", "batch_truncated", "tessellate_batch",
        "buffer_len", "buffer_ptr", "buffer_elem", "tessellation_failed", "tessellation_conformance",
        "tessellation_origin_x", "tessellation_origin_y", "memory_epoch", "view_current", "force_grow", "release",
        "directory_stat", "directory_bound", "directory_query", "cells_len", "cells_ptr", "cells_elem",
        "tier0_viewport", "query_leaf", "tier0_leaf", "tier0_stat", "tier0_buffer_len", "tier0_buffer_ptr",
        "tier0_buffer_elem", "open_ranged", "provide_range", "open_poll", "needed_count", "needed_offset",
        "needed_len", "raster_stat", "raster_skipped", "raster_placement", "raster_plan", "raster_tile",
        "raster_buffer_len", "raster_buffer_ptr", "raster_buffer_elem",
    ] {
        assert!(names.contains(&old), "the old export {old} is gone");
    }
    let doc = fs::read_to_string(root.join("../../docs/WASM_ABI.md")).unwrap();
    for new in ["entity_distance", "entity_touches_box"] {
        assert!(doc.contains(&format!("`{new}`")), "docs/WASM_ABI.md does not name {new}");
    }
}

#[test]
fn issue_252_every_curve_family_matches_the_dense_sampling_reference() {
    let (mut rt, f) = open();
    for (i, (name, c)) in curves().iter().enumerate() {
        let s = samples(c);
        for p in probes(&s) {
            let want = sampled_distance(&s, p);
            let got = rt.entity_distance(f, i as u32, p.x, p.y).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            // sampling can only overshoot the true minimum, by a hair at 400,000 samples
            assert!(got <= want + 1e-9, "{name} at {p:?}: exact {got} is above the sample minimum {want}");
            assert!((got - want).abs() < 1e-4, "{name} at {p:?}: exact {got}, reference {want}");
        }
    }
}

#[test]
fn issue_252_a_point_on_the_curve_is_at_distance_0() {
    let (mut rt, f) = open();
    for (i, (name, c)) in curves().iter().enumerate() {
        let d = c.domain();
        for k in [0.0, 0.17, 0.5, 0.83, 1.0] {
            let p = c.eval(d.lo() + (d.hi() - d.lo()) * k).unwrap();
            let got = rt.entity_distance(f, i as u32, p.x, p.y).unwrap();
            assert!(got < 1e-6, "{name} at parameter fraction {k}: {got}");
        }
    }
}

#[test]
fn issue_252_a_released_handle_an_unknown_entity_and_a_non_finite_argument_are_refused() {
    let (mut rt, f) = open();
    assert_eq!(rt.entity_distance(f, 999, 0.0, 0.0), Err(AbiError::BadArgument));
    for (x, y) in [(f64::NAN, 0.0), (0.0, f64::NAN), (f64::INFINITY, 0.0), (0.0, f64::NEG_INFINITY)] {
        assert_eq!(rt.entity_distance(f, 0, x, y), Err(AbiError::BadArgument), "{x} {y}");
    }
    assert_eq!(rt.entity_touches_box(f, 999, Bounds2 { min: [0.0, 0.0], max: [1.0, 1.0] }), Err(AbiError::BadArgument));
    for b in [[f64::NAN, 0.0, 1.0, 1.0], [0.0, 0.0, f64::INFINITY, 1.0], [2.0, 0.0, 1.0, 1.0]] {
        let bounds = Bounds2 { min: [b[0], b[1]], max: [b[2], b[3]] };
        assert_eq!(rt.entity_touches_box(f, 0, bounds), Err(AbiError::BadArgument), "{b:?}");
    }
    // a handle that is not a file
    let q = rt.query_viewport(f, Bounds2 { min: [-1e9, -1e9], max: [1e9, 1e9] }, 10).unwrap();
    assert_eq!(rt.entity_distance(q, 0, 0.0, 0.0), Err(AbiError::WrongKind));
    rt.release(f).unwrap();
    assert_eq!(rt.entity_distance(f, 0, 0.0, 0.0), Err(AbiError::Released));
    assert_eq!(
        rt.entity_touches_box(f, 0, Bounds2 { min: [0.0, 0.0], max: [1.0, 1.0] }),
        Err(AbiError::Released)
    );
}

#[test]
fn issue_252_the_box_test_agrees_with_the_sampled_curve() {
    let (mut rt, f) = open();
    let (mut yes, mut no) = (0, 0);
    for (i, (name, c)) in curves().iter().enumerate() {
        let s = samples(c);
        let mut boxes = Vec::new();
        for p in probes(&s) {
            for half in [0.5, 4.0, 25.0, 90.0] {
                boxes.push([p.x - half, p.y - half * 0.6, p.x + half, p.y + half * 0.6]);
            }
        }
        // a box that holds the whole curve
        boxes.push([-1e6, -1e6, 1e6, 1e6]);
        for b in boxes {
            let gap = sampled_box_distance(&s, b);
            let want = if gap == 0.0 {
                true
            } else if gap > 1e-3 {
                false
            } else {
                continue; // too close to call by sampling
            };
            let bounds = Bounds2 { min: [b[0], b[1]], max: [b[2], b[3]] };
            let got = rt.entity_touches_box(f, i as u32, bounds).unwrap();
            assert_eq!(got, want, "{name} against {b:?} (gap {gap})");
            if want { yes += 1 } else { no += 1 }
        }
    }
    assert!(yes > 20 && no > 20, "the boxes must cover both answers: {yes} touch, {no} do not");
}

#[test]
fn issue_252_a_box_inside_a_circle_and_away_from_its_curve_does_not_touch_it() {
    let (mut rt, f) = open();
    let id = curves().iter().position(|(n, _)| *n == "circle").unwrap() as u32;
    let inside = Bounds2 { min: [140.0, 140.0], max: [160.0, 160.0] };
    assert_eq!(rt.entity_touches_box(f, id, inside), Ok(false));
    let crossing = Bounds2 { min: [180.0, 140.0], max: [200.0, 160.0] };
    assert_eq!(rt.entity_touches_box(f, id, crossing), Ok(true));
    // the distance of the centre is the radius
    let d = rt.entity_distance(f, id, 150.0, 150.0).unwrap();
    assert!((d - 40.0).abs() < 1e-9, "{d}");
}
