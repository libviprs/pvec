//! Issue 216: the per-capability `Effect` list of a degraded open is reachable from the facade, so a viewer can
//! say "this file has an image your reader cannot show", and the geometry is bit-identical either way.
#![cfg(feature = "unstable-stream")]

use pvec::unstable::{Effect, ReaderCapabilities, ReaderConfig as Config};
use pvec::{Bounds2, CapabilityId, Curve, CurveEval, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_capabilities::{Declared, Requirement, Version};
use pvec_container::integrity::Integrity;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{RawBlock, assemble};
use pvec_geometry::{Point2, Polyline};

fn plain() -> Vec<u8> {
    let manifest = Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-216".into(),
        producer_version: "1".into(),
    };
    let mut w = PvecWriter::new(manifest).unwrap();
    for i in 0..12u32 {
        let y = f64::from(i) * 1.37;
        let line = Polyline::new(vec![
            Point2 { x: 0.1, y },
            Point2 { x: 10.3, y: y + 1.1 },
            Point2 { x: 20.7, y },
        ])
        .unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    w.finish().unwrap()
}

/// `bytes` with an optional capability 21 declared and a kind 9 and a kind 10 entry naming it.
fn raster(bytes: &[u8]) -> Vec<u8> {
    let r = ContainerReader::open(MemorySource::new(bytes.to_vec()), ReaderConfig::default()).unwrap();
    let mut manifest = r.manifest().clone();
    manifest.capabilities.push(Declared {
        id: CapabilityId::new(21),
        requirement: Requirement::Optional,
        version: Version::new(1, 0, 0),
    });
    manifest.capabilities.sort_by_key(|d| d.id);
    let mut blocks: Vec<RawBlock> = r
        .entries()
        .iter()
        .enumerate()
        .map(|(i, e)| RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags,
            codec: e.codec,
            capability: e.capability,
            uncompressed_length: e.uncompressed_length,
            stored: r.read_stored(BlockId(i as u32)).unwrap().to_vec(),
        })
        .collect();
    for kind in [9u16, 10] {
        blocks.push(RawBlock {
            kind,
            subtype: 0,
            cell: 0,
            count: 1,
            bounds: None,
            flags: 0,
            codec: 0,
            capability: 21,
            uncompressed_length: 16,
            stored: vec![0x5A; 16],
        });
    }
    assemble(manifest, &blocks, Integrity::Full).unwrap()
}

fn open(bytes: Vec<u8>, capabilities: ReaderCapabilities) -> PvecReader {
    PvecReader::open_with_config(bytes, Config { capabilities, ..Config::default() }).unwrap()
}

fn without_21() -> ReaderCapabilities {
    ReaderCapabilities::v1().without(CapabilityId::new(21))
}

#[test]
fn the_degraded_file_reports_unrastered_and_a_plain_file_reports_nothing() {
    let base = plain();
    let degraded = open(raster(&base), without_21());
    assert_eq!(degraded.skipped_effects(), vec![(CapabilityId::new(21), Effect::Unrastered)]);
    let clean = open(base.clone(), without_21());
    assert_eq!(clean.skipped_effects(), vec![]);
    let handed = open(raster(&base), ReaderCapabilities::v1().with(CapabilityId::new(21), 1, 0));
    assert_eq!(handed.skipped_effects(), vec![], "a reader that implements 21 is handed the entries");
}

#[test]
fn the_geometry_is_bit_identical_with_and_without_the_raster_entries() {
    let base = plain();
    let with = open(raster(&base), without_21());
    let without = PvecReader::open(base).unwrap();
    let (a, b) = (with.entities().unwrap(), without.entities().unwrap());
    assert_eq!(a.len(), b.len());
    for ((ia, ca), (ib, cb)) in a.iter().zip(b.iter()) {
        assert_eq!(ia, ib);
        let (da, db) = (ca.domain(), cb.domain());
        assert_eq!((da.lo().to_bits(), da.hi().to_bits()), (db.lo().to_bits(), db.hi().to_bits()));
        for k in 0..=16 {
            let t = da.lo() + da.width() * f64::from(k) / 16.0;
            let (pa, pb) = (ca.eval(t).unwrap(), cb.eval(t).unwrap());
            assert_eq!((pa.x.to_bits(), pa.y.to_bits()), (pb.x.to_bits(), pb.y.to_bits()));
        }
    }
    assert_eq!(with.manifest().bounds.min.map(f64::to_bits), without.manifest().bounds.min.map(f64::to_bits));
    assert_eq!(with.manifest().bounds.max.map(f64::to_bits), without.manifest().bounds.max.map(f64::to_bits));
}
