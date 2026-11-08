//! Issue #268 (#263 slice 4): the facade re-calibrates bytes and files. A file is rewritten to a temporary name in
//! its own directory and renamed over the old one, so a reader never sees half of it and nothing is left beside it.
#![cfg(feature = "unstable-stream")]

use pvec::unstable::{Calibration, Provenance, ScaleClaim, recalibrate, recalibrate_file};
use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_geometry::{Point2, Polyline};

fn manifest() -> Manifest {
    Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-268".into(),
        producer_version: "1".into(),
    }
}

fn scale(metres: f64) -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: metres,
            provenance: Provenance::Measured { p0: [0.0, 0.0], p1: [30.0, 40.0], distance_metres: 50.0 * metres },
            uncertainty: None,
            note: String::new(),
        }),
        georeference: None,
        external_id: String::new(),
    }
}

fn write(c: Option<&Calibration>) -> Vec<u8> {
    let mut w = PvecWriter::new(manifest()).unwrap();
    for i in 0..5u32 {
        let y = f64::from(i) * 9.0;
        let line = Polyline::new(vec![Point2 { x: 2.0, y }, Point2 { x: 60.0, y: y + 3.0 }]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    if let Some(c) = c {
        w.calibration(c).unwrap();
    }
    w.finish().unwrap()
}

#[test]
fn recalibrating_bytes_equals_writing_with_the_calibration() {
    assert_eq!(recalibrate(&write(None), Some(&scale(0.1))).unwrap(), write(Some(&scale(0.1))));
    assert_eq!(recalibrate(&write(Some(&scale(0.1))), None).unwrap(), write(None));
}

#[test]
fn a_file_is_replaced_whole_by_a_rename_and_nothing_is_left_beside_it() {
    let dir = std::env::temp_dir().join(format!("pvec-268-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.pvec");
    std::fs::write(&path, write(None)).unwrap();
    recalibrate_file(&path, Some(&scale(0.2))).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), write(Some(&scale(0.2))));
    let r = PvecReader::open(std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(r.calibration().unwrap(), Some(scale(0.2)));
    recalibrate_file(&path, None).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), write(None));
    let names: Vec<String> =
        std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["plan.pvec"], "a temporary file was left behind");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_refused_calibration_leaves_the_file_untouched() {
    let dir = std::env::temp_dir().join(format!("pvec-268-refused-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("plan.pvec");
    std::fs::write(&path, write(None)).unwrap();
    let e = recalibrate_file(&path, Some(&scale(f64::NAN))).unwrap_err();
    assert!(e.to_string().contains("CAL-04"), "{e}");
    assert_eq!(std::fs::read(&path).unwrap(), write(None));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
