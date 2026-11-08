//! Issue #269 (#263 slice 5, ADR 0019): the calibration exports at ABI version 2. The file is a calibrated one the
//! facade wrote, so every number the module returns is compared bit for bit with what was written, and the ranged
//! open asks for the calibration block in one range. `docs/WASM_ABI.md` is the contract.

use pvec::unstable::{
    Calibration, ControlPoint, CrsDefinition, CrsKind, DefinitionFormat, FitMethod, Georeference, Provenance,
    ScaleClaim,
};
use pvec::{Bounds2, Curve, EntityId, Manifest, PvecWriter};
use pvec_container::integrity::Integrity;
use pvec_container::writer::{RawBlock, assemble};
use pvec_geometry::{Point2, Polyline};
use pvec_wasm::{AbiError, Runtime, WASM_ABI_VERSION};

fn calibration() -> Calibration {
    let m = [0.5, 0.0, 0.0, 0.5, 530_000.0, 180_000.0];
    let control_points: Vec<ControlPoint> = [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0]]
        .iter()
        .map(|&[x, y]| ControlPoint {
            source: [x, y],
            world: [m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5]],
        })
        .collect();
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.1,
            provenance: Provenance::PlanScale {
                numerator: 1,
                denominator: 100,
                paper_metres_per_unit: 0.001,
            },
            uncertainty: Some(0.005),
            note: "1:100".into(),
        }),
        georeference: Some(Georeference {
            source_to_crs: m,
            crs_kind: CrsKind::Projected,
            method: FitMethod::Fitted,
            crs_id: "EPSG:27700".into(),
            definition: Some(CrsDefinition {
                format: DefinitionFormat::Wkt2,
                text: "PROJCRS[\"OSGB36\"]".into(),
            }),
            control_points,
            residual_max: 0.0,
        }),
        external_id: "sheet-7".into(),
    }
}

fn file(c: Option<&Calibration>) -> Vec<u8> {
    let manifest = Manifest {
        metres_per_unit: 0.001,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-269".into(),
        producer_version: "1".into(),
    };
    let mut w = PvecWriter::new(manifest).unwrap();
    for i in 0..4u32 {
        let y = f64::from(i) * 11.0;
        let line = Polyline::new(vec![Point2 { x: 1.0, y }, Point2 { x: 70.0, y: y + 5.0 }]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    if let Some(c) = c {
        w.calibration(c).unwrap();
    }
    w.finish().unwrap()
}

fn open(bytes: Vec<u8>) -> (Runtime, u32) {
    let mut rt = Runtime::new();
    let f = rt.open(bytes).unwrap();
    (rt, f)
}

#[test]
fn issue_269_the_abi_stays_2_and_the_documents_name_the_group() {
    assert_eq!(WASM_ABI_VERSION, 2);
    let doc = std::fs::read_to_string(pvec_checks::repo_root().join("docs/WASM_ABI.md")).unwrap();
    assert!(doc.contains("## Calibration (version 2, additive; issue #269, ADR 0019)"));
    for name in ["calibration_stat", "calibration_number", "calibration_point", "calibration_text"] {
        assert!(doc.contains(&format!("`{name}(")), "WASM_ABI.md does not describe {name}");
    }
    let bindings =
        std::fs::read_to_string(pvec_checks::repo_root().join("crates/pvec-wasm/src/bindings.rs")).unwrap();
    for name in ["calibration_stat", "calibration_number", "calibration_point", "calibration_text"] {
        assert!(bindings.contains(&format!("pub fn {name}(")), "no export {name}");
    }
    let m = std::fs::read_to_string(pvec_checks::repo_root().join("docs/MIGRATION.md")).unwrap();
    assert!(m.contains("probed by `calibration_stat`"));
}

#[test]
fn issue_269_the_integers_describe_the_records() {
    let (mut rt, f) = open(file(Some(&calibration())));
    let stat = |rt: &mut Runtime, n| rt.calibration_stat(f, n).unwrap();
    let got: Vec<u32> = (0..12).map(|n| stat(&mut rt, n)).collect();
    // records, provenance, has uncertainty, CRS kind, method, points, definition format, rule, family,
    // uncalibrated, ratio numerator, ratio denominator
    assert_eq!(got, [3, 2, 1, 1, 2, 3, 1, 0, 0, 0, 1, 100]);
    assert_eq!(rt.calibration_stat(f, 12), Err(AbiError::BadArgument));
}

#[test]
fn issue_269_the_floats_are_the_written_bits() {
    let (mut rt, f) = open(file(Some(&calibration())));
    let c = calibration();
    let (s, g) = (c.scale.unwrap(), c.georeference.unwrap());
    let mut want = vec![s.metres_per_source_unit, s.uncertainty.unwrap(), g.residual_max];
    want.extend(g.source_to_crs);
    want.extend([0.001, 0.001]); // the manifest unit, the paper unit
    let got: Vec<u64> = (0..11).map(|n| rt.calibration_number(f, n).to_bits()).collect();
    assert_eq!(got, want.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    for n in 11..16 {
        assert!(rt.calibration_number(f, n).is_nan(), "{n}: a plan scale has no measured evidence");
    }
    for (i, p) in g.control_points.iter().enumerate() {
        let got: Vec<u64> = (0..4).map(|w| rt.calibration_point(f, i as u32, w).to_bits()).collect();
        let want = [p.source[0], p.source[1], p.world[0], p.world[1]];
        assert_eq!(got, want.iter().map(|x| x.to_bits()).collect::<Vec<_>>());
    }
    assert!(rt.calibration_point(f, 3, 0).is_nan() && rt.calibration_point(f, 0, 4).is_nan());
}

#[test]
fn issue_269_the_texts_come_back_as_bytes() {
    let (mut rt, f) = open(file(Some(&calibration())));
    // a result handle whose array 0 is the bytes: no export returns a string (packed_abi, issue 54)
    let text = |rt: &mut Runtime, n| {
        let h = rt.calibration_text(f, n).unwrap();
        let s = String::from_utf8(rt.raster_u8(h, 0).unwrap().to_vec()).unwrap();
        rt.release(h).unwrap();
        s
    };
    assert_eq!(text(&mut rt, 0), "EPSG:27700");
    assert_eq!(text(&mut rt, 1), "PROJCRS[\"OSGB36\"]");
    assert_eq!(text(&mut rt, 2), "1:100");
    assert_eq!(text(&mut rt, 3), "sheet-7");
    assert_eq!(rt.calibration_text(f, 4), Err(AbiError::BadArgument));
}

#[test]
fn issue_269_a_file_without_a_calibration_has_zeros_nans_and_the_manifest_unit() {
    let (mut rt, f) = open(file(None));
    assert_eq!(rt.calibration_stat(f, 0), Ok(0));
    assert_eq!(rt.calibration_stat(f, 9), Ok(0));
    assert!(rt.calibration_number(f, 0).is_nan());
    assert_eq!(rt.calibration_number(f, 9), 0.001);
    let h = rt.calibration_text(f, 0).unwrap();
    assert_eq!(rt.raster_u8(h, 0), Ok(&[][..]));
}

#[test]
fn issue_269_a_refused_calibration_names_its_rule_and_the_file_still_reads() {
    let good = file(Some(&calibration()));
    let reader = pvec_container::reader::ContainerReader::open(
        pvec_container::source::MemorySource::new(good.clone()),
        pvec_container::reader::ReaderConfig::default(),
    )
    .unwrap();
    let mut blocks: Vec<RawBlock> = Vec::new();
    for (i, e) in reader.entries().iter().enumerate() {
        let mut stored = reader.read_stored(pvec_container::reader::BlockId(i as u32)).unwrap().to_vec();
        if e.kind == 11 {
            stored[0] = b'X';
        }
        blocks.push(RawBlock {
            kind: e.kind,
            subtype: e.subtype,
            cell: e.cell,
            count: e.count,
            bounds: e.bounds,
            flags: e.flags,
            codec: e.codec,
            capability: e.capability,
            uncompressed_length: e.uncompressed_length,
            stored,
        });
    }
    let damaged = assemble(reader.manifest().clone(), &blocks, Integrity::Full).unwrap();
    let (mut rt, f) = open(damaged);
    assert_eq!(rt.calibration_stat(f, 0), Ok(0));
    assert_eq!((rt.calibration_stat(f, 7), rt.calibration_stat(f, 8)), (Ok(1), Ok(0)), "CAL-01");
    assert_eq!(rt.calibration_stat(f, 9), Ok(1), "the file carries a calibration this reader cannot show");
    assert!(rt.calibration_number(f, 0).is_nan());
}

#[test]
fn issue_269_a_ranged_file_asks_for_the_calibration_block_in_one_range() {
    let bytes = file(Some(&calibration()));
    let mut rt = Runtime::new();
    let h = rt.open_ranged(bytes.len() as u64).unwrap();
    let f = loop {
        match rt.open_poll(h) {
            Ok(f) => break f,
            Err(AbiError::NeedRange) => {
                for (offset, len) in rt.needed(h).unwrap() {
                    let end = (offset + u64::from(len)) as usize;
                    rt.provide_range(h, offset, &bytes[offset as usize..end]).unwrap();
                }
            }
            Err(e) => panic!("{e:?}"),
        }
    };
    let first = rt.calibration_stat(f, 0);
    if first == Err(AbiError::NeedRange) {
        let needed = rt.needed(f).unwrap();
        assert_eq!(needed.len(), 1, "one range: {needed:?}");
        for (offset, len) in needed {
            let end = (offset + u64::from(len)) as usize;
            rt.provide_range(f, offset, &bytes[offset as usize..end]).unwrap();
        }
    }
    assert_eq!(rt.calibration_stat(f, 0), Ok(3));
    assert_eq!(rt.calibration_number(f, 0).to_bits(), 0.1f64.to_bits());
}
