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
