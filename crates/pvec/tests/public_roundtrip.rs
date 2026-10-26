//! Issue #52: write, read and write again through the facade only, with no internal crate in sight.

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [0.0, 0.0], max: [10.0, 10.0] },
        capabilities: Vec::new(),
        producer_id: "roundtrip".into(),
        producer_version: "1".into(),
    }
}

fn write() -> Vec<u8> {
    let mut w = PvecWriter::new(manifest()).unwrap();
    let line = Curve::Polyline(
        // The curve model's constructors are the one thing taken from outside the facade, because the
        // frozen core names `Curve` but not the primitives its variants hold.
        pvec_geometry::Polyline::new(vec![
            pvec_geometry::Point2 { x: 0.0, y: 0.0 },
            pvec_geometry::Point2 { x: 4.0, y: 3.0 },
        ])
        .unwrap(),
    );
    w.push(EntityId::new(1).unwrap(), &line).unwrap();
    w.finish().unwrap()
}

#[test]
fn issue_52_the_second_write_is_byte_identical_to_the_first() {
    let first = write();
    let second = write();
    assert_eq!(first, second);
}

#[test]
fn issue_52_the_reader_returns_the_manifest_and_the_entity_count_the_writer_wrote() {
    let bytes = write();
    let r = PvecReader::open(bytes).unwrap();
    assert_eq!(r.manifest(), &manifest());
    assert_eq!(r.entity_count().unwrap(), 1);
}

mod fixtures {
    use super::*;
    use pvec_corpus::Bundle;
    use pvec_testkit::generate::generate_all;

    /// The canonical artifacts of the vendored corpus (P2.4), as bytes. The generator only supplies
    /// input files here: everything below reads and writes through the facade.
    pub fn canonical() -> Vec<(String, Vec<u8>)> {
        let bundle = Bundle::vendored().expect("the vendored bundle opens");
        generate_all(&bundle)
            .expect("the generator runs")
            .into_iter()
            .filter_map(|o| o.result.ok().map(|a| (o.fixture, a.bytes)))
            .collect()
    }

    /// Reads a file and writes it again, through the facade only.
    pub fn rewrite(bytes: &[u8]) -> Vec<u8> {
        let r = PvecReader::open(bytes.to_vec()).unwrap();
        let mut w = PvecWriter::new(r.manifest().clone()).unwrap();
        for (id, curve) in r.entities().unwrap() {
            w.push(id, &curve).unwrap();
        }
        w.finish().unwrap()
    }
}

#[test]
fn issue_52_the_reader_returns_the_curves_the_writer_wrote() {
    let r = PvecReader::open(write()).unwrap();
    let all = r.entities().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, EntityId::new(1).unwrap());
    let Curve::Polyline(p) = &all[0].1 else { panic!("a polyline went in, got {:?}", all[0].1) };
    assert_eq!(p.vertices().len(), 2);
    assert_eq!((p.vertices()[1].x, p.vertices()[1].y), (4.0, 3.0));
}

#[test]
fn issue_52_writer_reader_writer_is_byte_identical_on_the_canonical_fixtures() {
    let all = fixtures::canonical();
    assert!(all.len() >= 10, "only {} canonical fixtures were generated", all.len());
    for (name, bytes) in &all {
        let first = fixtures::rewrite(bytes);
        let second = fixtures::rewrite(&first);
        assert_eq!(first, second, "{name}: the second write differs from the first");
    }
}

#[test]
fn issue_52_every_entity_of_a_canonical_fixture_survives_the_round_trip() {
    for (name, bytes) in &fixtures::canonical() {
        let before = PvecReader::open(bytes.clone()).unwrap().entities().unwrap();
        assert_eq!(
            before.len() as u64,
            PvecReader::open(bytes.clone()).unwrap().entity_count().unwrap(),
            "{name}"
        );
        let after = PvecReader::open(fixtures::rewrite(bytes)).unwrap().entities().unwrap();
        let ids = |v: &[(EntityId, Curve)]| v.iter().map(|(i, _)| i.index()).collect::<Vec<_>>();
        assert_eq!(ids(&before), ids(&after), "{name}: the entity ids moved");
    }
}

#[test]
fn issue_52_curves_read_back_equal_after_a_rewrite() {
    for (name, bytes) in &fixtures::canonical() {
        let first = fixtures::rewrite(bytes);
        let a = PvecReader::open(first.clone()).unwrap().entities().unwrap();
        let b = PvecReader::open(fixtures::rewrite(&first)).unwrap().entities().unwrap();
        assert_eq!(a, b, "{name}: the curves moved on the second pass");
    }
}

#[test]
fn issue_52_a_hand_built_file_of_every_storable_kind_round_trips() {
    use pvec_geometry::{
        BSpline, Bezier, Circle, Ellipse, Line, Placement2, Point2, RationalBSpline, Trimmed, Vector2,
    };
    let p = |x: f64, y: f64| Point2 { x, y };
    let frame = |x: f64, y: f64, ax: f64, ay: f64| Placement2::new(p(x, y), Vector2::new(ax, ay)).unwrap();
    let spline = |d: f64| {
        BSpline::new(2, vec![p(0.0, d), p(1.0, d + 2.0), p(3.0, d)], vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
            .unwrap()
    };
    let circle = Curve::Circle(Circle::new(frame(5.0, 5.0, 1.0, 0.0), 2.0).unwrap());
    let ellipse = Curve::Ellipse(Ellipse::new(frame(5.0, 5.0, 0.6, 0.8), 3.0, 1.0).unwrap());
    let curves = vec![
        Curve::Polyline(pvec_geometry::Polyline::new(vec![p(0.0, 0.0), p(1.0, 1.0), p(2.0, 0.5)]).unwrap()),
        circle.clone(),
        Curve::trimmed_conic(circle, 1.0, 2.5, false).unwrap(),
        Curve::trimmed_conic(ellipse.clone(), 0.5, 2.0, true).unwrap(),
        ellipse,
        Curve::Trimmed(
            Trimmed::new(
                Curve::Line(Line::new(p(0.1, 0.2), Vector2::new(1.0, 0.7)).unwrap()),
                0.0,
                3.3,
                true,
            )
            .unwrap(),
        ),
        Curve::Bezier(Bezier::new(vec![p(0.0, 0.0), p(1.0, 2.0), p(2.0, 2.0), p(3.0, 0.0)]).unwrap()),
        Curve::BSpline(spline(0.0)),
        Curve::RationalBSpline(
            RationalBSpline::new(
                2,
                vec![p(0.0, 0.0), p(1.0, 2.0), p(3.0, 0.0)],
                vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
                vec![1.0, 0.5, 1.0],
            )
            .unwrap(),
        ),
    ];
    let mut w = PvecWriter::new(manifest()).unwrap();
    for (i, c) in curves.iter().enumerate() {
        w.push(EntityId::new(i as u32 + 1).unwrap(), c).unwrap();
    }
    let first = w.finish().unwrap();
    let r = PvecReader::open(first.clone()).unwrap();
    let read = r.entities().unwrap();
    assert_eq!(read.len(), curves.len());
    assert_eq!(fixtures::rewrite(&first), first);
    // a composite names other entities, and the reader resolves them
    let mut w = PvecWriter::new(manifest()).unwrap();
    let s1 = spline(0.0);
    let s2 = BSpline::new(2, vec![p(3.0, 0.0), p(4.0, 2.0), p(6.0, 0.0)], vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0])
        .unwrap();
    let (e1, e2, e3) = (EntityId::new(1).unwrap(), EntityId::new(2).unwrap(), EntityId::new(3).unwrap());
    w.push(e1, &Curve::BSpline(s1.clone())).unwrap();
    w.push(e2, &Curve::BSpline(s2.clone())).unwrap();
    w.push_composite(
        e3,
        &[pvec::CurveRef { entity: e1 }, pvec::CurveRef { entity: e2 }],
        Bounds2 { min: [0.0, 0.0], max: [3.0, 3.0] },
        false,
    )
    .unwrap();
    let read = PvecReader::open(w.finish().unwrap()).unwrap().entities().unwrap();
    let Curve::Composite(c) = &read[2].1 else { panic!("expected a composite, got {:?}", read[2].1) };
    assert_eq!(c.segments(), &[Curve::BSpline(s1), Curve::BSpline(s2)]);
}
