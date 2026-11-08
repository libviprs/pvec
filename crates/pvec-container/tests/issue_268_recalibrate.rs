//! Issue #268 (#263 slice 4, ADR 0019): re-calibration is a canonical rewrite through `assemble` with every other
//! block copied raw. It gives exactly the bytes a one-shot write gives, a replace moves no other block, an add or
//! a remove moves every block by the manifest's 16 byte capability entry and nothing else, and a reader holding the
//! old directory fails loudly instead of handing back a stale block.

mod support;

use pvec_capabilities::{CapabilityId, Declared, Requirement, Version};
use pvec_components::{Calibration, CrsKind, FitMethod, Georeference, Provenance, ScaleClaim};
use pvec_container::integrity::{DigestStatus, Integrity};
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::{BlockSource, Bytes, MemorySource, SourceError};
use pvec_container::writer::{RawBlock, recalibrate};
use std::sync::atomic::{AtomicBool, Ordering};
use support::art::{Art, rebuild};
use support::fixtures::{fixtures, mixed};

fn scale_only(metres: f64) -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: metres,
            provenance: Provenance::Manual,
            uncertainty: None,
            note: String::new(),
        }),
        georeference: None,
        external_id: String::new(),
    }
}

fn georeferenced() -> Calibration {
    Calibration {
        scale: Some(ScaleClaim {
            metres_per_source_unit: 0.1,
            provenance: Provenance::PlanScale {
                numerator: 1,
                denominator: 100,
                paper_metres_per_unit: 0.001,
            },
            uncertainty: Some(0.01),
            note: "1:100 on A1".into(),
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
        external_id: "sheet-7".into(),
    }
}

fn written(input: &support::fixtures::Input, c: Option<&Calibration>) -> Vec<u8> {
    let mut w = input.load(&[]).expect("the fixture loads");
    if let Some(c) = c {
        w.calibration(c).expect("a valid calibration");
    }
    w.finish().expect("the fixture writes")
}

#[test]
fn adding_a_calibration_gives_the_bytes_of_a_one_shot_write_on_every_fixture() {
    for fx in fixtures() {
        for c in [scale_only(0.25), georeferenced()] {
            let plain = written(&fx, None);
            assert_eq!(recalibrate(&plain, Some(&c)).unwrap(), written(&fx, Some(&c)), "{}", fx.name);
        }
    }
}

#[test]
fn replacing_gives_the_one_shot_bytes_and_moves_no_other_block() {
    let (a, b) = (georeferenced(), scale_only(0.5));
    let first = written(&mixed(), Some(&a));
    let replaced = recalibrate(&first, Some(&b)).unwrap();
    assert_eq!(replaced, written(&mixed(), Some(&b)));
    let (old, new) = (Art::new(first), Art::new(replaced));
    assert_eq!(old.entries.len(), new.entries.len());
    let n = old.entries.len();
    assert_eq!(old.entries[..n - 1], new.entries[..n - 1], "an entry other than the calibration's moved");
    assert_ne!(old.entries[n - 1].stored_length, new.entries[n - 1].stored_length);
}

#[test]
fn removing_gives_back_the_uncalibrated_file() {
    let plain = written(&mixed(), None);
    let calibrated = written(&mixed(), Some(&georeferenced()));
    assert_eq!(recalibrate(&calibrated, None).unwrap(), plain);
    assert_eq!(recalibrate(&plain, None).unwrap(), plain, "removing nothing changes nothing");
}

#[test]
fn an_add_moves_every_block_by_16_and_keeps_every_checksum_and_the_digest_is_recomputed() {
    let plain = Art::new(written(&mixed(), None));
    let added = recalibrate(&plain.bytes, Some(&scale_only(0.25))).unwrap();
    let a = Art::new(added.clone());
    assert_eq!(a.entries.len(), plain.entries.len() + 1);
    for (old, new) in plain.entries.iter().zip(&a.entries) {
        assert_eq!(new.offset, old.offset + 16, "the manifest's capability entry is 16 bytes");
        assert_eq!((new.kind, new.stored_length), (old.kind, old.stored_length));
        let (o, n) = (old.offset as usize, new.offset as usize);
        let len = old.stored_length as usize;
        assert_eq!(plain.bytes[o..o + len], added[n..n + len], "a block's bytes changed");
    }
    let r = ContainerReader::open(MemorySource::new(added), ReaderConfig::default()).unwrap();
    assert_eq!(r.verify_artifact_digest().unwrap(), DigestStatus::Verified);
    r.validate().unwrap();
    assert_eq!(r.calibration().unwrap(), Some(scale_only(0.25)));
}

#[test]
fn a_block_the_rewriter_does_not_understand_is_carried_byte_for_byte() {
    let plain = Art::new(written(&mixed(), None));
    let opaque = RawBlock {
        kind: 99,
        subtype: 0,
        cell: 0,
        count: 1,
        bounds: None,
        flags: 0,
        codec: 0,
        capability: 99,
        uncompressed_length: 40,
        stored: (0..40u8).map(|i| i.wrapping_mul(37)).collect(),
    };
    let with_unknown = rebuild(&plain, Integrity::Full, |m, blocks| {
        m.capabilities.push(Declared {
            id: CapabilityId::new(99),
            requirement: Requirement::Optional,
            version: Version::new(1, 0, 0),
        });
        blocks.push(opaque.clone());
    });
    let out = Art::new(recalibrate(&with_unknown.bytes, Some(&scale_only(2.0))).unwrap());
    let kept = out.entries.iter().find(|e| e.kind == 99).expect("the unknown block is kept");
    let at = kept.offset as usize;
    assert_eq!(out.bytes[at..at + 40], opaque.stored[..]);
    assert!(out.manifest.capabilities.iter().any(|d| d.id.get() == 99));
    assert_eq!(out.entries.last().unwrap().kind, 11, "the calibration is still the last block");
}

/// A source that serves one file until it is switched, then another: a ranged reader whose directory came from
/// the first file and whose blocks come from the second.
struct Switched {
    old: Vec<u8>,
    new: Vec<u8>,
    switched: AtomicBool,
}

impl BlockSource for Switched {
    fn len(&self) -> u64 {
        (if self.switched.load(Ordering::SeqCst) { self.new.len() } else { self.old.len() }) as u64
    }

    fn read_range(&self, offset: u64, len: u32) -> Result<Bytes, SourceError> {
        let bytes = if self.switched.load(Ordering::SeqCst) { &self.new } else { &self.old };
        MemorySource::new(bytes.as_slice()).read_range(offset, len)
    }
}

fn stale(old: Vec<u8>, new: Vec<u8>) -> ContainerReader<Switched> {
    let source = Switched { old, new, switched: AtomicBool::new(false) };
    let r = ContainerReader::open(source, ReaderConfig::default()).expect("the old file opens");
    r.source().switched.store(true, Ordering::SeqCst);
    r
}

#[test]
fn a_reader_holding_the_directory_from_before_an_add_fails_loudly_on_every_block() {
    let plain = written(&mixed(), None);
    let added = recalibrate(&plain, Some(&georeferenced())).unwrap();
    let r = stale(plain, added);
    // a block the open already read is held from the old file and is right; every other one is fetched now
    let fetched: Vec<u32> = (0..r.entries().len() as u32).filter(|i| !r.is_cached(BlockId(*i))).collect();
    assert!(fetched.len() >= 3);
    for i in fetched {
        let e = r.read_block(BlockId(i)).expect_err("a stale block came back as if it were good");
        assert_eq!(e.rule(), "INTEGRITY-03", "block {i}: {e}");
    }
}

#[test]
fn after_a_replace_a_stale_reader_still_reads_the_unmoved_blocks_and_refuses_the_calibration() {
    let first = written(&mixed(), Some(&georeferenced()));
    let replaced = recalibrate(&first, Some(&scale_only(0.5))).unwrap();
    let fresh = ContainerReader::open(MemorySource::new(first.clone()), ReaderConfig::default()).unwrap();
    let r = stale(first, replaced);
    let n = r.entries().len();
    for i in 0..n - 1 {
        let id = BlockId(i as u32);
        assert_eq!(r.read_block(id).unwrap().as_bytes(), fresh.read_block(id).unwrap().as_bytes());
    }
    assert!(r.calibration().is_err(), "the stale calibration block came back as if it were good");
}
