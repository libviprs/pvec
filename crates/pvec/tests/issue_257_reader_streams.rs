//! Issue #257: `PvecReader::streams` reads the component streams back through the facade, in file order,
//! with the ones the reader stepped over listed apart. A file with no component block gives an empty pair,
//! and dropping one stream leaves the others parsing.

use pvec::components::Stream;
use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_capabilities::{Registry, Version};
use pvec_components::{LayerStream, SemanticClassStream, SkipReason};

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [10.0, 10.0] },
        capabilities: Vec::new(),
        producer_id: "issue-257".into(),
        producer_version: "1".into(),
    }
}

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

fn layers() -> Stream {
    LayerStream {
        names: vec!["walls".into(), "doors".into()],
        flags: vec![0, 0],
        assignments: vec![(id(1), 0)],
    }
    .into_stream(&Registry::v1())
    .unwrap()
}

fn classes() -> Stream {
    let version = Registry::v1().get(SemanticClassStream::CAPABILITY).unwrap().version;
    Stream { capability: SemanticClassStream::CAPABILITY, version, payload: Vec::new() }
}

fn write(streams: Vec<Stream>) -> Vec<u8> {
    let mut w = PvecWriter::new(manifest()).unwrap();
    let line = Curve::Polyline(
        pvec_geometry::Polyline::new(vec![
            pvec_geometry::Point2 { x: 0.0, y: 0.0 },
            pvec_geometry::Point2 { x: 4.0, y: 3.0 },
        ])
        .unwrap(),
    );
    w.push(id(1), &line).unwrap();
    for s in streams {
        w.components(s).unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn issue_257_a_file_with_no_component_block_gives_an_empty_pair() {
    let r = PvecReader::open(write(Vec::new())).unwrap();
    let (streams, skipped) = r.streams().unwrap();
    assert!(streams.is_empty() && skipped.is_empty());
}

#[test]
fn issue_257_the_streams_come_back_as_written() {
    let layer = layers();
    let r = PvecReader::open(write(vec![layer.clone()])).unwrap();
    let (streams, skipped) = r.streams().unwrap();
    assert_eq!(streams, vec![layer]);
    assert!(skipped.is_empty());
    let back = LayerStream::decode(&streams[0].payload).unwrap();
    assert_eq!(back.names, ["walls", "doors"]);
}

#[test]
fn issue_257_a_stream_of_another_major_version_is_listed_as_skipped() {
    let newer = Stream {
        capability: SemanticClassStream::CAPABILITY,
        version: Version::new(9, 0, 0),
        payload: vec![1, 2, 3],
    };
    let layer = layers();
    let r = PvecReader::open(write(vec![layer.clone(), newer])).unwrap();
    let (streams, skipped) = r.streams().unwrap();
    assert_eq!(streams, vec![layer]);
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0].capability, SemanticClassStream::CAPABILITY);
    assert_eq!(skipped[0].reason, SkipReason::IncompatibleVersion);
}

#[test]
fn issue_257_each_stream_can_be_dropped_and_the_rest_still_parse() {
    let all = [layers(), classes()];
    for drop in 0..all.len() {
        let kept: Vec<Stream> =
            all.iter().enumerate().filter(|(i, _)| *i != drop).map(|(_, s)| s.clone()).collect();
        let r = PvecReader::open(write(kept.clone())).unwrap();
        assert_eq!(r.entity_count().unwrap(), 1);
        let (streams, skipped) = r.streams().unwrap();
        assert_eq!(streams, kept, "dropping stream {drop}");
        assert!(skipped.is_empty());
    }
}
