//! Issue #201: the facade writer can lay a spatial directory and per-leaf Tier 0 blocks into the file
//! (P5.1 and P5.4b), and the facade reader hands them back. Both sit behind `unstable-stream`.
#![cfg(feature = "unstable-stream")]

use pvec::unstable::{PreviewForm, Tier0Entry};
use pvec::{Bounds2, Curve, CurveRef, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_geometry::{BSpline, Ellipse, Placement2, Point2, Polyline, Vector2};

const HOT_SHORT: u32 = 100;
const MAX_ERROR: f64 = 0.05;

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [1000.0, 1000.0] },
        capabilities: Vec::new(),
        producer_id: "issue-201".into(),
        producer_version: "1".into(),
    }
}

fn p(x: f64, y: f64) -> Point2 {
    Point2 { x, y }
}

fn id(i: u32) -> EntityId {
    EntityId::new(i).unwrap()
}

/// 100 short segments (exact in Tier 0), one 300 vertex wave (a preview), one ellipse (a preview), one
/// spline and a composite of two splines (both advanced, so in no leaf). 104 entities, 102 of them hot.
fn push_all(w: &mut PvecWriter) {
    for i in 0..HOT_SHORT {
        let (x, y) = (f64::from(i % 10) * 10.0, f64::from(i / 10) * 10.0);
        let seg = Polyline::new(vec![p(x, y), p(x + 4.0, y + 3.0)]).unwrap();
        w.push(id(i), &Curve::Polyline(seg)).unwrap();
    }
    let wave: Vec<Point2> =
        (0..300).map(|k| p(f64::from(k) * 3.0, 500.0 + (f64::from(k) * 0.1).sin() * 40.0)).collect();
    w.push(id(100), &Curve::Polyline(Polyline::new(wave).unwrap())).unwrap();
    let frame = Placement2::new(p(700.0, 700.0), Vector2::new(1.0, 0.0)).unwrap();
    w.push(id(101), &Curve::Ellipse(Ellipse::new(frame, 90.0, 30.0).unwrap())).unwrap();
    let spline = |dy: f64| {
        BSpline::new(
            2,
            vec![p(10.0, 900.0 + dy), p(60.0, 960.0 + dy), p(120.0, 900.0 + dy)],
            vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        )
        .unwrap()
    };
    w.push(id(102), &Curve::BSpline(spline(0.0))).unwrap();
    w.push(id(103), &Curve::BSpline(spline(5.0))).unwrap();
}

fn write(indexed: Option<(u32, Option<f64>)>) -> Vec<u8> {
    let mut w = match indexed {
        Some((capacity, tier0)) => PvecWriter::new_indexed(manifest(), capacity, tier0).unwrap(),
        None => PvecWriter::new(manifest()).unwrap(),
    };
    push_all(&mut w);
    w.finish().unwrap()
}

#[test]
fn issue_201_a_plain_writer_leaves_no_directory() {
    let r = PvecReader::open(write(None)).unwrap();
    assert!(r.spatial_directory().unwrap().is_none());
}

#[test]
fn issue_201_an_indexed_writer_lays_a_directory_over_the_hot_entities() {
    let r = PvecReader::open(write(Some((16, None)))).unwrap();
    let dir = r.spatial_directory().unwrap().expect("a directory");
    assert_eq!(dir.entities(), 102);
    assert_eq!(dir.cells(), 7, "102 hot entities in cells of 16");
    assert!(dir.advanced_bounds().is_some(), "the splines are in the advanced set");
    let leaves = r.tier0_leaves(&dir).unwrap();
    assert!(leaves.is_empty(), "no Tier 0 was asked for");
}

#[test]
fn issue_201_tier0_covers_every_hot_entity_once_and_declares_each_preview_error() {
    let r = PvecReader::open(write(Some((16, Some(MAX_ERROR))))).unwrap();
    let dir = r.spatial_directory().unwrap().unwrap();
    let leaves = r.tier0_leaves(&dir).unwrap();
    assert_eq!(leaves.len(), dir.cells());
    let (mut covered, mut previews, mut seen) = (0, 0, Vec::new());
    for cell in dir.query(&dir.bounds()) {
        let plan = r.tier0_plan(&leaves, cell.id).unwrap().expect("every leaf has a block");
        assert_eq!(plan.covered(), cell.entities as usize);
        covered += plan.covered();
        for pr in plan.previews() {
            previews += 1;
            assert!(pr.error_bound() > 0.0 && pr.error_bound() <= MAX_ERROR, "{}", pr.error_bound());
            assert!(matches!(pr.form(), PreviewForm::ChordPolyline | PreviewForm::ControlPolygon));
            seen.push(pr.replaces().index());
        }
        for e in plan.exact_ids() {
            assert!(matches!(plan.entry(e), Some(Tier0Entry::Exact { .. })));
            seen.push(e.index());
        }
    }
    assert_eq!(covered, 102);
    assert_eq!(previews, 2, "the wave and the ellipse");
    seen.sort_unstable();
    assert_eq!(seen, (0..102).collect::<Vec<u32>>(), "each hot entity once, the splines in no leaf");
}

#[test]
fn issue_201_the_leaves_report_their_stored_bytes() {
    let r = PvecReader::open(write(Some((16, Some(MAX_ERROR))))).unwrap();
    let dir = r.spatial_directory().unwrap().unwrap();
    for cell in dir.query(&dir.bounds()) {
        assert!(r.stored_bytes(cell.blocks) > 0);
    }
}

#[test]
fn issue_201_the_indexed_file_reads_back_the_same_entities_and_is_deterministic() {
    let plain = PvecReader::open(write(None)).unwrap().entities().unwrap();
    let a = write(Some((16, Some(MAX_ERROR))));
    let b = write(Some((16, Some(MAX_ERROR))));
    assert_eq!(a, b);
    assert_eq!(PvecReader::open(a).unwrap().entities().unwrap(), plain);
}

#[test]
fn issue_201_a_composite_of_advanced_curves_does_not_break_tier0_coverage() {
    let mut w = PvecWriter::new_indexed(manifest(), 16, Some(MAX_ERROR)).unwrap();
    push_all(&mut w);
    let segs = [CurveRef { entity: id(102) }, CurveRef { entity: id(103) }];
    w.push_composite(id(104), &segs, Bounds2 { min: [10.0, 900.0], max: [120.0, 965.0] }, false).unwrap();
    let r = PvecReader::open(w.finish().unwrap()).unwrap();
    let dir = r.spatial_directory().unwrap().unwrap();
    assert_eq!(dir.entities(), 102);
    let leaves = r.tier0_leaves(&dir).unwrap();
    assert_eq!(leaves.len(), dir.cells());
}

#[test]
fn issue_201_a_leaf_capacity_of_zero_is_refused_not_papered_over() {
    let mut w = PvecWriter::new_indexed(manifest(), 0, None).unwrap();
    push_all(&mut w);
    assert!(w.finish().is_err());
}
