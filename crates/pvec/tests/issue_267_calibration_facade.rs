//! Issue #267 (#263 slice 3): the facade writes a calibration and reads it back through `pvec::unstable`, and
//! `effective_scale` never gives a bare number: the calibration's scale with its provenance when there is one,
//! the manifest's unit otherwise.
#![cfg(feature = "unstable-stream")]

use pvec::unstable::{
    Calibration, CrsKind, EffectiveScale, FitMethod, Georeference, Provenance, ReaderCapabilities, ReaderConfig,
    ScaleClaim,
};
use pvec::{Bounds2, CapabilityId, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_geometry::{Point2, Polyline};

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-267".into(),
        producer_version: "1".into(),
    }
}

fn calibration() -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.1,
            provenance: Provenance::PlanScale { numerator: 1, denominator: 100, paper_metres_per_unit: 0.001 },
            uncertainty: Some(0.005),
            note: "1:100".into(),
        }),
        georeference: Some(Georeference {
            source_to_crs: [0.1, 0.0, 0.0, 0.1, 530_000.0, 180_000.0],
            crs_kind: CrsKind::Projected,
            method: FitMethod::Manual,
            crs_id: "EPSG:27700".into(),
            definition: None,
            control_points: Vec::new(),
            residual_max: 0.0,
        }),
        external_id: String::new(),
    }
}

fn write(mut w: PvecWriter, with: Option<&Calibration>) -> Vec<u8> {
    for i in 0..6u32 {
        let y = f64::from(i) * 7.5;
        let line = Polyline::new(vec![Point2 { x: 1.0, y }, Point2 { x: 50.0, y: y + 2.0 }]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    if let Some(c) = with {
        w.calibration(c).unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn the_facade_writes_and_reads_a_calibration() {
    let bytes = write(PvecWriter::new(manifest()).unwrap(), Some(&calibration()));
    let r = PvecReader::open(bytes).unwrap();
    assert_eq!(r.calibration().unwrap(), Some(calibration()));
    assert_eq!(r.georeference().unwrap(), calibration().georeference);
    assert!(r.manifest().capabilities.contains(&CapabilityId::new(22)));
    assert_eq!(r.manifest().metres_per_unit, 0.001, "the manifest unit is never touched");
}

#[test]
fn effective_scale_is_the_calibration_with_its_provenance_or_the_manifest() {
    let bytes = write(PvecWriter::new(manifest()).unwrap(), Some(&calibration()));
    let got = PvecReader::open(bytes).unwrap().effective_scale().unwrap();
    assert_eq!(
        got,
        EffectiveScale::Calibration {
            metres_per_source_unit: 0.1,
            provenance: Provenance::PlanScale { numerator: 1, denominator: 100, paper_metres_per_unit: 0.001 },
            uncertainty: Some(0.005),
        }
    );
    let plain = write(PvecWriter::new(manifest()).unwrap(), None);
    let r = PvecReader::open(plain).unwrap();
    assert_eq!(r.calibration().unwrap(), None);
    assert_eq!(r.georeference().unwrap(), None);
    assert_eq!(r.effective_scale().unwrap(), EffectiveScale::Manifest { metres_per_unit: 0.001 });
}

#[test]
fn a_georeference_without_a_scale_leaves_the_manifest_as_the_effective_scale() {
    let mut c = calibration();
    c.scale = None;
    let bytes = write(PvecWriter::new(manifest()).unwrap(), Some(&c));
    let r = PvecReader::open(bytes).unwrap();
    assert_eq!(r.effective_scale().unwrap(), EffectiveScale::Manifest { metres_per_unit: 0.001 });
    assert!(r.georeference().unwrap().is_some());
}

#[test]
fn the_indexed_writer_keeps_the_calibration_and_a_reader_without_22_does_not_see_it() {
    let bytes = write(PvecWriter::new_indexed(manifest(), 4, None).unwrap(), Some(&calibration()));
    let r = PvecReader::open(bytes.clone()).unwrap();
    assert_eq!(r.calibration().unwrap(), Some(calibration()));
    let config = ReaderConfig {
        capabilities: ReaderCapabilities::v1().without(CapabilityId::new(22)),
        ..ReaderConfig::default()
    };
    let old = PvecReader::open_with_config(bytes, config).unwrap();
    assert_eq!(old.calibration().unwrap(), None);
    assert_eq!(old.effective_scale().unwrap(), EffectiveScale::Manifest { metres_per_unit: 0.001 });
}

#[test]
fn a_refused_calibration_is_an_error_with_its_rule() {
    let mut c = calibration();
    c.georeference.as_mut().unwrap().crs_id = "EPSG 27700".into();
    let mut w = PvecWriter::new(manifest()).unwrap();
    let e = w.calibration(&c).unwrap_err();
    assert!(e.to_string().contains("CAL-07"), "{e}");
}
