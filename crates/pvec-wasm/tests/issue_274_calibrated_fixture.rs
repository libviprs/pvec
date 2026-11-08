//! Issue #274: the calibrated file the viewer's browser tests open, `tests/compat/viewer-calibrated.pvec`. The same
//! page as `viewer-page.pvec` without its rasters (a frame, a corner mark, a line, a circle, in points with y up),
//! carrying a calibration block (ADR 0019): a 1:100 plan scale on a sheet in points, with a 1% uncertainty, and a
//! declared georeference into `EPSG:27700` whose affine is that scale turned by the 3-4-5 angle (`[0.8 s, -0.6 s, 0.6 s,
//! 0.8 s]`, so a swapped entry shows) from (530000, 180000). `viewer-calibration-refused.pvec` is the same file with
//! the calibration block's magic damaged, so a reader refuses the calibration as CAL-01 and still draws the page.
//! The specs in `viewer/tests/issue-274-calibration.spec.mjs` know these numbers. `PVEC_BLESS_FIXTURE=1` writes both
//! files again.

use pvec_components::{Calibration, CrsKind, FitMethod, Georeference, Provenance, ScaleClaim};
use pvec_container::integrity::Integrity;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, EntityMeta, RawBlock, WriterConfig, assemble};
use pvec_container::{Bounds, LengthUnit, Metadata};
use pvec_entity::EntityId;
use pvec_wasm::Runtime;
use std::f64::consts::TAU;
use std::fs;
use std::path::Path;

#[test]
fn issue_274_the_calibrated_viewer_fixture_is_what_this_test_writes_and_the_module_reads_its_calibration() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/viewer-calibrated.pvec");
    let paper = 0.0254 / 72.0;
    let scale = paper * 100.0;
    let want = {
        let mut w = ContainerWriter::new(WriterConfig {
            unit: LengthUnit::Metre,
            metadata: Metadata {
                producer_id: "pvec-viewer-fixture".into(),
                producer_version: "0".into(),
                corpus_revision: None,
            },
            bounds: None,
            capabilities: Vec::new(),
        });
        let meta = |id: u32, min: [f64; 2], max: [f64; 2]| EntityMeta {
            id: EntityId::new(id).expect("an id"),
            style: 0,
            cell: 0,
            bounds: Bounds { min, max },
        };
        w.push_polyline(
            meta(0, [0.0, 0.0], [612.0, 792.0]),
            &[[0.0, 0.0], [612.0, 0.0], [612.0, 792.0], [0.0, 792.0], [0.0, 0.0]],
        )
        .expect("the frame");
        w.push_polyline(
            meta(1, [20.0, 712.0], [80.0, 772.0]),
            &[[20.0, 772.0], [80.0, 772.0], [80.0, 712.0]],
        )
        .expect("the mark");
        w.push_line(meta(2, [36.0, 550.0], [576.0, 550.0]), [36.0, 550.0], [576.0, 550.0]).expect("the line");
        w.push_circle_arc(meta(3, [226.0, 120.0], [386.0, 280.0]), [306.0, 200.0], 80.0, [0.0, TAU])
            .expect("the circle");
        w.calibration(&Calibration {
            scale: Some(ScaleClaim {
                metres_per_source_unit: scale,
                provenance: Provenance::PlanScale {
                    numerator: 1,
                    denominator: 100,
                    paper_metres_per_unit: paper,
                },
                uncertainty: Some(0.01),
                note: "1:100 on the title block".into(),
            }),
            georeference: Some(Georeference {
                source_to_crs: [scale * 0.8, -scale * 0.6, scale * 0.6, scale * 0.8, 530_000.0, 180_000.0],
                crs_kind: CrsKind::Projected,
                method: FitMethod::Declared,
                crs_id: "EPSG:27700".into(),
                definition: None,
                control_points: Vec::new(),
                residual_max: 0.0,
            }),
            external_id: "sheet-A101".into(),
        })
        .expect("the calibration");
        w.finish().expect("the file")
    };
    // the same file with the kind 11 block's magic damaged, every other block copied as it is
    let refused = {
        let reader = ContainerReader::open(MemorySource::new(want.clone()), ReaderConfig::default()).unwrap();
        let mut blocks = Vec::new();
        for (i, e) in reader.entries().iter().enumerate() {
            let mut stored = reader.read_stored(BlockId(i as u32)).unwrap().to_vec();
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
        assemble(reader.manifest().clone(), &blocks, Integrity::Full).unwrap()
    };
    let refused_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/compat/viewer-calibration-refused.pvec");
    if std::env::var_os("PVEC_BLESS_FIXTURE").is_some() {
        fs::write(&path, &want).unwrap();
        fs::write(&refused_path, &refused).unwrap();
    }
    assert!(fs::read(&refused_path).unwrap() == refused, "viewer-calibration-refused.pvec: bless it again");
    let mut rt = Runtime::new();
    let f = rt.open(refused).expect("the page still opens");
    assert_eq!(
        (rt.calibration_stat(f, 0), rt.calibration_stat(f, 7), rt.calibration_stat(f, 8)),
        (Ok(0), Ok(1), Ok(0)),
        "CAL-01"
    );
    let have =
        fs::read(&path).expect("tests/compat/viewer-calibrated.pvec: write it with PVEC_BLESS_FIXTURE=1");
    assert!(have == want, "tests/compat/viewer-calibrated.pvec is not what this test writes: bless it again");
    let mut rt = Runtime::new();
    let f = rt.open(have).expect("the module opens it");
    assert_eq!(rt.calibration_stat(f, 0), Ok(3), "a scale and a georeference");
    assert_eq!(rt.calibration_stat(f, 1), Ok(2), "a plan scale");
    assert_eq!(rt.calibration_number(f, 0).to_bits(), scale.to_bits());
}
