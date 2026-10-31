//! Issue #217: the module opens with `Limits::BROWSER`, not `Limits::DEFAULT`. A file the defaults admit and the
//! browser preset refuses is refused by `Runtime::open` and by the ranged open, and a plain file still opens.
//! `docs/WASM_ABI.md` and `docs/MIGRATION.md` carry the before and after values.

use pvec::{Bounds2, Curve, EntityId, Manifest, PvecReader, PvecWriter};
use pvec_container::limits::Limits;
use pvec_container::reader::{BlockId, ContainerReader, ReaderConfig};
use pvec_container::source::MemorySource;
use pvec_container::writer::{RawBlock, assemble};
use pvec_geometry::{Point2, Polyline};
use pvec_wasm::{AbiError, Runtime};
use std::fs;
use std::path::Path;

fn plain() -> Vec<u8> {
    let manifest = Manifest {
        metres_per_unit: 1.0,
        bounds: Bounds2 { min: [0.0, 0.0], max: [100.0, 100.0] },
        capabilities: Vec::new(),
        producer_id: "issue-217".into(),
        producer_version: "1".into(),
    };
    let mut w = PvecWriter::new(manifest).unwrap();
    for i in 0..8u32 {
        let y = f64::from(i);
        let line = Polyline::new(vec![Point2 { x: 0.0, y }, Point2 { x: 9.0, y: y + 1.0 }]).unwrap();
        w.push(EntityId::new(i).unwrap(), &Curve::Polyline(line)).unwrap();
    }
    w.finish().unwrap()
}

/// `base` plus an optional block of a kind nobody owns whose directory entry declares a decoded length 40 times
/// its stored length: inside the default expansion ratio (64), outside the browser's (32). The block is skipped,
/// so no byte of it is ever decoded; the directory is what decides.
fn expanding(base: &[u8]) -> Vec<u8> {
    let r = ContainerReader::open(MemorySource::new(base.to_vec()), ReaderConfig::default()).unwrap();
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
    blocks.push(RawBlock {
        kind: 99,
        subtype: 0,
        cell: 0,
        count: 1,
        bounds: None,
        flags: 0,
        codec: 2,
        capability: 0,
        uncompressed_length: 40_000,
        stored: vec![0x11; 1_000],
    });
    assemble(r.manifest().clone(), &blocks, pvec_container::integrity::Integrity::Full).unwrap()
}

#[test]
fn issue_217_the_file_is_admitted_by_the_defaults_and_refused_by_the_browser_preset() {
    let bytes = expanding(&plain());
    let admitted = ContainerReader::open(
        MemorySource::new(bytes.clone()),
        ReaderConfig { limits: Limits::DEFAULT, ..ReaderConfig::default() },
    );
    assert!(admitted.is_ok(), "the defaults admit it: {:?}", admitted.err());
    let refused = ContainerReader::open(
        MemorySource::new(bytes.clone()),
        ReaderConfig { limits: Limits::BROWSER, ..ReaderConfig::default() },
    );
    assert_eq!(refused.err().map(|e| e.rule()), Some("LIMIT-01".to_string()));
    // and the facade call the module uses is the plain open, which is the default one
    assert!(PvecReader::open(bytes).is_ok());
}

#[test]
fn issue_217_open_refuses_a_file_over_the_browser_limits() {
    let mut rt = Runtime::new();
    assert_eq!(rt.open(expanding(&plain())), Err(AbiError::Read));
    let h = rt.open(plain()).expect("a plain file still opens under the browser limits");
    assert!(rt.directory(h).is_ok());
}

#[test]
fn issue_217_the_ranged_open_is_held_to_the_browser_limits_too() {
    let bytes = expanding(&plain());
    let mut rt = Runtime::new();
    let h = rt.open_ranged(bytes.len() as u64).unwrap();
    let mut outcome = rt.open_poll(h);
    while outcome == Err(AbiError::NeedRange) {
        for (offset, len) in rt.needed(h).unwrap() {
            let end = (offset + u64::from(len)) as usize;
            rt.provide_range(h, offset, &bytes[offset as usize..end]).unwrap();
        }
        outcome = rt.open_poll(h);
    }
    assert_eq!(outcome, Err(AbiError::Read));
}

#[test]
fn issue_217_the_documents_state_the_change_with_the_before_and_after_values() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    let abi = fs::read_to_string(root.join("WASM_ABI.md")).unwrap();
    let migration = fs::read_to_string(root.join("MIGRATION.md")).unwrap();
    for (name, text) in [("WASM_ABI.md", &abi), ("MIGRATION.md", &migration)] {
        for needle in
            ["Limits::BROWSER", "Limits::DEFAULT", "max_expansion_ratio", "64", "32", "max_block_bytes"]
        {
            assert!(text.contains(needle), "docs/{name} does not mention {needle}");
        }
    }
    assert!(
        abi.contains("bytes with `Limits::BROWSER` wired in"),
        "the module size is recorded after the change"
    );
}
