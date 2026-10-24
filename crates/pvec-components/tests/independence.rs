//! Independence (P3.7): component streams cannot move a geometry verdict. The geometry is decoded and
//! evaluated with every stream present, with each stream removed in turn, and with all of them removed,
//! and the numbers are identical every time.
#![allow(missing_docs)]

use pvec_capabilities::{Registry, Version, ids};
use pvec_codec::{decode, encode};
use pvec_components::*;
use pvec_entity::EntityId;
use pvec_geometry::*;

fn id(n: u32) -> EntityId {
    EntityId::new(n).unwrap()
}

/// A little drawing: a segment, an arc, a polyline and a spline.
fn geometry() -> Vec<(EntityId, Curve)> {
    let seg = Trimmed::new(
        Curve::Line(Line::new(Point2::new(0.0, 0.0), Vector2::new(3.0, 4.0)).unwrap()),
        0.0,
        5.0,
        true,
    )
    .unwrap();
    let circle =
        Circle::new(Placement2::new(Point2::new(1.0, 1.0), Vector2::new(1.0, 0.0)).unwrap(), 2.0).unwrap();
    let arc = Trimmed::new(Curve::Circle(circle), 0.3, 2.9, true).unwrap();
    let poly =
        Polyline::new(vec![Point2::new(0.0, 0.0), Point2::new(2.0, 1.0), Point2::new(3.0, 3.0)]).unwrap();
    let spline = BSpline::new(
        2,
        vec![Point2::new(0.0, 0.0), Point2::new(1.0, 2.0), Point2::new(2.0, 0.0), Point2::new(3.0, 1.0)],
        vec![0.0, 0.0, 0.0, 0.5, 1.0, 1.0, 1.0],
    )
    .unwrap();
    vec![
        (id(0), Curve::Trimmed(seg)),
        (id(1), Curve::Trimmed(arc)),
        (id(2), Curve::Polyline(poly)),
        (id(3), Curve::BSpline(spline)),
    ]
}

/// Every number a geometry gate looks at: decoded curves evaluated at a sweep of parameters, as bits.
fn verdict(block: &[u8]) -> Vec<u64> {
    let mut out = Vec::new();
    for (eid, c) in decode(block).unwrap() {
        out.push(u64::from(eid.index()));
        let d = c.domain();
        for i in 0..=64 {
            let p = c.eval(d.lo() + d.width() * f64::from(i) / 64.0).unwrap();
            out.push(p.x.to_bits());
            out.push(p.y.to_bits());
        }
        let b = c.bounds().unwrap();
        out.extend([b.min().x.to_bits(), b.min().y.to_bits(), b.max().x.to_bits(), b.max().y.to_bits()]);
    }
    out
}

fn all_streams(r: &Registry) -> Vec<Stream> {
    vec![
        LayerStream { names: vec!["WALLS".into()], assignments: vec![(id(0), 0), (id(1), 0)] }
            .into_stream(r)
            .unwrap(),
        LineStyleStream {
            styles: vec![LineStyleDef { name: "DASHED".into(), dashes: vec![4.0, 2.0] }],
            assignments: vec![(id(2), 0)],
        }
        .into_stream(r)
        .unwrap(),
        TextStream {
            records: vec![TextRecord {
                entity: id(1),
                x: 0.0,
                y: 0.0,
                height: 1.0,
                rotation: 0.0,
                text: "arc".into(),
            }],
        }
        .into_stream(r)
        .unwrap(),
        HatchStream {
            records: vec![HatchRecord {
                boundary: vec![id(0), id(1), id(2)],
                fill: Fill::Solid { rgba: 0x8080_80ff },
            }],
        }
        .into_stream(r)
        .unwrap(),
        ProvenanceStream {
            records: vec![ProvenanceRecord { entity: id(0), source_kind: 1, handle: b"2A".to_vec() }],
        }
        .into_stream(r)
        .unwrap(),
        ReservedStream { payload: vec![0xde, 0xad] }.into_stream(r).unwrap(),
    ]
}

#[test]
fn the_geometry_verdict_is_identical_with_all_streams_with_each_one_removed_and_with_none() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let baseline = verdict(&block);
    assert!(baseline.len() > 100);

    let streams = all_streams(&r);
    // the artifact is the geometry block plus a stream set; the geometry block is never touched
    let with_all = encode_streams(&streams).unwrap();
    let reader = pvec_capabilities::Reader {
        version: Version::new(1, 0, 0),
        implemented: vec![
            ids::LAYER,
            ids::LINE_STYLE,
            ids::TEXT,
            ids::HATCH,
            ids::SOURCE_PROVENANCE,
            ids::DWG_SEMANTICS,
        ],
    };
    let opened = read_streams(&with_all, &r, &reader).unwrap();
    assert_eq!(opened.streams.len(), 6);
    assert_eq!(verdict(&block), baseline, "with every stream present");

    for removed in 0..streams.len() {
        let rest: Vec<Stream> =
            streams.iter().enumerate().filter(|(i, _)| *i != removed).map(|(_, s)| s.clone()).collect();
        let set = encode_streams(&rest).unwrap();
        let opened = read_streams(&set, &r, &reader).unwrap();
        assert_eq!(opened.streams.len(), 5, "without stream {removed}");
        assert_eq!(verdict(&block), baseline, "without stream {removed}");
    }
    let none = encode_streams(&[]).unwrap();
    assert!(read_streams(&none, &r, &reader).unwrap().streams.is_empty());
    assert_eq!(verdict(&block), baseline, "with no stream at all");
}

#[test]
fn a_fixture_with_all_the_streams_still_reaches_display_fidelity_for_linework_with_a_reader_that_knows_none()
{
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let set = encode_streams(&all_streams(&r)).unwrap();
    let nobody = pvec_capabilities::Reader { version: Version::new(1, 0, 0), implemented: vec![] };
    let opened = read_streams(&set, &r, &nobody).unwrap();
    assert_eq!(opened.skipped.len(), 6, "all six are reported skipped, none is an error");
    assert!(opened.streams.is_empty());
    // linework is fully available
    let curves = decode(&block).unwrap();
    assert_eq!(curves.len(), 4);
    for (_, c) in curves {
        assert!(c.eval(c.domain().lo()).is_ok());
    }
}

#[test]
fn an_unknown_stream_in_the_set_does_not_stop_the_geometry_opening() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let mut streams = all_streams(&r);
    streams.push(Stream {
        capability: pvec_capabilities::CapabilityId::new(30_000),
        version: Version::new(1, 0, 0),
        payload: vec![0xff; 11],
    });
    let set = encode_streams(&streams).unwrap();
    let reader = pvec_capabilities::Reader { version: Version::new(1, 0, 0), implemented: vec![ids::LAYER] };
    let opened = read_streams(&set, &r, &reader).unwrap();
    assert!(opened.skipped.iter().any(|s| s.reason == SkipReason::UnknownCapability));
    assert_eq!(verdict(&block).len(), verdict(&block).len());
    assert_eq!(decode(&block).unwrap().len(), 4);
}

#[test]
fn the_components_crate_cannot_reach_the_geometry_or_the_codec() {
    // The structural reason for the independence above: nothing the streams crate depends on can move a
    // curve. pvec-checks holds the allow-list; this reads the manifest directly as a second check.
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let deps = manifest.split("[dependencies]").nth(1).unwrap().split("\n[").next().unwrap();
    assert!(!deps.contains("pvec-codec"));
    assert!(!deps.contains("pvec-geometry"));
}

#[test]
fn a_damaged_payload_in_any_one_stream_cannot_move_the_geometry_verdict_or_stop_the_set_opening() {
    let r = Registry::v1();
    let block = encode(&geometry()).unwrap().bytes;
    let baseline = verdict(&block);
    let nobody = pvec_capabilities::Reader { version: Version::new(1, 0, 0), implemented: vec![] };
    let streams = all_streams(&r);
    for damaged in 0..streams.len() {
        let mut set = streams.clone();
        set[damaged].payload = vec![0xff; 37];
        let opened = read_streams(&encode_streams(&set).unwrap(), &r, &nobody).unwrap();
        assert_eq!(opened.skipped.len(), 6, "stream {damaged}: skipped, never an error");
        assert_eq!(verdict(&block), baseline, "stream {damaged} damaged");
        assert_eq!(decode(&block).unwrap().len(), 4, "stream {damaged} damaged");
    }
}
