//! Issue #267 (#263 slice 3): `pvec inspect` prints the manifest unit and the calibration scale as two lines, and
//! `pvec validate` reports the calibration, including the CAL-11 residual check, without failing the file.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_components::{
    Calibration, ControlPoint, CrsKind, FitMethod, Georeference, Provenance, ScaleClaim, encode_calibration,
};
use pvec_container::writer::RawBlock;
use support::json::validate;
use support::*;

fn calibration(residual_max: f64) -> Calibration {
    let m = [2.0, 0.0, 0.0, 2.0, 1000.0, 2000.0];
    let world = |x: f64, y: f64| [m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5]];
    let mut control_points: Vec<ControlPoint> = [[0.0, 0.0], [8.0, 0.0], [8.0, 6.0]]
        .iter()
        .map(|&[x, y]| ControlPoint { source: [x, y], world: world(x, y) })
        .collect();
    control_points[2].world[0] += 0.5;
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.05,
            provenance: Provenance::Measured { p0: [0.0, 0.0], p1: [8.0, 6.0], distance_metres: 0.5 },
            uncertainty: None,
            note: String::new(),
        }),
        georeference: Some(Georeference {
            source_to_crs: m,
            crs_kind: CrsKind::Engineering,
            method: FitMethod::Fitted,
            crs_id: "LOCAL:SITE-GRID".into(),
            definition: None,
            control_points,
            residual_max,
        }),
        external_id: String::new(),
    }
}

fn calibrated(c: &Calibration) -> Vec<u8> {
    let payload = encode_calibration(c).unwrap();
    rebuild(&good_file(), |m, blocks| {
        m.capabilities.push(Declared {
            id: CapabilityId::new(22),
            requirement: Requirement::Optional,
            version: Version::new(1, 0, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
        blocks.push(RawBlock {
            kind: 11,
            subtype: 0,
            cell: 0,
            count: c.records(),
            bounds: None,
            flags: 0,
            codec: 0,
            capability: 22,
            uncompressed_length: payload.len() as u32,
            stored: payload,
        });
    })
}

#[test]
fn inspect_prints_the_manifest_unit_and_the_calibration_scale_on_two_lines() {
    let path = file("calibrated.pvec", &calibrated(&calibration(0.5)));
    let human = pvec(&["inspect", &path]);
    assert_eq!(human.code, 0, "{}", human.stderr);
    assert!(human.stdout.contains("unit: 0.001 m per source unit (manifest)"), "{}", human.stdout);
    assert!(
        human.stdout.contains("calibration scale: 0.05 m per source unit (measured)"),
        "{}",
        human.stdout
    );
    assert!(
        human.stdout.contains("georeference: CRS LOCAL:SITE-GRID (engineering, fitted, 3 points)"),
        "{}",
        human.stdout
    );
    let run = pvec(&["inspect", "--format", "json", &path]);
    let doc = run.json();
    validate(&schema(), &doc).unwrap_or_else(|e| panic!("{e}\n{}", run.stdout));
    assert_eq!(doc.at("result.calibration.status").str(), "ok");
    assert_eq!(doc.at("result.calibration.scale.metres_per_source_unit").num(), 0.05);
    assert_eq!(doc.at("result.calibration.scale.provenance").str(), "measured");
    assert_eq!(doc.at("result.calibration.georeference.crs_id").str(), "LOCAL:SITE-GRID");
    assert_eq!(doc.at("result.manifest.metres_per_unit").num(), 0.001);
}

#[test]
fn a_file_without_a_calibration_says_so() {
    let path = file("plain-267.pvec", &good_file());
    let human = pvec(&["inspect", &path]);
    assert!(human.stdout.contains("calibration: none"), "{}", human.stdout);
    let doc = pvec(&["inspect", "--format", "json", &path]).json();
    assert_eq!(doc.at("result.calibration.status").str(), "absent");
}

#[test]
fn validate_recomputes_the_residual_and_a_mismatch_refuses_the_calibration_not_the_file() {
    let good = file("residual-ok.pvec", &calibrated(&calibration(0.5)));
    let run = pvec(&["validate", "--format", "json", &good]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let doc = run.json();
    validate(&schema(), &doc).unwrap_or_else(|e| panic!("{e}\n{}", run.stdout));
    assert_eq!(doc.at("result.calibration.status").str(), "ok");
    assert_eq!(doc.at("result.calibration.residual").num(), 0.5);
    let bad = file("residual-bad.pvec", &calibrated(&calibration(0.25)));
    let run = pvec(&["validate", "--format", "json", &bad]);
    assert_eq!(run.code, 0, "the file still conforms: {}", run.stdout);
    let doc = run.json();
    assert_eq!(doc.at("result.calibration.status").str(), "refused");
    assert_eq!(doc.at("result.calibration.rule").str(), "CAL-11");
    let human = pvec(&["validate", &bad]);
    assert!(human.stdout.contains("calibration refused (CAL-11)"), "{}", human.stdout);
}

#[test]
fn a_file_the_reader_refuses_has_its_calibration_unread_not_refused() {
    let needs = rebuild(&calibrated(&calibration(0.5)), |m, _| {
        m.capabilities.push(Declared {
            id: CapabilityId::new(99),
            requirement: Requirement::Required,
            version: Version::new(1, 0, 0),
        });
        m.capabilities.sort_by_key(|d| d.id);
    });
    let path = file("needs99-267.pvec", &needs);
    let run = pvec(&["inspect", "--format", "json", &path]);
    assert_eq!(run.code, 0, "{}", run.stdout);
    let doc = run.json();
    validate(&schema(), &doc).unwrap_or_else(|e| panic!("{e}\n{}", run.stdout));
    assert_eq!(doc.at("result.calibration.status").str(), "unread");
    assert_eq!(doc.at("result.calibration.rule").str(), "NEGOTIATE-01");
}
