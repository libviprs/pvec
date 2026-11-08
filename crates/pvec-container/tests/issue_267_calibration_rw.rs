//! Issue #267 (#263 slice 3, ADR 0019): the writer puts the calibration in one kind 11 block at the very end, the
//! reader hands it back, a reader without capability 22 skips it and keeps the geometry, LIMIT-17 is held from the
//! directory before a byte is fetched, and a refused calibration refuses the calibration and never the file.

mod support;

use pvec_capabilities::{CapabilityId, Requirement, Version};
use pvec_components::{
    Calibration, CrsKind, FitMethod, Georeference, Provenance, ScaleClaim, encode_calibration,
};
use pvec_container::integrity::Integrity;
use pvec_container::limits::Limits;
use pvec_container::negotiate::{Effect, ReaderCapabilities};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{ContainerWriter, WriteError};
use support::art::{self, Art, SEALED, rebuild};
use support::fixtures::mixed;

fn calibration() -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.1,
            provenance: Provenance::PlanScale {
                numerator: 1,
                denominator: 100,
                paper_metres_per_unit: 0.001,
            },
            uncertainty: None,
            note: "1:100".into(),
        }),
        georeference: Some(Georeference {
            source_to_crs: [0.1, 0.0, 0.0, 0.1, 530_000.0, 180_000.0],
            crs_kind: CrsKind::Projected,
            method: FitMethod::Declared,
            crs_id: "EPSG:27700".into(),
            definition: None,
            control_points: Vec::new(),
            residual_max: 0.0,
        }),
        external_id: "cal-1".into(),
    }
}

fn calibrated_writer() -> ContainerWriter {
    let mut w = mixed().load(&[]).expect("the fixture loads");
    w.calibration(&calibration()).expect("a valid calibration");
    w
}

fn open(bytes: Vec<u8>, caps: ReaderCapabilities, limits: Limits) -> ContainerReader<MemorySource<Vec<u8>>> {
    let cfg = ReaderConfig { capabilities: caps, limits, ..ReaderConfig::default() };
    ContainerReader::open(MemorySource::new(bytes), cfg).expect("the file opens")
}

#[test]
fn the_calibration_is_one_kind_11_block_last_with_codec_0_and_22_is_declared_optional() {
    let bytes = calibrated_writer().finish().unwrap();
    let a = Art::new(bytes);
    let last = a.entries.last().unwrap();
    assert_eq!((last.kind, last.capability, last.cell, last.subtype), (11, 22, 0, 0));
    assert_eq!((last.count, last.flags, last.codec), (2, 0, 0));
    let payload = encode_calibration(&calibration()).unwrap();
    assert_eq!(
        (last.stored_length as usize, last.uncompressed_length as usize),
        (payload.len(), payload.len())
    );
    assert_eq!(a.entries.iter().filter(|e| e.kind == 11).count(), 1);
    let d = a.manifest.capabilities.iter().find(|d| d.id == CapabilityId::new(22)).expect("22 is declared");
    assert_eq!((d.requirement, d.version), (Requirement::Optional, Version::new(1, 0, 0)));
    assert_eq!(a.manifest.min_reader, Version::new(1, 0, 0), "an optional 22 does not raise min_reader");
}

#[test]
fn a_reader_with_22_reads_it_back_and_the_file_validates() {
    let bytes = calibrated_writer().finish().unwrap();
    let r = open(bytes, ReaderCapabilities::v1(), Limits::DEFAULT);
    assert_eq!(r.calibration().unwrap(), Some(calibration()));
    r.validate().expect("a calibrated file is a canonical artifact");
    assert!(r.partial_open().is_none());
}

#[test]
fn a_file_without_one_has_none_and_does_not_declare_22() {
    let a = art::from_input_with(&mixed(), SEALED);
    assert!(a.manifest.capabilities.iter().all(|d| d.id != CapabilityId::new(22)));
    assert_eq!(open(a.bytes, ReaderCapabilities::v1(), Limits::DEFAULT).calibration().unwrap(), None);
}

#[test]
fn a_reader_without_22_skips_it_reports_it_and_reads_the_same_geometry() {
    let bytes = calibrated_writer().finish().unwrap();
    let plain = mixed().write().unwrap();
    let r = open(bytes, ReaderCapabilities::v1().without(CapabilityId::new(22)), Limits::DEFAULT);
    assert_eq!(r.calibration().unwrap(), None);
    let p = r.partial_open().expect("a degraded open");
    assert_eq!(
        p.capabilities.iter().map(|c| (c.id.get(), c.effect)).collect::<Vec<_>>(),
        [(22, Effect::Uncalibrated)]
    );
    let other = open(plain, ReaderCapabilities::v1(), Limits::DEFAULT);
    let geometry = |r: &ContainerReader<MemorySource<Vec<u8>>>| {
        r.entries()
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind < 9)
            .map(|(i, _)| r.read_block(BlockId(i as u32)).unwrap().as_bytes().to_vec())
            .collect::<Vec<_>>()
    };
    assert_eq!(geometry(&r), geometry(&other));
}

#[test]
fn limit_17_is_held_from_the_directory_before_the_block_is_fetched() {
    let bytes = calibrated_writer().finish().unwrap();
    let r = open(bytes, ReaderCapabilities::v1(), Limits { max_calibration_bytes: 64, ..Limits::DEFAULT });
    let id = BlockId(r.entries().len() as u32 - 1);
    let e = r.calibration().unwrap_err();
    assert_eq!(e.rule(), "LIMIT-17");
    assert!(!r.is_cached(id), "the block was fetched");
    r.read_block(BlockId(0)).expect("the file stays open and the geometry reads");
}

#[test]
fn the_writer_refuses_what_a_reader_would_as_wrt_18_with_the_cal_rule() {
    let mut c = calibration();
    c.georeference.as_mut().unwrap().source_to_crs = [1.0, 2.0, 2.0, 4.0, 0.0, 0.0];
    let mut w = mixed().load(&[]).unwrap();
    let e = w.calibration(&c).unwrap_err();
    assert_eq!(e.rule(), "WRT-18");
    assert_eq!(e, WriteError::Calibration { rule: "CAL-05" });
    assert!(e.to_string().contains("CAL-05"), "{e}");
    assert!(pvec_container::writer::WRITE_RULES.iter().any(|(r, m)| *r == "WRT-18" && m.contains("CAL")));
}

#[test]
fn a_damaged_or_misplaced_block_refuses_the_calibration_and_not_the_file() {
    let good = Art::new(calibrated_writer().finish().unwrap());
    let damaged = rebuild(&good, Integrity::Full, |_, blocks| {
        let last = blocks.last_mut().unwrap();
        last.stored[0] = b'X';
    });
    let r = open(damaged.bytes, ReaderCapabilities::v1(), Limits::DEFAULT);
    assert_eq!(r.calibration().unwrap_err().rule(), "CAL-01");
    r.validate().expect("the file is still valid: the payload is the capability's to judge");
    let misplaced = rebuild(&good, Integrity::Full, |_, blocks| blocks.last_mut().unwrap().subtype = 1);
    let r = open(misplaced.bytes, ReaderCapabilities::v1(), Limits::DEFAULT);
    assert_eq!(r.calibration().unwrap_err().rule(), "CAL-12");
    // LAYOUT-04 makes the keys unique, so a second kind 11 block always differs from the first in its cell or
    // subtype; the reader names it CAL-12 for being a second block before it looks at either
    let two = rebuild(&good, Integrity::Full, |_, blocks| {
        let mut second = blocks.last().unwrap().clone();
        second.subtype = 1;
        blocks.push(second);
    });
    let r = open(two.bytes, ReaderCapabilities::v1(), Limits::DEFAULT);
    assert_eq!(r.calibration().unwrap_err().rule(), "CAL-12");
}

#[test]
fn the_spill_writer_writes_the_same_bytes() {
    let dir = std::env::temp_dir().join(format!("pvec-267-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut w = mixed().load_spill(&dir, 1 << 20, art::SEALED).unwrap();
    w.calibration(&calibration()).unwrap();
    let mut out = support::VecSink::default();
    w.finish_to(&mut out).unwrap();
    let mut c = mixed().load_with(&[], art::SEALED).unwrap();
    c.calibration(&calibration()).unwrap();
    assert_eq!(out.0, c.finish().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}
