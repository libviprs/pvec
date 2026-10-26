//! The entities of a file come back as curves that trace the locus they were written from.

mod support;

use pvec::{Bounds2, CapabilityId, Curve, EntityId, Manifest, PvecWriter};
use pvec_container::reader::{ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_geometry::{
    Circle, Curve2d, Ellipse, Line, Placement2, Polyline, RationalBSpline, Trimmed, Vector2,
};
use support::pt;

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [-50.0, -50.0], max: [50.0, 50.0] },
        capabilities: [1, 4, 5, 6, 7, 8, 12].map(CapabilityId::new).to_vec(),
        producer_id: "decode-test".into(),
        producer_version: "0".into(),
    }
}

fn circle() -> Curve {
    Curve::Circle(Circle::new(Placement2::new(pt(3.0, -2.0), Vector2::new(1.0, 0.0)).unwrap(), 5.0).unwrap())
}

fn along(c: &Curve, fraction: f64) -> (f64, f64) {
    let d = c.domain();
    let p = c.eval(d.lo() + fraction * d.width()).unwrap();
    (p.x, p.y)
}

#[test]
fn every_kind_of_hot_and_advanced_entity_decodes_to_the_locus_it_was_written_from() {
    let line = Curve::Trimmed(
        Trimmed::new(Curve::Line(Line::new(pt(1.0, 1.0), Vector2::new(0.6, 0.8)).unwrap()), 0.0, 7.0, true)
            .unwrap(),
    );
    let ellipse = Curve::Ellipse(
        Ellipse::new(Placement2::new(pt(0.0, 0.0), Vector2::new(1.0, 0.0)).unwrap(), 9.0, 4.0).unwrap(),
    );
    let rational = Curve::RationalBSpline(
        RationalBSpline::new(
            2,
            vec![pt(0.0, 0.0), pt(2.0, 5.0), pt(5.0, 5.0), pt(7.0, 0.0), pt(9.0, 3.0)],
            vec![0.0, 0.0, 0.0, 0.4, 0.7, 1.0, 1.0, 1.0],
            vec![1.0, 2.0, 1.5, 1.0, 3.0],
        )
        .unwrap(),
    );
    let written: Vec<(u32, Curve)> = vec![
        (1, circle()),
        (2, Curve::Trimmed(Trimmed::new(circle(), 0.5, 2.0, true).unwrap())),
        (3, Curve::Trimmed(Trimmed::new(circle(), 2.0, 0.5, false).unwrap())),
        (4, line),
        (5, Curve::Polyline(Polyline::new(vec![pt(0.0, 0.0), pt(4.0, 3.0), pt(8.0, 3.0)]).unwrap())),
        (6, ellipse.clone()),
        (7, Curve::Trimmed(Trimmed::new(ellipse, 0.3, 4.0, true).unwrap())),
        (8, rational),
    ];
    let mut w = PvecWriter::new(manifest()).unwrap();
    for (id, c) in &written {
        w.push(EntityId::new(*id).unwrap(), c).unwrap();
    }
    let bytes = w.finish().unwrap();
    let reader = ContainerReader::open(MemorySource::new(bytes), ReaderConfig::default()).unwrap();
    let decoded = pvec_conformance::decode::decode(&reader).unwrap();
    assert!(decoded.unsupported.is_empty(), "{:?}", decoded.unsupported);
    let ids: Vec<u32> = decoded.curves.iter().map(|(i, _)| *i).collect();
    assert_eq!(ids, [1, 2, 3, 4, 5, 6, 7, 8]);
    for ((id, back), (_, original)) in decoded.curves.iter().zip(&written) {
        for fraction in [0.0, 0.125, 0.5, 0.875, 1.0] {
            let (a, b) = (along(back, fraction), along(original, fraction));
            assert!(
                pvec_numeric::Policy::V1.near_point([a.0, a.1], [b.0, b.1], 10.0),
                "entity {id} at {fraction}: decoded {a:?}, written {b:?}"
            );
        }
    }
}
